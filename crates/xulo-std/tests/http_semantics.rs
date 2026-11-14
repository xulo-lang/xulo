//! The `http` namespace semantics in isolation — the client and server of
//! `std-spec/modules/http.md` against loopback sockets, no Xulo pipeline
//! involved. `serve` has no shutdown member in 0.1, so each test detaches
//! its server thread and lets the process exit end it.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

use xulo_std::http::{self, Header, Request, Response};

/// Start a server on an ephemeral port with `handler`, detached in its own
/// thread; returns the bound `"ip:port"`.
fn serve<F>(handler: F) -> String
where
    F: Fn(&Request) -> Response + Send + 'static,
{
    let server = http::listen("127.0.0.1:0").expect("bind");
    let addr = server.addr().expect("addr");
    thread::spawn(move || {
        let _ = server.serve(handler);
    });
    addr
}

#[test]
fn fetch_gets_the_whole_answer() {
    let addr = serve(|_req| Response::text("ok", 200));
    let res = http::fetch(&format!("http://{addr}/health"), Some(5000)).expect("fetch");
    assert_eq!(res.status, 200);
    assert_eq!(res.body, "ok");
    assert_eq!(
        res.header("content-type").as_deref(),
        Some("text/plain; charset=utf-8")
    );
    assert!(res.header("date").is_some(), "the server writes a Date");
}

#[test]
fn request_carries_method_headers_and_body_both_ways() {
    let addr = serve(|req: &Request| {
        let greeting = req.form_value("name").unwrap_or("stranger".to_string());
        let who = req.query_value("who").unwrap_or("nobody".to_string());
        Response::text(
            &format!("{} {} {} {}", req.method, req.path, who, greeting),
            201,
        )
    });
    let res = http::request(
        "POST",
        &format!("http://{addr}/greet?who=server"),
        Some(vec![Header {
            name: "content-type".to_string(),
            value: "application/x-www-form-urlencoded".to_string(),
        }]),
        Some("name=xulo"),
        Some(5000),
    )
    .expect("request");
    assert_eq!(res.status, 201);
    assert_eq!(res.body, "POST /greet server xulo");
}

#[test]
fn query_and_path_decode_percent_escapes_but_keep_plus() {
    let addr = serve(|req: &Request| {
        Response::text(
            &format!("{}|{}", req.path, req.query_value("q").unwrap_or_default()),
            200,
        )
    });
    let res = http::fetch(&format!("http://{addr}/a%20b?q=x%2By"), Some(5000)).expect("fetch");
    assert_eq!(res.body, "/a b|x+y");
}

#[test]
fn hand_built_responses_write_exactly_their_headers() {
    let addr = serve(|_req| Response {
        status: 200,
        headers: vec![
            Header {
                name: "X-Opaque".to_string(),
                value: "yes".to_string(),
            },
            // The framing three are the server's own — a caller's is dropped.
            Header {
                name: "Content-Length".to_string(),
                value: "999".to_string(),
            },
        ],
        body: "made".to_string(),
    });
    let res = http::fetch(&format!("http://{addr}/"), Some(5000)).expect("fetch");
    assert_eq!(res.body, "made");
    assert_eq!(res.header("x-opaque").as_deref(), Some("yes"));
    assert_eq!(res.header("content-length").as_deref(), Some("4"));
    assert_eq!(
        res.header("content-type"),
        None,
        "a hand-built Response writes exactly its headers"
    );
    // The JSON shorthand sets application/json.
    let json = Response::json("{\"ok\":true}", 200);
    assert_eq!(
        json.header("content-type").as_deref(),
        Some("application/json")
    );
}

