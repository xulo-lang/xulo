//! End-to-end `std::http` through the full pipeline (lex → parse → semantic
//! → native interpreter): the client and the server of
//! `std-spec/modules/http.md` over loopback sockets. `Server::serve` has no
//! shutdown member in 0.1, so a server test detaches its thread and lets the
//! process exit end it.

use std::net::TcpListener;
use std::thread;

use xulo_core::error::XuloError;
use xulo_parser::parse_program;

/// Full pipeline with the semantic pass, like the CLI's `xulo run`.
fn run(src: &str) -> Result<Vec<String>, XuloError> {
    let program = parse_program(src).unwrap();
    xulo_semantic::analyze(&program)?;
    let interp = xulo_runtime::interpreter::Interpreter::new();
    interp.run(&program)
}

fn run_ok(src: &str) -> Vec<String> {
    run(src).unwrap_or_else(|e| panic!("{}", e.message))
}

/// A free port the Xulo server can bind (the socket is released first).
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind")
        .local_addr()
        .expect("addr")
        .port()
}

#[test]
fn fetch_reads_status_headers_and_body() {
    let server = xulo_std::http::listen("127.0.0.1:0").expect("bind");
    let addr = server.addr().expect("addr");
    thread::spawn(move || {
        let _ = server.serve(|_req| xulo_std::http::Response::text("from rust", 200));
    });
    let out = run_ok(&format!(
        r#"
        fn main() {{
            match std::http::fetch("http://{addr}/health") {{
                Result::Ok(res) => {{
                    print(res.status)
                    print(res.header("content-type"))
                    print(res.body)
                }}
                Result::Err(e) => print(e.message)
            }}
        }}
        "#
    ));
    assert_eq!(
        out,
        vec![
            "200".to_string(),
            "text/plain; charset=utf-8".to_string(),
            "from rust".to_string()
        ],
        "{out:?}"
    );
}

#[test]
fn request_carries_the_form_and_reads_the_answer() {
    let server = xulo_std::http::listen("127.0.0.1:0").expect("bind");
    let addr = server.addr().expect("addr");
    thread::spawn(move || {
        let _ = server.serve(|req| {
            let greeting = req.form_value("name").unwrap_or("stranger".to_string());
            let who = req.query_value("who").unwrap_or("nobody".to_string());
            xulo_std::http::Response::text(&format!("{} {}", who, greeting), 200)
        });
    });
    let out = run_ok(&format!(
        r#"
        fn main() {{
            match std::http::request(
                "POST",
                "http://{addr}/greet?who=server",
                [{{ name: "content-type", value: "application/x-www-form-urlencoded" }}],
                "name=xulo"
            ) {{
                Result::Ok(res) => print(res.body)
                Result::Err(e) => print(e.message)
            }}
        }}
        "#
    ));
    assert_eq!(out, vec!["server xulo".to_string()], "{out:?}");
}

#[test]
fn a_xulo_server_answers_a_xulo_client() {
    // Port allocation races with the other tests' ephemeral binds: the
    // server answers a `/token` marker of its own port, and the harness
    // respawns on a fresh port until the marker proves the right instance
    // is listening.
    let mut port = 0u16;
    for _ in 0..10 {
        port = free_port();
        let server_src = format!(
            r#"
            fn route(req: std::http::Request): std::http::Response {{
                if req.method == "GET" and req.path == "/token" {{
                    std::http::Response.text("xulo-{port}")
                }} else if req.method == "GET" and req.path == "/health" {{
                    std::http::Response.text("ok")
                }} else if req.method == "POST" and req.path == "/echo" {{
                    std::http::Response.text(req.body, 201)
                }} else if req.method == "GET" and req.path == "/greet" {{
                    std::http::Response.text("hello, " + (req.queryValue("name") ?? "stranger"))
                }} else if req.method == "GET" and req.path == "/json" {{
                    match std::http::Response.json({{ ok: true }}) {{
                        Result::Ok(res) => res
                        Result::Err(e) => std::http::Response.text("bad", 500)
                    }}
                }} else {{
                    std::http::Response.text("not found", 404)
                }}
            }}
            fn main() {{
                match std::http::listen("127.0.0.1:{port}") {{
                    Result::Ok(server) => match server.serve(route) {{
                        Result::Ok(u) => print("done")
                        Result::Err(e) => print(e.message)
                    }}
                    Result::Err(e) => print(e.message)
                }}
            }}
            "#
        );
        thread::spawn(move || {
            let _ = run(&server_src);
        });
        let marker = format!("xulo-{port}");
        let mut bound = false;
        for _ in 0..40 {
            if let Ok(res) = xulo_std::http::fetch(&format!("http://127.0.0.1:{port}/token"), Some(500))
                && res.body == marker
            {
                bound = true;
                break;
            }
            thread::sleep(std::time::Duration::from_millis(25));
        }
        if bound {
            break;
        }
        port = 0;
    }
    assert_ne!(port, 0, "the xulo server never bound a free port");
    let client_src = format!(
        r#"
        fn main() {{
            match std::http::fetch("http://127.0.0.1:{port}/health") {{
                Result::Ok(res) => {{
                    print(res.status)
                    print(res.body)
                }}
                Result::Err(e) => print(e.message)
            }}
            match std::http::fetch("http://127.0.0.1:{port}/greet?name=xulo") {{
                Result::Ok(res) => print(res.body)
                Result::Err(e) => print(e.message)
            }}
            match std::http::request("POST", "http://127.0.0.1:{port}/echo", null, "echoed") {{
                Result::Ok(res) => {{
                    print(res.status)
                    print(res.body)
                }}
                Result::Err(e) => print(e.message)
            }}
            match std::http::fetch("http://127.0.0.1:{port}/json") {{
                Result::Ok(res) => {{
                    print(res.header("content-type"))
                    print(res.body)
                }}
                Result::Err(e) => print(e.message)
            }}
            match std::http::fetch("http://127.0.0.1:{port}/missing") {{
                Result::Ok(res) => print(res.status)
                Result::Err(e) => print(e.message)
            }}
        }}
        "#
    );
    let out = run_ok(&client_src);
    assert_eq!(
        out,
        vec![
            "200".to_string(),
            "ok".to_string(),
            "hello, xulo".to_string(),
            "201".to_string(),
            "echoed".to_string(),
            "application/json".to_string(),
            "{\"ok\":true}".to_string(),
            "404".to_string(),
        ],
        "{out:?}"
    );
}

