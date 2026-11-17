//! Spec conformance runner (WP0, `docs/tasks.md` T3a–T3d).
//!
//! Extracts every fenced ```` ```xulo ```` block from the pinned `spec/`
//! submodule and turns its annotations into checkable cases:
//!
//! - a block that marks an accepted form (`// ok`, `// legal`, `// fine`,
//!   `// well-formed`) is checked after the rejected (`// error…`) lines are
//!   removed: stage 0 must accept it;
//! - every `// error[E0123]` line is re-inserted uncommented into that program:
//!   stage 0 must reject it, with that code when the annotation names one;
//! - every `// runs: …` annotation is executed: stdout must match.
//!
//! Blocks that are inherently not self-contained are skipped: ones containing
//! an ellipsis (`…` / `...`) and multi-file examples (a `// name.xulo` marker).
//!
//! Divergences are recorded in `spec_conformance_baseline.tsv` with a triage
//! reason, because the readiness matrix is the interim record of known
//! deviations. The test fails when a case diverges and is not baselined, and
//! also when a baselined case starts passing — a stale entry must be removed
//! (and the matrix updated) rather than silently kept.
//!
//! Rewrite the baseline after an intentional spec bump or fix with:
//!
//! ```text
//! XULO_SPEC_CONFORMANCE_BLESS=1 cargo test -p xulo-cli --test spec_conformance
//! ```
//!
//! then review the diff and add a reason to every new line.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_xulo");

/// Annotations that assert the block (minus its rejected lines) is well-formed.
const ACCEPT_MARKERS: [&str; 7] = ["ok", "OK", "Ok", "legal", "Legal", "fine", "well"];

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// `xulo check` must succeed.
    Accept,
    /// `xulo check` must fail, ideally with the annotated code.
    Reject,
    /// `xulo run` must succeed and print the annotated text.
    Runs,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Accept => "ok",
            Kind::Reject => "error",
            Kind::Runs => "runs",
        }
    }
}

struct Case {
    id: String,
    kind: Kind,
    /// The program to check or run.
    source: String,
    /// The error code the annotation names, if any.
    expect_code: Option<String>,
    /// The text the program must print, for [`Kind::Runs`].
    expect_stdout: Option<String>,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the repository root")
        .to_path_buf()
}

/// The text after a line comment's `//`, trimmed. `None` when the line has no
/// comment. Heuristic: the first `//` starts the comment, so a `//` inside a
/// string literal would confuse this — no pinned example does that today.
fn comment_text(line: &str) -> Option<&str> {
    let at = line.find("//")?;
    Some(line[at + 2..].trim_start())
}

fn word(rest: &str) -> &str {
    let end = rest
        .find(|c: char| !c.is_alphanumeric() && c != '-')
        .unwrap_or(rest.len());
    &rest[..end]
}

fn is_accept_marker(line: &str) -> bool {
    comment_text(line).is_some_and(|rest| ACCEPT_MARKERS.contains(&word(rest)))
}

/// `Some(code)` for `// error[E0123]`, `Some(None)` for a bare `// error`.
fn error_marker(line: &str) -> Option<Option<String>> {
    let rest = comment_text(line)?.strip_prefix("error")?;
    if !rest.is_empty() && !rest.starts_with(['[', ':', ' ']) {
        return None;
    }
    let code = rest
        .strip_prefix('[')
        .and_then(|rest| rest.split_once(']'))
        .map(|(code, _)| code.trim())
        .filter(|code| {
            code.len() == 5
                && code.starts_with('E')
                && code[1..].bytes().all(|b| b.is_ascii_digit())
        });
    Some(code.map(str::to_string))
}

fn runs_marker(line: &str) -> Option<&str> {
    comment_text(line)?.strip_prefix("runs:").map(str::trim)
}

/// Remove the common leading indentation of a markdown-nested block.
fn dedent(body: &[String]) -> Vec<String> {
    let indent = body
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    body.iter()
        .map(|line| {
            if line.len() >= indent {
                line[indent..].to_string()
            } else {
                line.clone()
            }
        })
        .collect()
}

fn collect_markdown(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.path());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            // `changes/` is the historical, non-normative change archive.
            if path.file_name().is_some_and(|name| name == "changes") {
                continue;
            }
            collect_markdown(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "md") {
            out.push(path);
        }
    }
}

/// Build the cases for one markdown file relative to the repository root.
fn cases_in_file(root: &Path, path: &Path, cases: &mut Vec<Case>) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let rel = path
        .strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string();
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        if lines[index].trim() != "```xulo" {
            index += 1;
            continue;
        }
        let open = index; // 0-based fence line
        index += 1;
        let mut body = Vec::new();
        while index < lines.len() && lines[index].trim() != "```" {
            body.push(lines[index].to_string());
            index += 1;
        }
        index += 1; // closing fence
        let body = dedent(&body);
        add_block_cases(&rel, open + 2, &body, cases);
    }
}

