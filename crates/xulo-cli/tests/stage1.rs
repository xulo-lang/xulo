//! WP5 smoke tests: the stage 1 lexer/parser must run under `xulo run`, print
//! tokens and spans, parse a fixed example, parse its own source, and fail
//! with a clean `file:line:column` diagnostic.
//!
//! These drive the real CLI binary rather than the Rust parser, because Gate 1
//! and Gate 2 are statements about the interpreted path.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_xulo");

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/xulo-cli has a repository parent")
        .to_path_buf()
}

fn run_stage1(mode: &str, file: &Path) -> Output {
    let root = repo_root();
    Command::new(BIN)
        .arg("run")
        .arg(root.join("xuloc/main.xulo"))
        .env("XULO_STAGE1_MODE", mode)
        .env("XULO_STAGE1_FILE", file)
        .current_dir(&root)
        .output()
        .expect("xulo run should start")
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The first `E….` code in a rendered diagnostic, if any.
fn first_error_code(output: &Output) -> Option<String> {
    let text = stderr_of(output);
    let at = text.find("error[E")?;
    let rest = &text[at + "error[".len()..];
    let end = rest.find(']')?;
    Some(rest[..end].to_string())
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed ({}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        stdout_of(output),
        stderr_of(output)
    );
}

/// T5a: stage 1 prints tokens with spans for the fixed example.
#[test]
fn stage1_tokens_smoke_test() {
    let root = repo_root();
    let out = run_stage1("tokens", &root.join("examples/fibonacci.xulo"));
    assert_success(&out, "stage1 tokens");
    let text = stdout_of(&out);
    assert!(
        text.contains("2:1 keyword 12:14 fn"),
        "first token line should carry the kind, span, and text:\n{text}"
    );
    assert!(
        text.contains("ident") && text.contains("fib"),
        "identifier tokens should be visible:\n{text}"
    );
    assert!(
        text.lines().last().unwrap_or("").contains("eof"),
        "the last token is EOF:\n{text}"
    );
    assert!(
        stderr_of(&out).is_empty(),
        "a clean lex prints nothing on stderr"
    );
}

/// T5a: stage 1 parses the fixed example and prints a tree with accurate
/// spans.
#[test]
fn stage1_parse_smoke_test() {
    let root = repo_root();
    let out = run_stage1("parse", &root.join("examples/fibonacci.xulo"));
    assert_success(&out, "stage1 parse");
    let text = stdout_of(&out);
    assert!(
        text.starts_with("Program @2:1 [12,"),
        "the tree starts at the first declaration after the comment:\n{text}"
    );
    assert!(text.contains("Fn fib @2:1"), "function node:\n{text}");
    assert!(
        text.contains("Param n @2:8"),
        "parameter span starts at `n`:\n{text}"
    );
    assert!(text.contains("If @3:5"), "if expression node:\n{text}");
    assert!(text.contains("Return @4:9"), "return statement:\n{text}");
    assert!(
        text.contains("Binary + @6:16"),
        "binary expression:\n{text}"
    );
    assert!(
        stderr_of(&out).is_empty(),
        "a clean parse prints nothing on stderr"
    );
}

/// T5b: every stage 1 source file parses with itself.
///
/// The file set is discovered from `xuloc/` instead of being hard-coded, so a
/// new stage 1 source file (for example the Gate 3 checker) is covered as soon
/// as it lands.
#[test]
fn stage1_parses_its_own_source() {
    let root = repo_root();
    let mut sources: Vec<PathBuf> = std::fs::read_dir(root.join("xuloc"))
        .expect("xuloc/ should exist")
        .map(|entry| entry.expect("xuloc/ entry should be readable").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "xulo"))
        .collect();
    sources.sort();
    assert!(
        sources.len() >= 6,
        "expected at least the six stage 1 sources, found {sources:?}"
    );
    for path in &sources {
        let name = path
            .file_name()
            .expect("a source path has a file name")
            .to_string_lossy();
        let out = run_stage1("parse", path);
        assert_success(&out, &format!("self-parse of xuloc/{name}"));
        let text = stdout_of(&out);
        assert!(
            text.starts_with("Program @"),
            "xuloc/{name} produced no Program root:\n{text}"
        );
        assert!(
            text.contains("Fn "),
            "xuloc/{name} tree has no functions:\n{text}"
        );
    }
}

