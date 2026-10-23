//! End-to-end `Path` namespace through the full pipeline (lex → parse →
//! semantic → native interpreter): pure string results, the qualified and
//! imported call forms, and the interpreter's argument guards.

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
fn qualified_members_compose_to_a_wiring_report() {
    let out = run_ok(
        r#"
        fn main() {
            let full = "/var/log/../log/app.log"
            let norm = std::path::normalize(full)
            print(norm)
            print(std::path::dirname(norm))
            print(std::path::basename(norm))
            print(std::path::ext(norm))
            print(std::path::join("/opt/app", "bin/run"))
            print(std::path::join("opt", "/abs"))
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "/var/log/app.log".to_string(),
            "/var/log".to_string(),
            "app.log".to_string(),
            ".log".to_string(),
            "/opt/app/bin/run".to_string(),
            "/abs".to_string(),
        ]
    );
}

#[test]
fn glob_import_binds_every_member_bare() {
    let out = run_ok(
        r#"
        use std::path::*;

        fn main() {
            print(join("a", "b"))
            print(dirname("/x/y"))
            print(basename("/x/y/"))
            print(ext("n.txt"))
            print(normalize("p/./q//r"))
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "a/b".to_string(),
            "/x".to_string(),
            "y".to_string(),
            ".txt".to_string(),
            "p/q/r".to_string(),
        ]
    );
}

#[test]
fn group_import_reaches_a_bare_call() {
    let out = run_ok(
        r#"
        use std::path::{dirname, ext};

        fn main() {
            print(dirname("a/b/c"))
            print(ext("a/b/c.tar"))
        }
        "#,
    );
    assert_eq!(out, vec!["a/b".to_string(), ".tar".to_string()]);
}

#[test]
fn wrong_argument_count_is_a_semantic_error() {
    let err = run(r#"
        fn main() {
            std::path::dirname()
        }
        "#)
    .expect_err("dirname needs one argument");
    assert!(
        err.message.contains("expected 1 argument"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn wrong_argument_type_is_a_semantic_error() {
    let err = run(r#"
        fn main() {
            std::path::join("a", 7)
        }
        "#)
    .expect_err("join takes two strings");
    assert!(
        err.message.contains("must be `string`"),
        "unexpected: {}",
        err.message
    );
}

#[test]
fn bare_path_head_is_redirected_under_std() {
    let err = run(r#"
        fn main() {
            Path::dirname("a")
        }
        "#)
    .expect_err("bare `Path` head is a legacy name");
    assert!(
        err.message.contains("lives under `std`"),
        "unexpected: {}",
        err.message
    );
}
