//! The `examples/` gate: the corpus is checked, executed, and compared against
//! its recorded output on every `cargo test --workspace`.
//!
//! Discovery, header directives, and sidecar resolution come from
//! `xulo-examples`, shared with `crates/xupm/tests/aot_differential.rs`, so the
//! interpreter gate and the native gate cannot disagree about what an example
//! is or what it should print.
//!
//! ```text
//! XULO_BLESS=1 cargo test -p xulo-cli --test examples   # rewrite sidecars
//! XULO_BIN=target/release/xulo cargo test -p xulo-cli --test examples
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use xulo_examples::Corpus;

const BIN: &str = env!("CARGO_BIN_EXE_xulo");

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crates/xulo-cli has a repository parent")
        .to_path_buf()
}

/// The binary under test. CI points this at the release build.
fn xulo_bin() -> PathBuf {
    match std::env::var_os("XULO_BIN") {
        Some(path) => PathBuf::from(path),
        None => PathBuf::from(BIN),
    }
}

fn corpus() -> Corpus {
    xulo_examples::load(&repo_root())
}

fn run(args: &[&str]) -> Output {
    Command::new(xulo_bin())
        .args(args)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("cannot run {}: {e}", xulo_bin().display()))
}

fn render(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn bless() -> bool {
    std::env::var_os("XULO_BLESS").is_some()
}

/// Every unit has a distinct relative path, and the corpus is big enough that a
/// broken walk cannot pass silently.
#[test]
fn corpus_is_discoverable() {
    let corpus = corpus();
    assert!(
        corpus.units.len() >= 40,
        "expected at least 40 example programs, found {}:\n{:#?}",
        corpus.units.len(),
        corpus
            .units
            .iter()
            .map(|unit| &unit.rel)
            .collect::<Vec<_>>()
    );
    let mut seen = BTreeMap::new();
    for unit in &corpus.units {
        if let Some(previous) = seen.insert(unit.rel.clone(), unit.entry.clone()) {
            panic!(
                "duplicate example `{}`: {} and {}",
                unit.rel,
                previous.display(),
                unit.entry.display()
            );
        }
    }
}

/// `xulo check` accepts every example, except those that declare the static
/// error they are meant to demonstrate.
#[test]
fn examples_check_clean() {
    let corpus = corpus();
    let mut failures = Vec::new();
    for unit in &corpus.units {
        let entry = unit.entry.to_str().expect("an example path is UTF-8");
        let out = run(&["check", entry]);
        let stderr = render(&out.stderr);
        match (&unit.directives.error, out.status.success()) {
            (None, true) => {}
            (None, false) => failures.push(format!("{}: `xulo check` failed\n{stderr}", unit.rel)),
            (Some(code), false) if stderr.contains(&format!("error[{code}]")) => {}
            (Some(code), false) => failures.push(format!(
                "{}: expected error[{code}], got\n{stderr}",
                unit.rel
            )),
            (Some(code), true) => failures.push(format!(
                "{}: expected error[{code}] but `xulo check` passed",
                unit.rel
            )),
        }
    }
    assert!(
        failures.is_empty(),
        "example checks failed:\n\n{}",
        failures.join("\n\n")
    );
}

/// Runs every example and compares stdout/stderr with its sidecars.
#[test]
fn examples_match_expected_output() {
    let corpus = corpus();
    let mut failures = Vec::new();
    let mut missing = Vec::new();
    let mut executed = 0;

    for unit in &corpus.units {
        if unit.directives.error.is_some() {
            continue;
        }
        if let Some(reason) = unit.directives.no_run_here() {
            eprintln!("check-only: {}: {reason}", unit.rel);
            continue;
        }

        let entry = unit.entry.to_str().expect("an example path is UTF-8");
        let out = run(&["run", entry]);
        executed += 1;
        let stderr = xulo_examples::normalize(&out.stderr);

        if let Some(expected) = &unit.directives.panic {
            if out.status.success() {
                failures.push(format!(
                    "{}: expected `xulo run` to fail containing `{expected}`",
                    unit.rel
                ));
                continue;
            }
            if !stderr.contains(expected.as_str()) {
                failures.push(format!(
                    "{}: stderr does not contain `{expected}`\n{stderr}",
                    unit.rel
                ));
                continue;
            }
        } else if !out.status.success() {
            failures.push(format!("{}: `xulo run` failed\n{stderr}", unit.rel));
            continue;
        }

        if bless() {
            std::fs::write(xulo_examples::bless_target(&unit.entry, false), &out.stdout)
                .expect("cannot write the stdout sidecar");
            if !out.stderr.is_empty() {
                std::fs::write(xulo_examples::bless_target(&unit.entry, true), &out.stderr)
                    .expect("cannot write the stderr sidecar");
            }
            continue;
        }

        if xulo_examples::sidecar(&unit.entry, false).is_none() {
            missing.push(unit.rel.clone());
            continue;
        }
        if let Some(report) = xulo_examples::mismatch(&unit.entry, false, &out.stdout) {
            failures.push(report);
        }
        match xulo_examples::sidecar(&unit.entry, true) {
            Some(_) => {
                if let Some(report) = xulo_examples::mismatch(&unit.entry, true, &out.stderr) {
                    failures.push(report);
                }
            }
            None => {
                if !stderr.is_empty() {
                    failures.push(format!(
                        "{}: `xulo run` wrote to stderr without a sidecar\n{stderr}",
                        unit.rel
                    ));
                }
            }
        }
    }

    assert!(
        missing.is_empty(),
        "examples without a recorded stdout sidecar:\n{}\n\nrun `XULO_BLESS=1 cargo test -p xulo-cli --test examples` and review the diff",
        missing.join("\n")
    );
    assert!(
        failures.is_empty(),
        "examples printed unexpected output:\n\n{}",
        failures.join("\n\n")
    );
    assert!(executed > 0, "no example was executed");
}

/// Examples that document the test harness also assert the report line.
#[test]
fn examples_declare_their_tests() {
    let corpus = corpus();
    let mut failures = Vec::new();
    let mut counted = 0;
    for unit in &corpus.units {
        let Some(expected) = &unit.directives.test_expect else {
            continue;
        };
        counted += 1;
        let entry = unit.entry.to_str().expect("an example path is UTF-8");
        let out = run(&["test", entry]);
        let stdout = render(&out.stdout);
        if !stdout.contains(expected.as_str()) {
            failures.push(format!(
                "{}: expected `{expected}` in the report\nstdout:\n{stdout}\nstderr:\n{}",
                unit.rel,
                render(&out.stderr)
            ));
        }
    }
    assert!(counted > 0, "no example declares `@test-expect`");
    assert!(
        failures.is_empty(),
        "example test reports changed:\n\n{}",
        failures.join("\n\n")
    );
}

/// The corpus must not accumulate examples that no gate reads.
#[test]
fn every_example_entry_is_reachable() {
    let corpus = corpus();
    for unit in &corpus.units {
        assert!(
            unit.entry.is_file(),
            "{} does not exist",
            unit.entry.display()
        );
    }
}

/// `examples/coverage.tsv` accounts for every production of
/// `spec/grammar.md`'s index: each one names an existing example that really
/// contains the probe text, or carries an explicit reason for the exemption.
#[test]
fn coverage_table_accounts_for_grammar() {
    let root = repo_root();
    let corpus = corpus();
    let grammar = std::fs::read_to_string(root.join("spec/grammar.md"))
        .expect("spec/grammar.md must be present (it is a pinned submodule)");

    // The index is the source of truth: `| `Name`, `Name` | [Section] |` rows.
    let index_at = grammar
        .find("## Production index")
        .expect("spec/grammar.md has a production index");
    let mut productions: Vec<String> = Vec::new();
    for line in grammar[index_at..].lines() {
        if !line.starts_with("| `") {
            continue;
        }
        for part in line.split('`').skip(1).step_by(2) {
            if let Some(name) = backticked_production(part) {
                productions.push(name);
            }
        }
    }
    assert!(
        productions.len() > 150,
        "the production index looks truncated: {} entries",
        productions.len()
    );

    let table = std::fs::read_to_string(root.join("examples/coverage.tsv"))
        .expect("examples/coverage.tsv must exist");
    let mut rows: BTreeMap<String, (String, String, String)> = BTreeMap::new();
    let mut sources: BTreeMap<String, String> = BTreeMap::new();
    for unit in &corpus.files {
        let rel = unit
            .strip_prefix(root.join("examples"))
            .unwrap_or(unit)
            .to_string_lossy()
            .replace('\\', "/");
        let source = std::fs::read_to_string(unit).expect("an example file is readable");
        sources.insert(rel, source);
    }

    let mut problems: Vec<String> = Vec::new();
    for (number, line) in table.lines().enumerate() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() != 4 {
            problems.push(format!(
                "coverage.tsv:{}: expected 4 tab-separated fields, found {}",
                number + 1,
                fields.len()
            ));
            continue;
        }
        let (production, path, probe, note) = (fields[0], fields[1], fields[2], fields[3]);
        if rows
            .insert(
                production.to_string(),
                (path.to_string(), probe.to_string(), note.to_string()),
            )
            .is_some()
        {
            problems.push(format!(
                "coverage.tsv:{}: duplicate row `{production}`",
                number + 1
            ));
            continue;
        }
        if path == "-" {
            if !note.starts_with("trivia:")
                && !note.starts_with("terminal:")
                && !note.starts_with("abstract:")
                && !note.starts_with("error-only:")
                && !note.starts_with("reserved:")
            {
                problems.push(format!(
                    "coverage.tsv:{}: `{production}` is exempt without a recognised reason",
                    number + 1
                ));
            }
            continue;
        }
        let Some(source) = sources.get(path) else {
            problems.push(format!(
                "coverage.tsv:{}: `{production}` names `{path}`, which is not an example file",
                number + 1
            ));
            continue;
        };
        if probe != "-" && !source.contains(probe) {
            problems.push(format!(
                "coverage.tsv:{}: `{production}` probe `{probe}` does not appear in {path}",
                number + 1
            ));
        }
    }

    let listed: std::collections::BTreeSet<&String> = rows.keys().collect();
    let indexed: std::collections::BTreeSet<&String> = productions.iter().collect();
    for production in indexed.difference(&listed) {
        problems.push(format!(
            "spec/grammar.md lists `{production}` but examples/coverage.tsv does not"
        ));
    }
    for production in listed.difference(&indexed) {
        problems.push(format!(
            "examples/coverage.tsv lists `{production}`, which is not in the production index"
        ));
    }

    // A mapped example must actually be part of the corpus.
    let known: std::collections::BTreeSet<String> =
        corpus.units.iter().map(|unit| unit.rel.clone()).collect();
    for (production, (path, _, _)) in &rows {
        if path == "-" {
            continue;
        }
        let unit_owns_file = known.iter().any(|entry| {
            let dir = entry.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");
            path == entry || (!dir.is_empty() && path.starts_with(&format!("{dir}/")))
        });
        if !unit_owns_file {
            problems.push(format!(
                "coverage.tsv: `{production}` names `{path}`, which no example unit owns"
            ));
        }
    }

    assert!(
        problems.is_empty(),
        "the example coverage table is out of sync with spec/grammar.md:\n\n{}",
        problems.join("\n")
    );
}

/// A production name from a backticked index cell: `SourceFile`, not `[Section]`.
fn backticked_production(cell: &str) -> Option<String> {
    let name = cell.trim();
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_uppercase()) {
        return None;
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(name.to_string())
}
