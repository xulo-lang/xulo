//! End-to-end `Fs` namespace through the full pipeline (lex → parse →
//! semantic → native interpreter): the built-in `Result` wrapping, matching
//! on `Result::Ok`/`Result::Err`, and the interpreter's argument guards.
//!
//! Every test owns a unique scratch directory under the system temp dir and
//! removes it at the end, so runs stay hermetic.

use std::fs;
use std::path::{Path, PathBuf};

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

/// A fresh scratch directory per test: pid + label + name keeps parallel
/// tests from colliding.
fn scratch(label: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("xulo-runtime-fs-{}-{}", std::process::id(), label));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// A path safe to embed in a test program's string literal (POSIX temp paths
/// carry no quotes or backslashes).
fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn write_read_roundtrips_through_result_ok() {
    let dir = scratch("roundtrip");
    let file = dir.join("note.txt");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            let path = "{}"
            match std::fs::write(path, "hello xulo") {{
                Result::Ok(_) => print("wrote")
                Result::Err(e) => print(e.message)
            }}
            match std::fs::read(path) {{
                Result::Ok(content) => print(content)
                Result::Err(e) => print(e.message)
            }}
        }}
        "#,
        p(&file)
    ));
    assert_eq!(out, vec!["wrote".to_string(), "hello xulo".to_string()]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn read_missing_file_surfaces_the_error_message() {
    let dir = scratch("read-missing");
    let file = dir.join("absent.txt");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            match std::fs::read("{}") {{
                Result::Ok(content) => print(content)
                Result::Err(e) => print(e.message)
            }}
        }}
        "#,
        p(&file)
    ));
    assert_eq!(out.len(), 1, "one line printed: {out:?}");
    let line = &out[0];
    assert!(
        line.contains("cannot read") && line.contains("absent.txt"),
        "unexpected message: {line}"
    );
    assert!(
        line.contains("No such file or directory"),
        "the OS error rides along: {line}"
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn exists_is_a_total_predicate() {
    let dir = scratch("exists");
    let file = dir.join("here.txt");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            let path = "{}"
            print(str(std::fs::exists(path)))
            std::fs::write(path, "x")
            print(str(std::fs::exists(path)))
            print(str(std::fs::exists("{}")))
        }}
        "#,
        p(&file),
        p(&dir.join("gone.txt"))
    ));
    assert_eq!(
        out,
        vec!["false".to_string(), "true".to_string(), "false".to_string()]
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn mkdir_creates_parents_and_remove_deletes() {
    let dir = scratch("mkdir-remove");
    let deep = dir.join("a").join("b");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            match std::fs::mkdir("{}") {{
                Result::Ok(_) => print("made")
                Result::Err(e) => print(e.message)
            }}
            print(str(std::fs::exists("{}")))
            let file = "{}/leaf.txt"
            std::fs::write(file, "x")
            match std::fs::remove(file) {{
                Result::Ok(_) => print("removed")
                Result::Err(e) => print(e.message)
            }}
            print(str(std::fs::exists(file)))
        }}
        "#,
        p(&deep),
        p(&deep),
        p(&deep)
    ));
    assert_eq!(
        out,
        vec![
            "made".to_string(),
            "true".to_string(),
            "removed".to_string(),
            "false".to_string(),
        ]
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn exists_result_can_be_bound_and_matched_like_any_result() {
    let dir = scratch("bind");
    let file = dir.join("data.txt");
    let out = run_ok(&format!(
        r#"
        fn main() {{
            let path = "{}"
            std::fs::write(path, "42")
            let r: Result<string, Error> = std::fs::read(path)
            match r {{
                Result::Ok(v) => print(v)
                Result::Err(e) => print(e.message)
            }}
        }}
        "#,
        p(&file)
    ));
    assert_eq!(out, vec!["42".to_string()]);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn wrong_argument_type_is_an_interpreter_guard() {
    let err = run_raw(
        r#"
        fn main() {
            print(str(std::fs::exists(42)))
        }
        "#,
    )
    .unwrap_err();
    assert!(
        err.message.contains("`std::fs::exists` expects a string"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn unknown_fs_member_is_rejected_by_the_semantic_pass() {
    let err = run(r#"fn main() { std::fs::typo("x") }"#).unwrap_err();
    assert!(
        err.message.contains("`std::fs` has no member `typo`"),
        "unexpected message: {}",
        err.message
    );
}
