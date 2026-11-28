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
        .arg(root.join("xuloc/src/main.xulo"))
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
    let out = run_stage1("tokens", &root.join("examples/functions/fibonacci.xulo"));
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
    let out = run_stage1("parse", &root.join("examples/functions/fibonacci.xulo"));
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

/// The parser keeps enough `use`/`mod` structure for the C backend's module
/// namespaces: inline bodies, path segments, groups, globs, and aliases.
#[test]
fn stage1_parse_keeps_module_and_use_structure() {
    let root = repo_root();
    let out = run_stage1("parse", &root.join("examples/modules/inline/main.xulo"));
    assert_success(&out, "stage1 parse of an inline-module example");
    let text = stdout_of(&out);
    for needle in [
        "Mod inner @",
        "ModBody @",
        "Use self @",
        "UseSegment inner @",
        "UseAlias i @",
        "Use inner @",
        "UseGroup @",
        "UseEntry hi @",
        "Use tools @",
        "UseGlob @",
        "Use super @",
        "UseSegment twice @",
    ] {
        assert!(
            text.contains(needle),
            "the tree should keep `{needle}`:\n{text}"
        );
    }
}

/// T5b: every stage 1 source file parses with itself.
///
/// Every `.xulo` file under `dir`, recursively, sorted.
fn collect_xulo_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("source directory should exist")
        .map(|entry| entry.expect("source entry should be readable").path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_xulo_sources(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "xulo") {
            out.push(path);
        }
    }
}