/// One-based line number of `body[at]` inside the markdown file.
fn add_block_cases(rel: &str, first_line: usize, body: &[String], cases: &mut Vec<Case>) {
    let self_contained = !body
        .iter()
        .any(|line| line.contains('…') || line.contains("..."));
    let multi_file = body.iter().any(|line| {
        comment_text(line).is_some_and(|rest| {
            let rest = rest.trim();
            !rest.is_empty()
                && rest.ends_with(".xulo")
                && !rest.contains(' ')
                && !rest.contains('\t')
        })
    });
    if !self_contained || multi_file {
        return;
    }

    let rejected: Vec<usize> = (0..body.len())
        .filter(|&at| error_marker(&body[at]).is_some() && !is_accept_marker(&body[at]))
        .collect();
    let accepted = body.iter().any(|line| is_accept_marker(line));
    // The program an accepted/rejected annotation is checked against: the
    // block with its rejected lines removed.
    let base: Vec<String> = body
        .iter()
        .enumerate()
        .filter(|(at, _)| !rejected.contains(at))
        .map(|(_, line)| line.clone())
        .collect();

    if accepted {
        cases.push(Case {
            id: format!("{rel}:{first_line} {}", Kind::Accept.name()),
            kind: Kind::Accept,
            source: base.join("\n") + "\n",
            expect_code: None,
            expect_stdout: None,
        });
    }

    for at in rejected {
        let line = &body[at];
        let Some(code) = error_marker(line) else {
            continue;
        };
        let mut source = base.clone();
        source.insert(at.min(source.len()), uncomment(line));
        cases.push(Case {
            id: format!("{rel}:{} {}", first_line + at, Kind::Reject.name()),
            kind: Kind::Reject,
            source: source.join("\n") + "\n",
            expect_code: code,
            expect_stdout: None,
        });
    }

    for (at, line) in body.iter().enumerate() {
        if let Some(expected) = runs_marker(line) {
            cases.push(Case {
                id: format!("{rel}:{} {}", first_line + at, Kind::Runs.name()),
                kind: Kind::Runs,
                source: base.join("\n") + "\n",
                expect_code: None,
                expect_stdout: Some(expected.to_string()),
            });
        }
    }
}

/// Turn `    // let x = 1 // error…` into `    let x = 1 // error…`.
fn uncomment(line: &str) -> String {
    match line.find("//") {
        Some(at) => format!("{}{}", &line[..at], &line[at + 2..]),
        None => line.to_string(),
    }
}

/// Write `source` to a unique temporary file and return its path.
fn stage(case: &Case, dir: &Path) -> PathBuf {
    let name = case
        .id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>();
    let path = dir.join(format!("{name}.xulo"));
    fs::write(&path, &case.source).expect("write staged spec example");
    path
}

struct Outcome {
    passed: bool,
    actual: String,
}

fn run_case(case: &Case, file: &Path) -> Outcome {
    match case.kind {
        Kind::Accept => {
            let out = Command::new(BIN).args(["check"]).arg(file).output();
            match out {
                Ok(out) if out.status.success() => Outcome {
                    passed: true,
                    actual: String::new(),
                },
                Ok(out) => Outcome {
                    passed: false,
                    actual: error_codes(&out.stderr),
                },
                Err(err) => panic!("cannot run {BIN}: {err}"),
            }
        }
        Kind::Reject => {
            let out = Command::new(BIN).args(["check"]).arg(file).output();
            match out {
                Ok(out) if out.status.success() => Outcome {
                    passed: false,
                    actual: "accepted".to_string(),
                },
                Ok(out) => {
                    let seen = error_codes(&out.stderr);
                    let passed = match &case.expect_code {
                        Some(code) => seen.contains(code),
                        None => true,
                    };
                    Outcome {
                        passed,
                        actual: seen,
                    }
                }
                Err(err) => panic!("cannot run {BIN}: {err}"),
            }
        }
        Kind::Runs => {
            let out = Command::new(BIN).args(["run"]).arg(file).output();
            match out {
                Ok(out) if out.status.success() => {
                    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    let expected = case.expect_stdout.clone().unwrap_or_default();
                    let passed = stdout == expected;
                    Outcome {
                        passed,
                        actual: stdout,
                    }
                }
                Ok(out) => Outcome {
                    passed: false,
                    actual: error_codes(&out.stderr),
                },
                Err(err) => panic!("cannot run {BIN}: {err}"),
            }
        }
    }
}

/// The `E0123` codes stage 0 reported, comma-separated and de-duplicated.
fn error_codes(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let mut codes = BTreeSet::new();
    for (at, _) in text.match_indices("error[E") {
        let code = &text[at + "error[".len()..];
        if let Some(end) = code.find(']') {
            codes.insert(code[..end].to_string());
        }
    }
    if codes.is_empty() {
        text.lines()
            .next()
            .unwrap_or("no diagnostic")
            .trim()
            .to_string()
    } else {
        codes.into_iter().collect::<Vec<_>>().join(",")
    }
}

