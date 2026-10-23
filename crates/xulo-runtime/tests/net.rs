//! End-to-end `Net` namespace through the full pipeline (lex → parse →
//! semantic → native interpreter): loopback TCP on 127.0.0.1 with an
//! OS-assigned port, the built-in `Result` wrapping, and the interpreter's
//! argument guards. v1 is synchronous, so every program handshakes in
//! program order (`listen` → `connect` → `accept`) — the connect completes
//! through the listen backlog before any `accept` blocks.

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

#[test]
fn echo_round_trip_through_qualified_calls() {
    let out = run_ok(
        r#"
        fn main() {
            match std::net::tcp_listen("127.0.0.1:0") {
                Result::Ok(server) => {
                    match std::net::sock_local_addr(server) {
                        Result::Ok(addr) => {
                            print(addr)
                            match std::net::tcp_connect(addr) {
                                Result::Ok(client) => {
                                    match std::net::tcp_accept(server) {
                                        Result::Ok(accepted) => {
                                            match std::net::sock_write(client, "ping") {
                                                Result::Ok(n) => print(n)
                                                Result::Err(e) => print(e.message)
                                            }
                                            match std::net::sock_read(accepted, 64) {
                                                Result::Ok(text) => print(text ?? "eof")
                                                Result::Err(e) => print(e.message)
                                            }
                                            match std::net::sock_write(accepted, "pong") {
                                                Result::Ok(n) => print(n)
                                                Result::Err(e) => print(e.message)
                                            }
                                            match std::net::sock_read(client, 64) {
                                                Result::Ok(text) => print(text ?? "eof")
                                                Result::Err(e) => print(e.message)
                                            }
                                            match std::net::sock_close(client) {
                                                Result::Ok(_) => print("client closed")
                                                Result::Err(e) => print(e.message)
                                            }
                                            match std::net::sock_close(accepted) {
                                                Result::Ok(_) => print("accepted closed")
                                                Result::Err(e) => print(e.message)
                                            }
                                            match std::net::sock_close(server) {
                                                Result::Ok(_) => print("server closed")
                                                Result::Err(e) => print(e.message)
                                            }
                                        }
                                        Result::Err(e) => print(e.message)
                                    }
                                }
                                Result::Err(e) => print(e.message)
                            }
                        }
                        Result::Err(e) => print(e.message)
                    }
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out.len(), 8, "one line per step: {out:?}");
    assert!(
        out[0].starts_with("127.0.0.1:") && !out[0].ends_with(":0"),
        "ephemeral port read back: {:?}",
        out[0]
    );
    assert_eq!(
        out[1..],
        [
            "4",
            "ping",
            "4",
            "pong",
            "client closed",
            "accepted closed",
            "server closed",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
    );
}

#[test]
fn glob_import_binds_every_member_bare_and_eof_is_null() {
    let out = run_ok(
        r#"
        use std::net::*;

        fn main() {
            match tcp_listen("127.0.0.1:0") {
                Result::Ok(server) => {
                    match sock_local_addr(server) {
                        Result::Ok(addr) => {
                            match tcp_connect(addr) {
                                Result::Ok(client) => {
                                    match tcp_accept(server) {
                                        Result::Ok(accepted) => {
                                            match sock_shutdown(client) {
                                                Result::Ok(_) => print("half-closed")
                                                Result::Err(e) => print(e.message)
                                            }
                                            match sock_read(accepted, 64) {
                                                Result::Ok(text) => print(text ?? "eof")
                                                Result::Err(e) => print(e.message)
                                            }
                                        }
                                        Result::Err(e) => print(e.message)
                                    }
                                }
                                Result::Err(e) => print(e.message)
                            }
                        }
                        Result::Err(e) => print(e.message)
                    }
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["half-closed".to_string(), "eof".to_string(),]);
}

#[test]
fn kind_mismatch_and_dead_handles_surface_error_messages() {
    let out = run_ok(
        r#"
        fn main() {
            match std::net::tcp_listen("127.0.0.1:0") {
                Result::Ok(server) => {
                    // A listener is not a stream: writing to it is data.
                    match std::net::sock_write(server, "x") {
                        Result::Ok(_) => print("wrote")
                        Result::Err(e) => print(e.message)
                    }
                    // A number the registry never issued stays an Err.
                    match std::net::sock_read(999999, 8) {
                        Result::Ok(text) => print(text ?? "eof")
                        Result::Err(e) => print(e.message)
                    }
                    match std::net::sock_close(server) {
                        Result::Ok(_) => print("closed")
                        Result::Err(e) => print(e.message)
                    }
                    // The second close sees no handle at all.
                    match std::net::sock_close(server) {
                        Result::Ok(_) => print("closed again")
                        Result::Err(e) => print(e.message)
                    }
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out.len(), 4, "one line per step: {out:?}");
    assert!(out[0].contains("listener, not a stream"), "{:?}", out[0]);
    assert!(out[1].contains("not open"), "{:?}", out[1]);
    assert_eq!(out[2], "closed");
    assert!(out[3].contains("not open"), "{:?}", out[3]);
}

#[test]
fn unresolvable_address_is_an_err_not_a_crash() {
    let out = run_ok(
        r#"
        fn main() {
            match std::net::tcp_connect("no.such.host.invalid:80") {
                Result::Ok(h) => print(h)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out.len(), 1);
    assert!(out[0].contains("cannot resolve"), "{:?}", out[0]);
}

#[test]
fn read_size_is_validated_and_reported() {
    let out = run_ok(
        r#"
        fn main() {
            match std::net::tcp_listen("127.0.0.1:0") {
                Result::Ok(server) => {
                    match std::net::sock_read(server, 0) {
                        Result::Ok(_) => print("read")
                        Result::Err(e) => print(e.message)
                    }
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out.len(), 1);
    assert!(out[0].contains("read size"), "{:?}", out[0]);
}
