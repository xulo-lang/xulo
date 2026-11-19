//! Repository-state gate (WP0, `docs/tasks.md` T7).
//!
//! The readiness matrix is written against `spec@815edad` and
//! `std-spec@a2e4cb7`. A working tree detached at another submodule commit
//! silently invalidates probes (the document calls this out in its repository
//! state note), so CI must fail when the checkout drifts from the pin.
//!
//! Three things must agree:
//!
//! 1. the commit the superproject records in `HEAD` (the pin itself),
//! 2. the commit staged in the index (`git submodule status`),
//! 3. the commit the submodule working tree is checked out at, when it is
//!    initialized.
//!
//! Any deliberate bump must update `PINS`, this repository's pin note in
//! `docs/stage1-readiness.md` §1, and the conformance baseline at once.

use std::path::Path;
use std::process::Command;

/// The submodule commits this repository's matrix is written against.
const PINS: [(&str, &str); 2] = [
    ("spec", "815edad14a9f129e289eb855126ec8e774150735"),
    ("std-spec", "a2e4cb70b108745bdb58a554424b5500cf50933d"),
];

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives two levels below the repository root")
}

/// Run `git` in `cwd`; `None` when git is unavailable or the command fails.
fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// The gitlink commit the superproject's `HEAD` records for `path`.
fn head_pin(root: &Path, path: &str) -> Option<String> {
    let out = git(root, &["ls-tree", "HEAD", "--", path])?;
    // `160000 commit <sha>\t<path>`
    out.split_whitespace().nth(2).map(str::to_string)
}

/// The commit staged in the index (`git submodule status`), independent of
/// whether the submodule is initialized. A leading `-`/`+`/`U` marks a
/// not-initialized / drifted / conflicted entry.
fn index_pin(root: &Path, path: &str) -> Option<(char, String)> {
    let out = git(root, &["submodule", "status", "--", path])?;
    let line = out.lines().next()?;
    let mut chars = line.chars();
    let first = chars.next()?;
    if first.is_ascii_hexdigit() {
        Some((' ', line.split_whitespace().next()?.to_string()))
    } else {
        let sha = chars.as_str().split_whitespace().next()?.to_string();
        Some((first, sha))
    }
}

#[test]
fn submodules_match_the_pinned_commits() {
    let root = repo_root();
    if git(root, &["rev-parse", "--show-toplevel"]).is_none() {
        eprintln!(
            "skipping submodule pin check: `{}` is not a git checkout",
            root.display()
        );
        return;
    }

    let mut problems = Vec::new();
    for (path, pin) in PINS {
        match head_pin(root, path) {
            Some(head) if head == pin => {}
            Some(head) => problems.push(format!(
                "`{path}` is pinned to {head} in HEAD but this matrix and the \
                 readiness document describe {pin}:\n  update docs/stage1-readiness.md \
                 and the conformance baseline, then this test"
            )),
            None => problems.push(format!(
                "`{path}` is not a submodule of this repository's HEAD"
            )),
        }

        match index_pin(root, path) {
            // `-` : not initialized; the index still records the pin. `+`/`U`
            // (working tree differs / conflict) are real drift.
            Some((marker, index)) if matches!(marker, ' ' | '-') && index == pin => {}
            Some((marker, index)) => problems.push(format!(
                "`{path}` is at {index} in the index (status `{marker}`) but pinned \
                 to {pin}:\n  run `git submodule update --init {path}` (or commit the \
                 intended bump)"
            )),
            None => problems.push(format!("`{path}` is not listed by `git submodule status`")),
        }

        // An initialized submodule must be checked out at the pin; an absent
        // working tree is reported by the index check above.
        if root.join(path).join(".git").exists() {
            match git(&root.join(path), &["rev-parse", "HEAD"]) {
                Some(head) if head == pin => {}
                Some(head) => problems.push(format!(
                    "the `{path}` working tree is checked out at {head}, not the pinned \
                     {pin}:\n  run `git -C {path} checkout {pin}`"
                )),
                None => problems.push(format!("cannot read the `{path}` working tree HEAD")),
            }
        }
    }

    assert!(
        problems.is_empty(),
        "submodule pins do not match the committed state:\n\n{}",
        problems.join("\n\n")
    );
}