fn baseline_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("spec_conformance_baseline.tsv")
}

/// `id -> reason`. Each data line is `<id>\t<reason>`; `#` starts a comment.
fn read_baseline() -> BTreeMap<String, String> {
    let path = baseline_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return BTreeMap::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
        .map(|line| match line.split_once('\t') {
            Some((id, reason)) => (id.to_string(), reason.trim().to_string()),
            None => (line.trim().to_string(), String::new()),
        })
        .collect()
}

fn write_baseline(entries: &BTreeMap<String, String>) {
    let mut text = String::from(
        "# Spec conformance divergences, triaged in docs/stage1-readiness.md §6.\n\
         # Format: <case id>\\t<reason>. Regenerate with XULO_SPEC_CONFORMANCE_BLESS=1;\n\
         # review every line you add or remove, and update the readiness matrix.\n",
    );
    for (id, reason) in entries {
        text.push_str(id);
        text.push('\t');
        text.push_str(reason);
        text.push('\n');
    }
    fs::write(baseline_path(), text).expect("write conformance baseline");
}

#[test]
fn pinned_spec_examples_conform() {
    let root = repo_root();
    let spec = root.join("spec");
    if !spec.is_dir() {
        eprintln!(
            "skipping spec conformance: `{}` is absent \
             (check out submodules with `git submodule update --init`)",
            spec.display()
        );
        return;
    }

    let mut files = Vec::new();
    collect_markdown(&spec, &mut files);
    // The pinned spec has no `// runs:` examples today; these local cases keep
    // the executable path covered (T3c).
    let local = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("runs_cases");
    collect_markdown(&local, &mut files);
    let mut cases = Vec::new();
    for file in &files {
        cases_in_file(&root, file, &mut cases);
    }
    cases.sort_by(|a, b| a.id.cmp(&b.id));
    assert!(
        cases.len() > 50,
        "expected the pinned spec to yield annotated examples, found {}",
        cases.len()
    );
    assert!(
        cases.iter().any(|case| case.kind == Kind::Runs),
        "expected at least one `// runs:` case under {}",
        local.display()
    );

    let dir = std::env::temp_dir().join(format!(
        "xulo_spec_conformance_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&dir).expect("create staging directory");

    let baseline = read_baseline();
    let mut known = 0usize;
    let mut passed = 0usize;
    let mut unexpected = Vec::new();
    let mut stale = Vec::new();
    let mut failing = BTreeSet::new();

    for case in &cases {
        let file = stage(case, &dir);
        let outcome = run_case(case, &file);
        let baselined = baseline.contains_key(&case.id);
        match (outcome.passed, baselined) {
            (true, false) => passed += 1,
            (true, true) => {
                passed += 1;
                stale.push(case.id.clone());
            }
            (false, true) => {
                known += 1;
                failing.insert(case.id.clone());
            }
            (false, false) => {
                unexpected.push(format!(
                    "{}: {}{}",
                    case.id,
                    match case.kind {
                        Kind::Accept => "expected acceptance, got ",
                        Kind::Reject => "expected rejection, got ",
                        Kind::Runs => "expected output, got ",
                    },
                    outcome.actual
                ));
                failing.insert(case.id.clone());
            }
        }
    }
    for id in baseline.keys() {
        if !cases.iter().any(|case| &case.id == id) {
            stale.push(id.clone());
        }
    }
    let _ = fs::remove_dir_all(&dir);

    if std::env::var_os("XULO_SPEC_CONFORMANCE_BLESS").is_some() {
        let merged: BTreeMap<String, String> = failing
            .into_iter()
            .map(|id| {
                let reason = baseline.get(&id).cloned().unwrap_or_default();
                (id, reason)
            })
            .collect();
        write_baseline(&merged);
        eprintln!("baseline rewritten: {} known divergences", merged.len());
        return;
    }

    println!(
        "spec conformance: {passed} passed, {known} known divergences, {} cases",
        cases.len()
    );

    let mut problems = Vec::new();
    if !unexpected.is_empty() {
        problems.push(format!(
            "{} spec example(s) diverged and are not baselined:\n  {}",
            unexpected.len(),
            unexpected.join("\n  ")
        ));
    }
    if !stale.is_empty() {
        problems.push(format!(
            "{} baselined case(s) now pass or vanished (remove them from \
             spec_conformance_baseline.tsv and update docs/stage1-readiness.md):\n  {}",
            stale.len(),
            stale.join("\n  ")
        ));
    }
    assert!(
        problems.is_empty(),
        "spec conformance failed\n\n{}",
        problems.join("\n\n")
    );
}
