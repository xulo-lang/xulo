//! T4: build every example program with `xupm build` and compare the native
//! binary's stdout/stderr against `xupm run` (the interpreter).
//!
//! The runner is opt-in because it links real executables and needs
//! `target/release/libxulo_runtime.a`. CI enables it after the release build:
//!
//! ```text
//! XULO_AOT_DIFFERENTIAL=1 cargo test -p xupm --test aot_differential
//! ```
//!
//! Discovery and the header directives come from `xulo-examples`, the same
//! support library the example gate under `crates/xulo-cli/tests/examples.rs`
//! uses. An example that still lacks AOT lowering carries
//! `// @no-aot <platform>: <reason>`. That marker must keep failing: either the
//! native build fails or the two paths still disagree. Once they agree, this
//! test fails until the marker is removed, so a recorded gap cannot silently
//! rot. `@skip-aot` excludes an example that is not meant to be built at all
//! (the `#[test]` harness examples).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_xupm");

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn workspace_root() -> PathBuf {
    manifest_dir().join("../..")
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
    let corpus = xulo_examples::load(&root);
    let mut compared = 0;

    for unit in &corpus.units {
        let name = &unit.rel;
        let example = &unit.entry;
        let path = example.to_str().expect("an example path is UTF-8");

        if let Some(reason) = unit.directives.skip_aot_here() {
            eprintln!("skipped: {name}: {reason}");
            continue;
        }
        if let Some(reason) = unit.directives.no_aot_here() {
            // A recorded gap must stay a gap: either the native build fails or
            // the outputs still differ. Once both paths agree, the marker is
            // stale and the run fails.
            let build = run(&["build", path], &root);
            let mut matches = false;
            if build.status.success() {
                let artifact = artifact_path(&build);
                let interpreted = run(&["run", path], &root);
                let native = Command::new(&artifact)
                    .current_dir(&root)
                    .output()
                    .unwrap_or_else(|e| panic!("cannot run {}: {e}", artifact.display()));
                matches = interpreted.status.success() == native.status.success()
                    && interpreted.stdout == native.stdout
                    && interpreted.stderr == native.stderr;
            }
            assert!(
                !matches,
                "{name} now matches natively; remove its `@no-aot` marker ({reason})"
            );
            eprintln!("known AOT gap: {name}: {reason}");
            continue;
        }
        if let Some(code) = &unit.directives.error {
            eprintln!("skipped: {name}: expects static error {code}");
            continue;
        }
        if let Some(reason) = unit.directives.no_run_here() {
            eprintln!("skipped: {name}: not runnable here: {reason}");
            continue;
        }

        let interpreted = run(&["run", path], &root);
        let expects_failure = unit.directives.panic.is_some();
        if !expects_failure && !interpreted.status.success() {
            panic!(
                "`xupm run {}` failed:\n{}",
                example.display(),
                render(&interpreted.stderr)
            );
        }

        let build = run(&["build", path], &root);
        assert!(
            build.status.success(),
            "`xupm build {}` failed:\n{}",
            example.display(),
            render(&build.stderr)
        );
        let artifact = artifact_path(&build);
        let native = Command::new(&artifact)
            .current_dir(&root)
            .output()
            .unwrap_or_else(|e| panic!("cannot run {}: {e}", artifact.display()));

        assert_eq!(
            interpreted.status.success(),
            native.status.success(),
            "{name}: `xupm run` and the AOT binary disagree on success"
        );
        assert_same(example, "stdout", &interpreted.stdout, &native.stdout);
        assert_same(example, "stderr", &interpreted.stderr, &native.stderr);
        compared += 1;
    }

    assert!(
        compared > 0,
        "no examples were compared; examples dir: {}",
        corpus.dir.display()
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
        ("tokens", "examples/functions/fibonacci.xulo"),
        ("parse", "examples/functions/fibonacci.xulo"),
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
