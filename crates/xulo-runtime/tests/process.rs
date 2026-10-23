//! End-to-end `Process` namespace through the full pipeline (lex → parse →
//! semantic → native interpreter): `args` shape, `run` capture semantics,
//! the optional input argument, and the interpreter's arity/type guards.
//! `exit` is deliberately absent — it would take the test harness with it;
//! the CLI's `process_exit` test covers it in a child process.

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
fn args_is_a_non_empty_string_list() {
    let out = run_ok(
        r#"
        fn main() {
            let argv = std::process::args()
            print(argv[0] != "")
            let mut n = 0
            for a in argv {
                n = n + 1
            }
            print(n > 0)
        }
        "#,
    );
    assert_eq!(out, vec!["true".to_string(), "true".to_string()]);
}

#[test]
fn run_captures_streams_and_codes_as_an_object_payload() {
    let out = run_ok(
        r#"
        fn main() {
            match std::process::run("echo hi; echo oops 1>&2; exit 4") {
                Result::Ok(r) => {
                    print(r.stdout)
                    print(r.stderr)
                    print(r.code)
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(
        out,
        vec!["hi\n".to_string(), "oops\n".to_string(), "4".to_string(),]
    );
}

#[test]
fn run_pipes_the_optional_input_argument() {
    let out = run_ok(
        r#"
        fn main() {
            match std::process::run("cat", "pong") {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["pong".to_string()]);
}

#[test]
fn glob_import_binds_run_and_args_bare() {
    let out = run_ok(
        r#"
        use std::process::*;

        fn main() {
            print(args()[0] != "")
            match run("printf '%s' ok") {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["true".to_string(), "ok".to_string()]);
}

#[test]
fn run_arity_is_one_or_two_arguments() {
    // Zero arguments: rejected with the fixed-arity message.
    let zero = run(r#"
        fn main() {
            std::process::run()
        }
        "#)
    .expect_err("run() has no command");
    assert!(
        zero.message.contains("expected 1 or 2 argument"),
        "{}",
        zero.message
    );

    // Three: the same bound rejects it.
    let three = run(r#"
        fn main() {
            std::process::run("echo", "a", "b")
        }
        "#)
    .expect_err("run() takes at most an input");
    assert!(
        three.message.contains("expected 1 or 2 argument"),
        "{}",
        three.message
    );

    // One and two both pass the semantic gate (the one-arg form is covered
    // by the qualified-call test above; two here).
    let out = run_ok(
        r#"
        fn main() {
            match std::process::run("cat", "x") {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["x".to_string()]);
}

#[test]
fn run_argument_types_are_enforced() {
    let err = run(r#"
        fn main() {
            std::process::run(42)
        }
        "#)
    .expect_err("cmd must be a string");
    assert!(err.message.contains("must be `string`"), "{}", err.message);
}
