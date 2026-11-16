//! Readiness-matrix audit (`docs/tasks.md` T3d).
//!
//! `docs/stage1-readiness.md` §5 marks a capability **OK** only when a test or
//! a named probe backs it. This test enforces that claim:
//!
//! - every row whose status is `**OK**` must cite at least one existing
//!   repository path in its evidence cell (or the workspace-wide
//!   `cargo test --workspace` command for T1, which *is* the suite);
//! - every probe under `tests/matrix_probes/` is executed here, so a cited
//!   probe is a live check rather than a filename; and
//! - every probe on disk is cited by at least one OK row, so the corpus cannot
//!   rot after a row is removed or re-graded.
//!
//! Probe format: a `.xulo` file whose header comments carry directives.
//!
//! ```text
//! // @expect
//! // first stdout line
//! // second stdout line
//! // @error E0006     (or)  // @panic substring  (or)  // @check
//! ```
//!
//! `@expect` collects the comment lines that follow it. Without `@error` or
//! `@panic`, `xulo run` must exit 0 and print exactly that text (`@check` runs
//! `xulo check` instead and expects success). A probe directory with a
//! `main.xulo` entry point is a multi-file module probe.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_xulo");

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the repository root")
        .to_path_buf()
}

fn readiness_path() -> PathBuf {
    repo_root().join("docs").join("stage1-readiness.md")
}

fn probes_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("matrix_probes")
}

/// A row of one of the §5 capability tables.
struct Row {
    id: String,
    status: String,
    evidence: String,
}

fn matrix_rows(text: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() != 5 {
            continue;
        }
        let id = cells[0];
        let is_id = id.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && id.chars().skip(1).any(|c| c.is_ascii_digit())
            && id.chars().all(|c| c.is_ascii_alphanumeric())
            && !id.contains(char::is_whitespace);
        if !is_id {
            continue;
        }
        rows.push(Row {
            id: id.to_string(),
            status: cells[2].to_string(),
            evidence: cells[3].to_string(),
        });
    }
    rows
}

fn backticked(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('`') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('`') else {
            break;
        };
        out.push(&after[..end]);
        rest = &after[end + 1..];
    }
    out
}

/// Whether a backticked token names something the repository can point at.
fn is_proof(root: &Path, token: &str) -> bool {
    if token == "cargo test --workspace" {
        return true;
    }
    let path_like = token.contains('/')
        || token.ends_with(".rs")
        || token.ends_with(".xulo")
        || token.ends_with(".tsv")
        || token.ends_with(".yml");
    path_like && root.join(token).exists()
}

/// Discover probes: top-level `.xulo` files plus directories with `main.xulo`.
fn discover_probes(dir: &Path) -> Vec<PathBuf> {
    let mut probes = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return probes;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            let main = path.join("main.xulo");
            if main.is_file() {
                probes.push(main);
            }
        } else if path.extension().is_some_and(|ext| ext == "xulo") {
            probes.push(path);
        }
    }
    probes
}

struct Probe {
    /// `path/to/probe.xulo`, repository-relative and always `/`-separated.
    id: String,
    expect_stdout: Vec<String>,
    expect_error: Option<String>,
    expect_panic: Option<String>,
    check_only: bool,
}

fn parse_probe(path: &Path) -> Probe {
    let text = fs::read_to_string(path).expect("read probe");
    let mut probe = Probe {
        id: path
            .strip_prefix(repo_root())
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/"),
        expect_stdout: Vec::new(),
        expect_error: None,
        expect_panic: None,
        check_only: false,
    };
    let mut in_expect = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        let Some(comment) = trimmed.strip_prefix("//") else {
            in_expect = false;
            continue;
        };
        let comment = comment.strip_prefix(' ').unwrap_or(comment);
        if let Some(rest) = comment.strip_prefix("@expect") {
            assert!(
                rest.trim().is_empty(),
                "{}: @expect takes no argument",
                probe.id
            );
            in_expect = true;
            continue;
        }
        if let Some(rest) = comment.strip_prefix("@error") {
            probe.expect_error = Some(rest.trim().to_string());
            in_expect = false;
            continue;
        }
        if let Some(rest) = comment.strip_prefix("@panic") {
            probe.expect_panic = Some(rest.trim().to_string());
            in_expect = false;
            continue;
        }
        if comment.trim() == "@check" {
            probe.check_only = true;
            in_expect = false;
            continue;
        }
        if in_expect {
            probe.expect_stdout.push(comment.to_string());
        }
    }
    probe
}