#[test]
fn urls_outside_the_contract_are_errors() {
    for (url, needle) in [
        ("ftp://example.com/", "unsupported scheme"),
        ("https://example.com/", "unsupported scheme"),
        ("example.com/", "must start with `http://`"),
        ("http://:80/", "empty host"),
        ("http://example.com:0/", "port 0"),
        ("http://exa mple.com/", "space, a control character"),
    ] {
        let err = http::fetch(url, Some(5000)).expect_err(url);
        assert!(err.contains(needle), "{url}: {err}");
    }
    // The happy parse: default port and path, fragment dropped.
    let url = http::parse_url("http://example.com/x?a=1#frag").expect("parse");
    assert_eq!(url.host, "example.com");
    assert_eq!(url.port, 80);
    assert_eq!(url.target, "/x?a=1");
}

#[test]
fn an_expired_budget_is_an_error_class_of_its_own() {
    // A listener that accepts and then says nothing at all.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let addr = listener.local_addr().expect("addr");
    thread::spawn(move || {
        // Keep the connection open and say nothing at all.
        let _conn = listener.accept();
        thread::sleep(Duration::from_secs(5));
    });
    let err = http::fetch(&format!("http://{addr}/"), Some(300)).expect_err("budget");
    assert!(err.contains("timed out after 300 ms"), "{err}");
}

#[test]
fn server_error_answers_are_status_alone_and_close() {
    let addr = serve(|_req| Response::text("never", 200));
    let cases: [(&str, i64); 5] = [
        // Malformed request line.
        ("nonsense\r\n\r\n", 400),
        // Another version than HTTP/1.1.
        ("GET / HTTP/1.0\r\nHost: x\r\n\r\n", 505),
        // A transfer coding the server does not implement.
        (
            "POST / HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\n\r\n",
            501,
        ),
        // Any other Expect.
        (
            "POST / HTTP/1.1\r\nHost: x\r\nExpect: pizza\r\nContent-Length: 1\r\n\r\nx",
            417,
        ),
        // A body past the 16 MiB cap.
        (
            "POST / HTTP/1.1\r\nHost: x\r\nContent-Length: 17000000\r\n\r\n",
            413,
        ),
    ];
    for (raw, want) in cases {
        let mut stream = TcpStream::connect(&addr).expect("connect");
        stream.write_all(raw.as_bytes()).expect("write");
        let mut answer = String::new();
        stream.read_to_string(&mut answer).expect("read");
        let code: i64 = answer
            .split_whitespace()
            .nth(1)
            .and_then(|c| c.parse().ok())
            .unwrap_or(-1);
        assert_eq!(code, want, "{raw:?}: {answer:?}");
    }
    // An over-long header block answers 431.
    let mut stream = TcpStream::connect(&addr).expect("connect");
    let mut big = String::from("GET / HTTP/1.1\r\nHost: x\r\n");
    for i in 0..200 {
        big.push_str(&format!("X-Pad-{i}: {}\r\n", "y".repeat(400)));
    }
    big.push_str("\r\n");
    stream.write_all(big.as_bytes()).expect("write");
    let mut answer = String::new();
    stream.read_to_string(&mut answer).expect("read");
    assert!(answer.contains("431"), "{answer:?}");
}

#[test]
fn head_and_no_body_statuses_carry_no_body() {
    let addr = serve(|req: &Request| {
        if req.path == "/empty" {
            Response {
                status: 204,
                headers: vec![],
                body: "ignored".to_string(),
            }
        } else {
            Response::text("hidden", 200)
        }
    });
    let head =
        http::request("HEAD", &format!("http://{addr}/x"), None, None, Some(5000)).expect("head");
    assert_eq!(head.status, 200);
    assert_eq!(head.body, "", "a HEAD answer carries no body");
    assert_eq!(
        head.header("content-length").as_deref(),
        Some("6"),
        "the headers and Content-Length still arrive"
    );
    let no_body = http::fetch(&format!("http://{addr}/empty"), Some(5000)).expect("fetch");
    assert_eq!(no_body.status, 204);
    assert_eq!(no_body.body, "");
    assert_eq!(
        no_body.header("content-length"),
        None,
        "204 writes no Content-Length"
    );
}