/// T5b: every stage 1 source file parses with itself.
///
/// The file set is discovered from `xuloc/src/` instead of being hard-coded, so a
/// new stage 1 source file (for example a checker or parser module) is covered
/// as soon as it lands.
#[test]
fn stage1_parses_its_own_source() {
    let root = repo_root();
    let mut sources: Vec<PathBuf> = Vec::new();
    collect_xulo_sources(&root.join("xuloc/src"), &mut sources);
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
        assert_success(&out, &format!("self-parse of xuloc/src/{name}"));
        let text = stdout_of(&out);
        assert!(
            text.starts_with("Program @"),
            "xuloc/src/{name} produced no Program root:\n{text}"
        );
        assert!(
            text.contains("Fn "),
            "xuloc/src/{name} tree has no functions:\n{text}"
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
        .arg("xuloc/src/tests.xulo")
        .current_dir(&root)
        .output()
        .expect("xulo test should start");
    assert_success(&out, "xulo test xuloc/src/tests.xulo");
    let text = stdout_of(&out);
    assert!(
        text.contains("17 passed, 0 failed"),
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
        .arg(root.join("xuloc/src/main.xulo"))
        .current_dir(&root)
        .output()
        .expect("xulo check should start");
    assert_success(&out, "xulo check xuloc/src/main.xulo");
}

/// Gate 3: stage 1's checker agrees with stage 0 on accept/reject for the
/// fixed bootstrap corpus, and rejected files carry a file:line:column code.
#[test]
fn stage1_check_agrees_with_stage0_on_the_corpus() {
    let root = repo_root();
    let cases = [
        ("xuloc/corpus/accept/fibonacci.xulo", true),
        ("xuloc/corpus/accept/basics.xulo", true),
        ("xuloc/corpus/accept/modules.xulo", true),
        ("xuloc/corpus/accept/enums.xulo", true),
        ("xuloc/corpus/accept/aliases.xulo", true),
        ("xuloc/corpus/accept/methods.xulo", true),
        ("xuloc/corpus/accept/results.xulo", true),
        ("xuloc/corpus/accept/collections.xulo", true),
        ("xuloc/corpus/accept/closures.xulo", true),
        ("xuloc/corpus/accept/unions.xulo", true),
        ("xuloc/corpus/accept/concurrency.xulo", true),
        ("xuloc/corpus/accept/extern.xulo", true),
        ("xuloc/corpus/accept/generics.xulo", true),
        ("xuloc/corpus/accept/imports.xulo", true),
        ("xuloc/corpus/accept/generic_enums.xulo", true),
        ("xuloc/corpus/accept/generic_structs.xulo", true),
        ("xuloc/corpus/accept/generic_methods.xulo", true),
        ("xuloc/corpus/accept/traits.xulo", true),
        ("xuloc/corpus/reject/undefined_name.xulo", false),
        ("xuloc/corpus/reject/type_mismatch.xulo", false),
        ("xuloc/corpus/reject/arity.xulo", false),
        ("xuloc/corpus/reject/condition.xulo", false),
        ("xuloc/corpus/reject/not_iterable.xulo", false),
        ("xuloc/corpus/reject/return_mismatch.xulo", false),
        ("xuloc/corpus/reject/module_return_mismatch.xulo", false),
        ("xuloc/corpus/reject/struct_field_mismatch.xulo", false),
        ("xuloc/corpus/reject/alias_mismatch.xulo", false),
        ("xuloc/corpus/reject/list_element_mismatch.xulo", false),
        ("xuloc/corpus/reject/fn_value_arity.xulo", false),
        ("xuloc/corpus/reject/union_mismatch.xulo", false),
        ("xuloc/corpus/reject/extern_argument.xulo", false),
        ("xuloc/corpus/reject/try_non_result.xulo", false),
        ("xuloc/corpus/reject/generic_call_mismatch.xulo", false),
        ("xuloc/corpus/reject/trait_return_mismatch.xulo", false),
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

/// Template expressions and generic-enum payload bindings are name-resolved
/// even when the full type is outside the checker's subset.
#[test]
fn stage1_check_accepts_templated_generic_enums() {
    let root = repo_root();
    let out = run_stage1("check", &root.join("examples/types/enums_match.xulo"));
    assert_success(&out, "stage 1 check of examples/types/enums_match.xulo");
    assert!(
        stdout_of(&out).contains("no errors"),
        "stage 1 check should report success:\n{}",
        stdout_of(&out)
    );
}

/// Untyped object literals get an anonymous shape, so template member reads
/// (`${user.name}`) are checked rather than rejected.
#[test]
fn stage1_check_accepts_object_literals() {
    let root = repo_root();
    let out = run_stage1("check", &root.join("examples/basics/templates.xulo"));
    assert_success(&out, "stage 1 check of examples/basics/templates.xulo");
    assert!(
        stdout_of(&out).contains("no errors"),
        "stage 1 check should report success:\n{}",
        stdout_of(&out)
    );
}

/// Tuple literals, tuple type annotations, positional reads, and tuple
/// destructuring are modeled by the checker.
#[test]
fn stage1_check_accepts_tuples() {
    let root = repo_root();
    for relative in [
        "examples/basics/bindings.xulo",
        "examples/patterns/destructuring.xulo",
        "examples/types/tuples.xulo",
    ] {
        let out = run_stage1("check", &root.join(relative));
        assert_success(&out, &format!("stage 1 check of {relative}"));
        assert!(
            stdout_of(&out).contains("no errors"),
            "{relative} should check cleanly:\n{}",
            stdout_of(&out)
        );
    }
}

/// Inherent methods, omitted optional fields, typed object literals, and
/// struct patterns are all modeled.
#[test]
fn stage1_check_accepts_struct_methods() {
    let root = repo_root();
    let out = run_stage1("check", &root.join("examples/types/structs.xulo"));
    assert_success(&out, "stage 1 check of examples/types/structs.xulo");
    assert!(
        stdout_of(&out).contains("no errors"),
        "struct methods should check cleanly:\n{}",
        stdout_of(&out)
    );
}

/// Default parameters relax the call's minimum arity.
#[test]
fn stage1_check_accepts_default_params() {
    let root = repo_root();
    let out = run_stage1("check", &root.join("examples/functions/params.xulo"));
    assert_success(&out, "stage 1 check of examples/functions/params.xulo");
    assert!(
        stdout_of(&out).contains("no errors"),
        "default parameters should check cleanly:\n{}",
        stdout_of(&out)
    );
}

/// Ranges, optional chaining, and null narrowing are modeled.
#[test]
fn stage1_check_accepts_ranges_and_optional_chaining() {
    let root = repo_root();
    for relative in [
        "examples/basics/for.xulo",
        "examples/types/optional_chaining.xulo",
    ] {
        let out = run_stage1("check", &root.join(relative));
        assert_success(&out, &format!("stage 1 check of {relative}"));
        assert!(
            stdout_of(&out).contains("no errors"),
            "{relative} should check cleanly:\n{}",
            stdout_of(&out)
        );
    }
}

/// Builtin collection types and member calls are modeled.
#[test]
fn stage1_check_accepts_builtin_collections() {
    let root = repo_root();
    for relative in [
        "examples/collections/arrays.xulo",
        "examples/collections/maps.xulo",
        "examples/collections/sets.xulo",
        "examples/collections/collections.xulo",
    ] {
        let out = run_stage1("check", &root.join(relative));
        assert_success(&out, &format!("stage 1 check of {relative}"));
        assert!(
            stdout_of(&out).contains("no errors"),
            "{relative} should check cleanly:\n{}",
            stdout_of(&out)
        );
    }
}

/// Function types and closures are modeled, including calls through values.
#[test]
fn stage1_check_accepts_function_values() {
    let root = repo_root();
    for relative in [
        "examples/functions/closures_hof.xulo",
        "examples/std/regex.xulo",
    ] {
        let out = run_stage1("check", &root.join(relative));
        assert_success(&out, &format!("stage 1 check of {relative}"));
        assert!(
            stdout_of(&out).contains("no errors"),
            "{relative} should check cleanly:\n{}",
            stdout_of(&out)
        );
    }
}

/// Union types and `is` narrowing are modeled, and heterogeneous literals
/// infer `unknown` instead of failing.
#[test]
fn stage1_check_accepts_unions_and_narrowing() {
    let root = repo_root();
    for relative in [
        "examples/types/aliases.xulo",
        "examples/std/fmt.xulo",
        "examples/std/json.xulo",
        "examples/std/io_extra.xulo",
    ] {
        let out = run_stage1("check", &root.join(relative));
        assert_success(&out, &format!("stage 1 check of {relative}"));
        assert!(
            stdout_of(&out).contains("no errors"),
            "{relative} should check cleanly:\n{}",
            stdout_of(&out)
        );
    }
}

/// `async` functions, `await`, `spawn`, `lock`, and `shared` are modeled.
#[test]
fn stage1_check_accepts_concurrency() {
    let root = repo_root();
    for relative in [
        "examples/concurrency/async_tasks.xulo",
        "examples/concurrency/spawn_lock.xulo",
        "examples/testing/test_functions.xulo",
    ] {
        let out = run_stage1("check", &root.join(relative));
        assert_success(&out, &format!("stage 1 check of {relative}"));
        assert!(
            stdout_of(&out).contains("no errors"),
            "{relative} should check cleanly:\n{}",
            stdout_of(&out)
        );
    }
}

/// `extern "C"` declarations, `as` aliases, `Ptr`, and omitted optional
/// arguments are modeled.
#[test]
fn stage1_check_accepts_extern() {
    let root = repo_root();
    let relative = "examples/ffi/ffi.xulo";
    let out = run_stage1("check", &root.join(relative));
    assert_success(&out, &format!("stage 1 check of {relative}"));
    assert!(
        stdout_of(&out).contains("no errors"),
        "{relative} should check cleanly:\n{}",
        stdout_of(&out)
    );
}

/// Generic type/const parameters and trait declarations are modeled; trait
/// dispatch stays opaque.
#[test]
fn stage1_check_accepts_generics_and_traits() {
    let root = repo_root();
    for relative in [
        "examples/functions/generics.xulo",
        "examples/types/generics_impl.xulo",
        "examples/types/trait_shapes.xulo",
    ] {
        let out = run_stage1("check", &root.join(relative));
        assert_success(&out, &format!("stage 1 check of {relative}"));
        assert!(
            stdout_of(&out).contains("no errors"),
            "{relative} should check cleanly:\n{}",
            stdout_of(&out)
        );
    }
}

/// Macro declarations are skipped (their bodies run at compile time) and
/// macro invocations stay opaque.
#[test]
fn stage1_check_accepts_macro_declarations() {
    let root = repo_root();
    let relative = "examples/macros/macros.xulo";
    let out = run_stage1("check", &root.join(relative));
    assert_success(&out, &format!("stage 1 check of {relative}"));
    assert!(
        stdout_of(&out).contains("no errors"),
        "{relative} should check cleanly:\n{}",
        stdout_of(&out)
    );
}

/// External group imports and named function values are modeled.
#[test]
fn stage1_check_accepts_external_imports() {
    let root = repo_root();
    let relative = "examples/std/http.xulo";
    let out = run_stage1("check", &root.join(relative));
    assert_success(&out, &format!("stage 1 check of {relative}"));
    assert!(
        stdout_of(&out).contains("no errors"),
        "{relative} should check cleanly:\n{}",
        stdout_of(&out)
    );
}

/// T9: the stage 1 lexer/parser is measured against the example corpus, and
/// the gaps are a shrinking baseline.
///
/// The scan runs in one process; `crash` rows are skipped in that pass and
/// probed one by one, because a deep `match` nest exhausts the interpreter's
/// call depth and would otherwise hide every later file. A baseline row that
/// starts passing fails the test, so the list can only shrink.
#[test]
fn stage1_example_corpus_tracks_baseline() {
    use std::collections::BTreeMap;

    let root = repo_root();
    let corpus = xulo_examples::load(&root);
    let baseline = stage1_baseline();

    let examples = root.join("examples");
    // Scan output is relative to the repository root (`examples/basics/...`);
    // corpus paths are absolute. Both normalize to `<group>/<file>.xulo`.
    let relative = |path: &Path| -> String {
        let text = path.to_string_lossy().replace('\\', "/");
        if let Some(rest) = text.strip_prefix("examples/") {
            return rest.to_string();
        }
        text.strip_prefix(&format!(
            "{}/",
            examples.to_string_lossy().replace('\\', "/")
        ))
        .unwrap_or(&text)
        .to_string()
    };

    // Crash rows are named with the host's separator so the scan can match the
    // paths its own walk produces.
    let separator = std::path::MAIN_SEPARATOR.to_string();
    let skip: Vec<String> = baseline
        .iter()
        .filter(|(_, (stage, _))| stage == "crash")
        .map(|(path, _)| {
            format!(
                "examples{}{path}",
                std::path::MAIN_SEPARATOR,
                path = path.replace('/', &separator)
            )
        })
        .collect();

    let out = Command::new(BIN)
        .arg("run")
        .arg("xuloc/src/main.xulo")
        .current_dir(&root)
        .env("XULO_STAGE1_MODE", "scan")
        .env("XULO_STAGE1_DIR", "examples")
        .env("XULO_STAGE1_SKIP", skip.join(";"))
        .output()
        .expect("xulo run should start");
    assert!(
        out.status.success(),
        "the stage 1 scan failed:\n{}\n{}",
        stdout_of(&out),
        stderr_of(&out)
    );

    let mut seen: BTreeMap<String, (String, String)> = BTreeMap::new();
    for line in stdout_of(&out).lines() {
        let mut fields = line.trim_end().split('\t');
        let path = fields.next().unwrap_or("");
        let lex = fields.next().unwrap_or("");
        let parse = fields.next().unwrap_or("");
        if path.is_empty() {
            continue;
        }
        seen.insert(
            relative(Path::new(path)),
            (lex.to_string(), parse.to_string()),
        );
    }

    let mut problems: Vec<String> = Vec::new();
    for file in &corpus.files {
        let path = relative(file);
        let (_, (stage, _)) = match baseline.iter().find(|(known, _)| **known == path) {
            Some(entry) => entry,
            None => continue,
        };
        if stage == "crash" {
            continue;
        }
        let Some((lex, parse)) = seen.get(&path) else {
            problems.push(format!("{path}: the stage 1 scan did not report it"));
            continue;
        };
        let (status, kind) = if stage == "tokens" {
            (lex, "lexer")
        } else {
            (parse, "parser")
        };
        if status == "ok" {
            problems.push(format!(
                "{path}: the stage 1 {kind} now accepts it; remove its baseline row"
            ));
        }
    }

    for file in &corpus.files {
        let path = relative(file);
        if baseline.contains_key(&path) {
            continue;
        }
        let Some((lex, parse)) = seen.get(&path) else {
            problems.push(format!("{path}: the stage 1 scan did not report it"));
            continue;
        };
        if lex != "ok" {
            problems.push(format!(
                "{path}: stage 1 lexer rejects it ({lex}); record the gap in tests/stage1_examples_baseline.tsv"
            ));
        }
        if parse != "ok" {
            problems.push(format!(
                "{path}: stage 1 parser rejects it ({parse}); record the gap in tests/stage1_examples_baseline.tsv"
            ));
        }
    }

    // A `crash` row must still crash; otherwise the scan no longer needs the skip.
    for (path, (stage, _)) in &baseline {
        if stage != "crash" {
            continue;
        }
        let file = examples.join(path);
        let probe = Command::new(BIN)
            .arg("run")
            .arg("xuloc/src/main.xulo")
            .current_dir(&root)
            .env("XULO_STAGE1_MODE", "parse")
            .env("XULO_STAGE1_FILE", &file)
            .output()
            .expect("xulo run should start");
        if probe.status.success() {
            problems.push(format!(
                "{path}: stage 1 parses it now; remove its `crash` baseline row"
            ));
            continue;
        }
        let err = stderr_of(&probe);
        if !err.contains("call depth exceeded") {
            problems.push(format!(
                "{path}: expected a call-depth failure, got:\n{err}"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "stage 1's example coverage moved:\n\n{}",
        problems.join("\n")
    );
}

fn stage1_baseline() -> std::collections::BTreeMap<String, (String, String)> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/stage1_examples_baseline.tsv");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut rows = std::collections::BTreeMap::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            fields.len(),
            3,
            "{}:{}: expected `path<TAB>stage<TAB>reason`",
            path.display(),
            number + 1
        );
        let stage = fields[1];
        assert!(
            stage == "tokens" || stage == "parse" || stage == "crash",
            "{}:{}: stage must be tokens, parse, or crash",
            path.display(),
            number + 1
        );
        rows.insert(
            fields[0].to_string(),
            (stage.to_string(), fields[2].to_string()),
        );
    }
    rows
}
