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
//! removed, so the list cannot silently rot. Windows adds
//! `tests/aot_differential_baseline.windows.tsv` for platform-only gaps; it is
//! ignored on Unix.

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

fn read_baseline(path: &Path, entries: &mut BTreeMap<String, String>) {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
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
}

/// The examples that cannot be compared on this host. The shared baseline is
/// loaded everywhere; `aot_differential_baseline.windows.tsv` adds the platform
/// gaps and is ignored on Unix, where those examples are compared normally.
fn baseline() -> BTreeMap<String, String> {
    let mut entries = BTreeMap::new();
    read_baseline(
        &manifest_dir().join("tests/aot_differential_baseline.tsv"),
        &mut entries,
    );
    if cfg!(target_os = "windows") {
        read_baseline(
            &manifest_dir().join("tests/aot_differential_baseline.windows.tsv"),
            &mut entries,
        );
    }
    entries
}

/// Every platform baseline names an example that exists, even when the current
/// host does not apply it. A rename cannot silently leave a dead entry behind.
fn assert_platform_baseline_names_exist(root: &Path) {
    let path = manifest_dir().join("tests/aot_differential_baseline.windows.tsv");
    let mut entries = BTreeMap::new();
    read_baseline(&path, &mut entries);
    for name in entries.keys() {
        assert!(
            root.join("examples").join(name).is_file(),
            "{} names `{name}`, which is not an example",
            path.display()
        );
    }
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
    assert_platform_baseline_names_exist(&root);
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
            "{name} now matches; remove it from the active AOT differential baseline"
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

/// WP6/B8a + Gate 4: stage 0's `xupm` builds stage 1 into a native binary, and
/// that binary matches the interpreter on the bootstrap workload. Like the
/// example differential, this needs `target/release/libxulo_runtime.a` and is
/// enabled by `XULO_AOT_DIFFERENTIAL=1`.
#[test]
fn stage1_binary_matches_the_interpreter() {
    if std::env::var_os("XULO_AOT_DIFFERENTIAL").is_none() {
        eprintln!(
            "skipping stage 1 native differential (set XULO_AOT_DIFFERENTIAL=1 after \
             building target/release/libxulo_runtime.a)"
        );
        return;
    }

    let root = workspace_root();
    let build = run(&["build", "xuloc/main.xulo", "--release"], &root);
    assert!(
        build.status.success(),
        "`xupm build xuloc/main.xulo --release` failed:\n{}",
        render(&build.stderr)
    );
    let artifact = artifact_path(&build);

    let workloads = [
        ("tokens", "examples/fibonacci.xulo"),
        ("parse", "examples/fibonacci.xulo"),
        ("parse", "xuloc/main.xulo"),
        ("check", "xuloc/corpus/accept/basics.xulo"),
    ];
    for (mode, file) in workloads {
        let interpreted = Command::new(BIN)
            .arg("run")
            .arg("xuloc/main.xulo")
            .current_dir(&root)
            .env("XULO_STAGE1_MODE", mode)
            .env("XULO_STAGE1_FILE", root.join(file))
            .output()
            .unwrap_or_else(|e| panic!("cannot run {BIN}: {e}"));
        let native = Command::new(&artifact)
            .arg(mode)
            .arg(root.join(file))
            .current_dir(&root)
            .output()
            .unwrap_or_else(|e| panic!("cannot run {}: {e}", artifact.display()));

        assert!(
            interpreted.status.success(),
            "interpreter failed for {mode} {file}:\n{}",
            render(&interpreted.stderr)
        );
        assert!(
            native.status.success(),
            "native stage 1 failed for {mode} {file}:\n{}",
            render(&native.stderr)
        );
        assert_eq!(
            render(&interpreted.stdout),
            render(&native.stdout),
            "native stage 1 differs from the interpreter for {mode} {file}"
        );
        assert_eq!(
            render(&interpreted.stderr),
            render(&native.stderr),
            "native stage 1 stderr differs for {mode} {file}"
        );
    }
}
