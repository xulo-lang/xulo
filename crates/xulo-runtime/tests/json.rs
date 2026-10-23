//! End-to-end `Json` namespace through the full pipeline (lex → parse →
//! semantic → native interpreter): tree walking through member access,
//! `parse`/`stringify` round-trips, failure paths as data, and the
//! interpreter's arity/type guards.

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
fn parse_walks_the_tree_through_member_access() {
    let out = run_ok(
        r#"
        fn main() {
            match std::json::parse("{\"a\": 1, \"b\": [true, \"x\"], \"c\": 2.5}") {
                Result::Ok(v) => {
                    print(v.a)
                    print(v.b[0])
                    print(v.b[1])
                    print(v.c)
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "1".to_string(),
            "true".to_string(),
            "x".to_string(),
            "2.5".to_string()
        ]
    );
}

#[test]
fn parse_and_stringify_roundtrip_semantically_identical_text() {
    let out = run_ok(
        r#"
        fn main() {
            let src = "{\"a\": [1, \"two\", false, null], \"b\": 2.5}"
            match std::json::parse(src) {
                Result::Ok(v) => {
                    match std::json::stringify(v) {
                        Result::Ok(text) => {
                            print(text)
                            match std::json::parse(text) {
                                Result::Ok(again) => {
                                    match std::json::stringify(again) {
                                        Result::Ok(t2) => print(t2 == text)
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
    assert_eq!(
        out,
        vec![
            r#"{"a":[1,"two",false,null],"b":2.5}"#.to_string(),
            "true".to_string()
        ]
    );
}

#[test]
fn stringify_renders_xulo_values_as_compact_json() {
    let out = run_ok(
        r#"
        fn main() {
            match std::json::stringify([1, "two", false, null, 2.5]) {
                Result::Ok(s) => print(s)
                Result::Err(e) => print(e.message)
            }
            match std::json::stringify({ name: "x", n: 3 }) {
                Result::Ok(s) => print(s)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            r#"[1,"two",false,null,2.5]"#.to_string(),
            // 对象键按字典序输出（serde_json 的默认 Map）
            r#"{"n":3,"name":"x"}"#.to_string()
        ]
    );
}

#[test]
fn parse_syntax_errors_surface_as_result_err() {
    let out = run_ok(
        r#"
        fn main() {
            match std::json::parse("{oops") {
                Result::Ok(v) => print(std::json::stringify(v))
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out.len(), 1);
    assert!(
        out[0].starts_with("invalid JSON: "),
        "unexpected: {}",
        out[0]
    );
}

#[test]
fn stringify_rejects_values_json_cannot_express() {
    let out = run_ok(
        r#"
        fn main() {
            let nan = std::math::sqrt(-1.0)
            match std::json::stringify(nan) {
                Result::Ok(s) => print(s)
                Result::Err(e) => print(e.message)
            }
            let ignore = fn() { print("hi") }
            match std::json::stringify(ignore) {
                Result::Ok(s) => print(s)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "cannot serialize NaN as JSON".to_string(),
            "cannot serialize a function as JSON".to_string()
        ]
    );
}

#[test]
fn stringify_refuses_cyclic_lists_through_the_depth_ceiling() {
    let out = run_ok(
        r#"
        fn main() {
            let ring = [1]
            ring.push(ring)
            match std::json::stringify(ring) {
                Result::Ok(s) => print(s)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(
        out,
        vec!["JSON value nests deeper than 128 levels".to_string()]
    );
}

#[test]
fn glob_import_binds_parse_and_stringify_bare() {
    let out = run_ok(
        r#"
        use std::json::*;

        fn main() {
            match parse("[1, 2, 3]") {
                Result::Ok(v) => {
                    match stringify(v) {
                        Result::Ok(s) => print(s)
                        Result::Err(e) => print(e.message)
                    }
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["[1,2,3]".to_string()]);
}

#[test]
fn json_member_and_arity_errors_are_semantic() {
    // Unknown member.
    let unknown = run(r#"
        fn main() {
            std::json::dumps("x")
        }
        "#)
    .expect_err("no such member");
    assert!(
        unknown.message.contains("has no member `dumps`"),
        "{}",
        unknown.message
    );

    // stringify takes exactly one argument.
    let zero = run(r#"
        fn main() {
            std::json::stringify()
        }
        "#)
    .expect_err("stringify() has no value");
    assert!(
        zero.message.contains("expected 1 argument"),
        "{}",
        zero.message
    );

    // The bare `Json::` head points at `std`.
    let bare = run(r#"
        fn main() {
            Json::parse("1")
        }
        "#)
    .expect_err("Json lives under std");
    assert!(
        bare.message.contains("lives under `std`"),
        "{}",
        bare.message
    );
}
