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

/// Strip ANSI SGR sequences so the diagnostic can be recognized in captured
/// output (the CLI colors its header and source snippet).
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// The runtime message both paths carry for a failure, independent of their
/// rendering: `error[E0006] runtime: …` for the interpreter and `panic: …` for
/// the native binary.
fn failure_message(stderr: &str) -> Option<String> {
    let plain = strip_ansi(stderr);
    for line in plain.lines() {
        let line = line.trim();
        let Some(message) = line
            .split_once("runtime: ")
            .map(|(_, rest)| rest)
            .or_else(|| line.strip_prefix("panic: "))
        else {
            continue;
        };
        let message = message.trim();
        // The interpreter prefixes operator failures with the operator
        // (`` `/`: division by zero``) and panics with `uncaught exception:`.
        let message = message
            .strip_prefix("uncaught exception: ")
            .unwrap_or(message);
        let message = message
            .split_once("`: ")
            .filter(|(head, _)| head.starts_with('`'))
            .map(|(_, rest)| rest)
            .unwrap_or(message);
        return Some(message.trim().to_string());
    }
    None
}

#[test]
fn failure_shape_normalizes_both_renderings() {
    let interpreter = "\u{1b}[1merror\u{1b}[0m[\u{1b}[36mE0006\u{1b}[0m] runtime: `/`: division by zero\n --> x.xulo:1:1\n";
    let native = "panic: division by zero\n";
    assert_eq!(
        failure_message(interpreter).as_deref(),
        Some("division by zero")
    );
    assert_eq!(failure_message(native).as_deref(), Some("division by zero"));
    let caught = "error[E0006] runtime: uncaught exception: cannot continue";
    assert_eq!(failure_message(caught).as_deref(), Some("cannot continue"));
    assert_eq!(failure_message("no failure here"), None);
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
        if let Some(reason) = unit.directives.failure_shape_here() {
            // T13 option (c): both paths keep their own failure format, but the
            // differential compares the normalized failure shape — same failing
            // exit status and the same runtime message. stdout is not compared:
            // the interpreter drops buffered output, the native binary streams it.
            let interpreted = run(&["run", path], &root);
            assert!(
                !interpreted.status.success(),
                "{name}: `xupm run` was expected to fail"
            );
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
            assert!(
                !native.status.success(),
                "{name}: the AOT binary was expected to fail"
            );

            let interpreted_msg = failure_message(&render(&interpreted.stderr))
                .unwrap_or_else(|| panic!("{name}: no runtime message in interpreter stderr"));
            let native_msg = failure_message(&render(&native.stderr))
                .unwrap_or_else(|| panic!("{name}: no panic message in native stderr"));
            assert_eq!(
                interpreted_msg, native_msg,
                "{name}: normalized failure messages differ"
            );
            if let Some(expected) = &unit.directives.panic {
                assert!(
                    interpreted_msg.contains(expected),
                    "{name}: failure message `{interpreted_msg}` does not contain `{expected}`"
                );
            }
            eprintln!("normalized failure shape: {name}: {reason}");
            compared += 1;
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