struct Failure {
    id: String,
    message: String,
}

fn run_probe(probe: &Probe) -> Result<(), String> {
    let path = repo_root().join(&probe.id);
    let subcommand = if probe.check_only { "check" } else { "run" };
    let out = Command::new(BIN)
        .arg(subcommand)
        .arg(&path)
        .output()
        .map_err(|err| format!("cannot run {BIN}: {err}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    if let Some(code) = &probe.expect_error {
        if out.status.success() {
            return Err(format!(
                "expected a runtime failure with {code}, but it exited 0"
            ));
        }
        if !code.is_empty() && !stderr.contains(code) {
            return Err(format!("expected {code} on stderr, got:\n{stderr}"));
        }
        return Ok(());
    }
    if let Some(message) = &probe.expect_panic {
        if out.status.success() {
            return Err(format!(
                "expected a panic containing `{message}`, but it exited 0"
            ));
        }
        if !message.is_empty() && !stderr.contains(message) {
            return Err(format!("expected `{message}` on stderr, got:\n{stderr}"));
        }
        return Ok(());
    }
    if !out.status.success() {
        return Err(format!("`xulo {subcommand}` failed:\n{stderr}"));
    }
    let actual: Vec<&str> = stdout.lines().collect();
    let expected: Vec<&str> = probe.expect_stdout.iter().map(String::as_str).collect();
    if actual != expected {
        return Err(format!(
            "stdout mismatch\n  expected: {expected:?}\n  actual:   {actual:?}"
        ));
    }
    Ok(())
}

#[test]
fn every_ok_row_cites_a_proof() {
    let root = repo_root();
    let text = fs::read_to_string(readiness_path()).expect("read docs/stage1-readiness.md");
    let rows = matrix_rows(&text);
    assert!(
        rows.iter().any(|row| row.status.contains("OK")),
        "the §5 matrix was not parsed; check its table shape"
    );

    let mut missing: Vec<String> = Vec::new();
    for row in rows.iter().filter(|row| row.status.contains("OK")) {
        let cited = backticked(&row.evidence);
        if !cited.iter().any(|token| is_proof(&root, token)) {
            let tokens = if cited.is_empty() {
                "<none>".to_string()
            } else {
                cited.join(", ")
            };
            missing.push(format!(
                "{}: evidence cites no existing path or named probe (tokens: {tokens})",
                row.id
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "OK rows need a test, workflow, or probe path in the evidence column:\n  {}",
        missing.join("\n  ")
    );
}

#[test]
fn probes_pass_and_are_cited() {
    let text = fs::read_to_string(readiness_path()).expect("read docs/stage1-readiness.md");
    // Any matrix row may cite a probe (a PARTIAL row documents its limitation
    // with one); only OK rows must cite a passing proof.
    let evidence: Vec<String> = matrix_rows(&text)
        .iter()
        .map(|row| row.evidence.clone())
        .collect();

    let probes = discover_probes(&probes_dir());
    assert!(
        !probes.is_empty(),
        "no probes found under tests/matrix_probes"
    );

    let mut failures: Vec<Failure> = Vec::new();
    for path in probes {
        let probe = parse_probe(&path);
        let cited = evidence.iter().any(|cell| cell.contains(&probe.id));
        if !cited {
            failures.push(Failure {
                id: probe.id.clone(),
                message: "probe is not cited by any OK row's evidence".to_string(),
            });
        }
        if let Err(message) = run_probe(&probe) {
            failures.push(Failure {
                id: probe.id.clone(),
                message,
            });
        }
    }
    assert!(
        failures.is_empty(),
        "readiness probes failed:\n{}",
        failures
            .iter()
            .map(|failure| format!("  {}: {}", failure.id, failure.message))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