#[test]
fn data_types_construct_and_answer_their_lookups() {
    let out = run_ok(
        r#"
        use std::http::{Request, Response};
        fn main() {
            let r: Request = Request(
                method: "GET",
                path: "/x",
                query: [{ name: "q", value: "1" }],
                form: [],
                headers: [{ name: "X-A", value: "b" }],
                body: "",
                version: "HTTP/1.1",
                remoteAddr: "1.2.3.4:5"
            )
            print(r.queryValue("q"))
            print(r.header("x-a"))
            print(r.header("nope"))
            print(r.formValue("nope"))
            let res = std::http::Response(status: 201, headers: [], body: "made")
            print(res.status)
            print(res.body)
            print(res.header("content-type"))
            let t = Response.text("hi", 203)
            print(t.status)
            print(t.header("content-type"))
            match Response.json({ a: 1 }) {
                Result::Ok(j) => print(j.header("content-type"))
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "1".to_string(),
            "b".to_string(),
            "null".to_string(),
            "null".to_string(),
            "201".to_string(),
            "made".to_string(),
            "null".to_string(),
            "203".to_string(),
            "text/plain; charset=utf-8".to_string(),
            "application/json".to_string(),
        ],
        "{out:?}"
    );
}

#[test]
fn statics_reach_the_types_through_paths_and_imports() {
    let out = run_ok(
        r#"
        use std::http as h;
        use std::http::*;
        fn main() {
            let r = h::Response.text("alias")
            print(r.body)
            let g = Response.text("glob")
            print(g.body)
            let q = std::http::Response.text("qualified")
            print(q.body)
        }
        "#,
    );
    assert_eq!(
        out,
        vec!["alias".to_string(), "glob".to_string(), "qualified".to_string()],
        "{out:?}"
    );
}

#[test]
fn fetch_budget_expiry_is_an_error_value() {
    // A listener that accepts and then says nothing at all.
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    thread::spawn(move || {
        let _conn = listener.accept();
        thread::sleep(std::time::Duration::from_secs(5));
    });
    let out = run_ok(&format!(
        r#"
        fn main() {{
            match std::http::fetch("http://127.0.0.1:{port}/", 300) {{
                Result::Ok(res) => print(res.body)
                Result::Err(e) => print(e.message)
            }}
        }}
        "#
    ));
    assert!(
        out[0].contains("timed out after 300 ms"),
        "{out:?}"
    );
}

#[test]
fn head_requests_carry_no_body() {
    let server = xulo_std::http::listen("127.0.0.1:0").expect("bind");
    let addr = server.addr().expect("addr");
    thread::spawn(move || {
        let _ = server.serve(|_req| xulo_std::http::Response::text("hidden", 200));
    });
    let out = run_ok(&format!(
        r#"
        fn main() {{
            match std::http::request("HEAD", "http://{addr}/x", null, null) {{
                Result::Ok(res) => {{
                    print(res.status)
                    print(res.header("content-length"))
                    print(res.body == "")
                }}
                Result::Err(e) => print(e.message)
            }}
        }}
        "#
    ));
    assert_eq!(
        out,
        vec!["200".to_string(), "6".to_string(), "true".to_string()],
        "{out:?}"
    );
}
