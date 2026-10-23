//! End-to-end `Os` namespace through the full pipeline (lex → parse →
//! semantic → native interpreter): the built-in `Result` wrapping, matching
//! on `Result::Ok`/`Result::Err`, the `string?` payload of `getenv`, and the
//! interpreter's argument guards. Tests that mutate process state (the
//! environment, the working directory) restore it before finishing.

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
fn pid_is_positive_and_prints() {
    let out = run_ok(
        r#"
        fn main() {
            let p = std::os::pid()
            print(str(p > 0))
        }
        "#,
    );
    assert_eq!(out, vec!["true".to_string()]);
}

#[test]
fn platform_and_arch_are_nonempty_strings() {
    let out = run_ok(
        r#"
        fn main() {
            let os = std::os::platform()
            let cpu = std::os::arch()
            print(str(os != ""))
            print(str(cpu != ""))
        }
        "#,
    );
    assert_eq!(out, vec!["true".to_string(), "true".to_string()]);
}

#[test]
fn getenv_setenv_roundtrip_through_result() {
    let key = format!("XULO_RT_OS_TEST_{}", std::process::id());
    // SAFETY: single test process, unique plain key, restored below.
    unsafe { std::env::remove_var(&key) };
    let out = run_ok(&format!(
        r#"
        fn main() {{
            match std::os::getenv("{key}") {{
                Result::Ok(v) => print(v ?? "unset")
                Result::Err(e) => print(e.message)
            }}
            match std::os::setenv("{key}", "from-xulo") {{
                Result::Ok(_) => print("set")
                Result::Err(e) => print(e.message)
            }}
            match std::os::getenv("{key}") {{
                Result::Ok(v) => print(v ?? "unset")
                Result::Err(e) => print(e.message)
            }}
        }}
        "#
    ));
    assert_eq!(
        out,
        vec![
            "unset".to_string(),
            "set".to_string(),
            "from-xulo".to_string()
        ]
    );
    // SAFETY: cleanup of this test's own variable, same thread.
    unsafe { std::env::remove_var(&key) };
}

#[test]
fn setenv_rejects_an_illegal_name_through_result_err() {
    let out = run_ok(
        r#"
        fn main() {
            match std::os::setenv("A=B", "v") {
                Result::Ok(_) => print("set")
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out.len(), 1);
    assert!(
        out[0].starts_with("cannot set environment variable") && out[0].contains("contains '='"),
        "unexpected: {}",
        out[0]
    );
}

#[test]
fn cwd_returns_a_directory_and_chdir_to_missing_fails() {
    let missing =
        std::env::temp_dir().join(format!("xulo-rt-os-absent-{}-nope", std::process::id()));
    let out = run_ok(&format!(
        r#"
        fn main() {{
            match std::os::cwd() {{
                Result::Ok(path) => print(str(path != ""))
                Result::Err(e) => print(e.message)
            }}
            match std::os::chdir("{}") {{
                Result::Ok(_) => print("moved")
                Result::Err(e) => print(e.message)
            }}
        }}
        "#,
        missing.to_string_lossy()
    ));
    assert_eq!(out.len(), 2, "two lines: {out:?}");
    assert_eq!(out[0], "true", "cwd should be a non-empty path");
    assert!(
        out[1].starts_with("cannot change directory to '") && out[1].contains("No such file"),
        "unexpected: {}",
        out[1]
    );
}

#[test]
fn glob_import_binds_every_member() {
    let out = run_ok(
        r#"
        use std::os::{pid, platform, arch};
        fn main() {
            print(str(pid() > 0))
            print(str(platform() != ""))
            print(str(arch() != ""))
        }
        "#,
    );
    assert_eq!(
        out,
        vec!["true".to_string(), "true".to_string(), "true".to_string()]
    );
}

#[test]
fn wrong_argument_count_is_a_semantic_error() {
    let err = run(r#"
        fn main() {
            std::os::getenv()
        }
        "#)
    .expect_err("getenv needs one argument");
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
            std::os::chdir(42)
        }
        "#)
    .expect_err("chdir takes a string");
    assert!(
        err.message.contains("must be `string`"),
        "unexpected: {}",
        err.message
    );
}
