//! End-to-end `Io` namespace through the full pipeline (lex → parse →
//! semantic → native interpreter): the built-in `Result` wrapping, matching
//! on `Result::Ok`/`Result::Err`, and the interpreter's argument guards.
//!
//! These members write the *real* process streams, which the interpreter's
//! captured `print` buffer never sees — so a test observes them through the
//! `Result` it returns, and writes an empty payload whenever it must not
//! disturb the harness. End-of-input behaviour (`readLine`/`read`) needs a
//! controlled stdin and lives in the CLI integration tests instead.

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

/// Lex + parse only, so an interpreter-level guard (a wrong operand the
/// semantic pass would reject) can be exercised on its own.
fn run_raw(src: &str) -> Result<Vec<String>, XuloError> {
    let program = parse_program(src).unwrap();
    let interp = xulo_runtime::interpreter::Interpreter::new();
    interp.run(&program)
}

#[test]
fn writes_and_flush_report_success_through_the_builtin_result() {
    // Empty payloads: the write and flush really happen on the process's
    // stdout/stderr, but nothing lands in the harness's output.
    let out = run_ok(
        r#"
        fn main() {
            match std::io::print("") {
                Result::Ok(_) => print("print ok")
                Result::Err(e) => print(e.message)
            }
            match std::io::println("") {
                Result::Ok(_) => print("println ok")
                Result::Err(e) => print(e.message)
            }
            match std::io::eprint("") {
                Result::Ok(_) => print("eprint ok")
                Result::Err(e) => print(e.message)
            }
            match std::io::eprintln("") {
                Result::Ok(_) => print("eprintln ok")
                Result::Err(e) => print(e.message)
            }
            match std::io::flush() {
                Result::Ok(_) => print("flush ok")
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "print ok".to_string(),
            "println ok".to_string(),
            "eprint ok".to_string(),
            "eprintln ok".to_string(),
            "flush ok".to_string(),
        ]
    );
}

#[test]
fn a_write_result_binds_and_matches_like_any_result() {
    let out = run_ok(
        r#"
        fn main() {
            let wrote: Result<unit, Error> = std::io::print("")
            match wrote {
                Result::Ok(_) => print("bound")
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["bound".to_string()]);
}

#[test]
fn an_alias_head_reaches_the_namespace() {
    let out = run_ok(
        r#"
        use std::io as io;

        fn main() {
            match io::flush() {
                Result::Ok(_) => print("alias flush")
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["alias flush".to_string()]);
}

#[test]
fn a_group_import_binds_a_bare_member() {
    let out = run_ok(
        r#"
        use std::io::{flush};

        fn main() {
            match flush() {
                Result::Ok(_) => print("bare flush")
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["bare flush".to_string()]);
}

#[test]
fn a_glob_import_keeps_the_prelude_print_binding() {
    // `print`/`println` belong to the prelude (`spec/builtins/
    // intrinsic-functions.md`): the glob deliberately does not contribute
    // them, so a bare call stays the buffered builtin.
    let out = run_ok(
        r#"
        use std::io::*;

        fn main() {
            print("buffered")
        }
        "#,
    );
    assert_eq!(out, vec!["buffered".to_string()]);
}

#[test]
fn wrong_argument_type_is_an_interpreter_guard() {
    let err = run_raw(
        r#"
        fn main() {
            print(str(std::io::print(42)))
        }
        "#,
    )
    .unwrap_err();
    assert!(
        err.message.contains("`std::io::print` expects a string"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn an_operand_for_a_no_argument_member_is_an_interpreter_guard() {
    let err = run_raw(
        r#"
        fn main() {
            print(str(std::io::flush(1)))
        }
        "#,
    )
    .unwrap_err();
    assert!(
        err.message.contains("`std::io::flush` takes no arguments"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn unknown_io_member_is_rejected_by_the_semantic_pass() {
    let err = run(r#"fn main() { print(str(std::io::typo())) }"#).unwrap_err();
    assert!(
        err.message.contains("`std::io` has no member `typo`"),
        "unexpected message: {}",
        err.message
    );
}
