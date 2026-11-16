//! T4: build each top-level example with `xupm build` and compare the native
//! binary's stdout/stderr against `xupm run` (the interpreter).
//!
//! The runner is opt-in because it links real executables and needs
//! `target/release/libxulo_runtime.a`. CI enables it after the release build:
//!
//! ```text
//! XULO_AOT_DIFFERENTIAL=1 cargo test -p xupm --test aot_differential
//! ```
//!
//! Examples that still lack AOT lowering are listed in
//! `tests/aot_differential_baseline.tsv`. A baseline entry must keep failing
//! to build; once it starts building, the runner fails until the entry is
//! removed, so the list cannot silently rot.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_xupm");

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_root() -> PathBuf {
    manifest_dir().join("../..")
}

fn baseline() -> BTreeMap<String, String> {
    let path = manifest_dir().join("tests/aot_differential_baseline.tsv");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut entries = BTreeMap::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((name, reason)) = line.split_once('\t') else {
            panic!(
                "{}:{}: expected `example.xulo<TAB>reason`",
                path.display(),
                index + 1
            );
        };
        entries.insert(name.trim().to_string(), reason.trim().to_string());
    }
    entries
}

fn run(args: &[&str], cwd: &Path) -> Output {
    Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("cannot run {BIN}: {e}"))
}

fn artifact_path(build: &Output) -> PathBuf {
    let stdout = String::from_utf8_lossy(&build.stdout);
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix("build successful: "))
        .unwrap_or_else(|| panic!("build did not report an artifact:\n{stdout}"));
    let path = line
        .split_once(" (backend:")
        .map(|(path, _)| path)
        .unwrap_or(line);
    PathBuf::from(path)
}

fn render(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn assert_same(example: &Path, kind: &str, interpreted: &[u8], native: &[u8]) {
    assert_eq!(
        render(interpreted),
        render(native),
        "{} {kind} differs between `xupm run` and the AOT binary",
        example.display()
    );
}

#[test]
fn aot_matches_interpreter_on_examples() {
    if std::env::var_os("XULO_AOT_DIFFERENTIAL").is_none() {
        eprintln!(
            "skipping AOT differential examples (set XULO_AOT_DIFFERENTIAL=1 after \
             building target/release/libxulo_runtime.a)"
        );
        return;
    }

    let root = workspace_root();
    let examples_dir = root.join("examples");
    let mut examples: Vec<PathBuf> = std::fs::read_dir(&examples_dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", examples_dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "xulo"))
        .collect();
    examples.sort();

    let mut expected_failures = baseline();
    let mut compared = 0;

    for example in &examples {
        let name = example
            .file_name()
            .and_then(|name| name.to_str())
            .expect("example file name");
        let known_failure = expected_failures.remove(name);

        let interpreted = run(&["run", example.to_str().unwrap()], &root);
        if !interpreted.status.success() {
            if let Some(reason) = known_failure {
                eprintln!("known gap: {name}: {reason}");
                continue;
            }
            panic!(
                "`xupm run {}` failed:\n{}",
                example.display(),
                render(&interpreted.stderr)
            );
        }

        let build = run(&["build", example.to_str().unwrap()], &root);
        if !build.status.success() {
            if let Some(reason) = known_failure {
                eprintln!("known AOT gap: {name}: {reason}");
                continue;
            }
            panic!(
                "`xupm build {}` failed:\n{}",
                example.display(),
                render(&build.stderr)
            );
        }
        let artifact = artifact_path(&build);
        let native = Command::new(&artifact)
            .current_dir(&root)
            .output()
            .unwrap_or_else(|e| panic!("cannot run {}: {e}", artifact.display()));
        assert!(
            native.status.success(),
            "{} exited with {}:\n{}",
            artifact.display(),
            native.status,
            render(&native.stderr)
        );

        if interpreted.stdout != native.stdout || interpreted.stderr != native.stderr {
            if let Some(reason) = &known_failure {
                eprintln!("known output mismatch: {name}: {reason}");
                continue;
            }
            assert_same(example, "stdout", &interpreted.stdout, &native.stdout);
            assert_same(example, "stderr", &interpreted.stderr, &native.stderr);
        }
        assert!(
            known_failure.is_none(),
            "{name} now matches; remove it from tests/aot_differential_baseline.tsv"
        );
        compared += 1;
    }

    assert!(
        expected_failures.is_empty(),
        "baseline names with no matching example: {:?}",
        expected_failures.keys().collect::<Vec<_>>()
    );
    assert!(
        compared > 0,
        "no examples were compared; examples dir: {}",
        examples_dir.display()
    );
}