/// Gate 1: a malformed file exits nonzero and cites file, line, and column on
/// stderr instead of panicking.
#[test]
fn stage1_reports_clean_diagnostics() {
    let dir = std::env::temp_dir().join(format!(
        "xulo_stage1_diag_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let syntax = dir.join("syntax.xulo");
    std::fs::write(&syntax, "fn main( {\n}\n").unwrap();
    let out = run_stage1("parse", &syntax);
    assert!(!out.status.success(), "a syntax error must exit nonzero");
    let err = stderr_of(&out);
    assert!(
        err.contains(&format!("{}:1:10: error[E0002] syntax:", syntax.display())),
        "diagnostic should name the file, line, and column:\n{err}"
    );
    assert!(
        err.contains("expected an identifier"),
        "diagnostic should explain the failure:\n{err}"
    );

    let lexical = dir.join("lexical.xulo");
    std::fs::write(&lexical, "fn main() {\n    let x = \"open\n}\n").unwrap();
    let out = run_stage1("parse", &lexical);
    assert!(!out.status.success(), "a lexical error must exit nonzero");
    let err = stderr_of(&out);
    assert!(
        err.contains(&format!(
            "{}:2:13: error[E0001] lexical: unterminated string literal",
            lexical.display()
        )),
        "lexical diagnostic should point at the opening quote:\n{err}"
    );
}

/// WP5/B6: `xulo test` executes stage 1's own `#[test]` functions.
#[test]
fn stage1_tests_run_under_xulo_test() {
    let root = repo_root();
    let out = Command::new(BIN)
        .arg("test")
        .arg("xuloc/tests.xulo")
        .current_dir(&root)
        .output()
        .expect("xulo test should start");
    assert_success(&out, "xulo test xuloc/tests.xulo");
    let text = stdout_of(&out);
    assert!(
        text.contains("10 passed, 0 failed"),
        "all stage 1 tests should pass:\n{text}\n{}",
        stderr_of(&out)
    );
}

/// The stage 1 entry point must pass stage 0's own check before it runs.
#[test]
fn stage1_entry_point_checks() {
    let root = repo_root();
    let out = Command::new(BIN)
        .arg("check")
        .arg(root.join("xuloc/main.xulo"))
        .current_dir(&root)
        .output()
        .expect("xulo check should start");
    assert_success(&out, "xulo check xuloc/main.xulo");
}

/// Gate 3: stage 1's checker agrees with stage 0 on accept/reject for the
/// fixed bootstrap corpus, and rejected files carry a file:line:column code.
#[test]
fn stage1_check_agrees_with_stage0_on_the_corpus() {
    let root = repo_root();
    let cases = [
        ("xuloc/corpus/accept/fibonacci.xulo", true),
        ("xuloc/corpus/accept/basics.xulo", true),
        ("xuloc/corpus/reject/undefined_name.xulo", false),
        ("xuloc/corpus/reject/type_mismatch.xulo", false),
        ("xuloc/corpus/reject/arity.xulo", false),
        ("xuloc/corpus/reject/condition.xulo", false),
        ("xuloc/corpus/reject/not_iterable.xulo", false),
        ("xuloc/corpus/reject/return_mismatch.xulo", false),
    ];
    for (relative, accept) in cases {
        let file = root.join(relative);
        let stage0 = Command::new(BIN)
            .arg("check")
            .arg(&file)
            .current_dir(&root)
            .output()
            .expect("xulo check should start");
        let stage1 = run_stage1("check", &file);
        assert_eq!(
            stage0.status.success(),
            accept,
            "stage 0 disagreed on {relative}:\n{}",
            stderr_of(&stage0)
        );
        assert_eq!(
            stage1.status.success(),
            accept,
            "stage 1 disagreed on {relative}:\n{}\n{}",
            stdout_of(&stage1),
            stderr_of(&stage1)
        );
        if accept {
            assert!(
                stdout_of(&stage1).contains("no errors"),
                "stage 1 check should report success for {relative}"
            );
        } else {
            let err = stderr_of(&stage1);
            assert!(
                err.contains(": error[") && err.contains("] semantic:"),
                "stage 1 should report a coded semantic diagnostic for {relative}:\n{err}"
            );
            assert!(
                err.starts_with(&file.display().to_string()),
                "stage 1 diagnostic should name the file for {relative}:\n{err}"
            );
            // D4: the corpus codes must agree, not just the accept/reject
            // verdict.
            let stage0_code = first_error_code(&stage0);
            let stage1_code = first_error_code(&stage1);
            assert_eq!(
                stage0_code, stage1_code,
                "stage 0 and stage 1 disagreed on the code for {relative}"
            );
        }
    }
}
