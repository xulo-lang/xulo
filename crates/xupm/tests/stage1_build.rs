//! WP6: stage 1 code generation, corpus builds, the self-host loop, and the
//! reproducible-build smoke test.
//!
//! The runner is opt-in because it needs a C compiler and stage 0's AOT
//! linker/runtime library:
//!
//! ```text
//! XULO_STAGE1_BUILD=1 cargo test -p xupm --test stage1_build -- --nocapture
//! ```
//!
//! It builds stage 1 with `xupm` (bootstrap step 0 -> 1), then:
//!
//! * compiles and runs the accept corpus through stage 1's C backend and
//!   compares stdout with the checked-in `.expected` files (WP6 corpus);
//! * uses stage 1 to build `xuloc/main.xulo` into a stage 2 binary (self-host);
//! * compares stage 1 and stage 2 on the bootstrap suite: `tokens`/`parse` of
//!   every stage 1 source, `check` of the accept/reject corpus, and the
//!   corpus programs' stdout (T5c);
//! * builds the compiler twice and compares the generated C byte for byte, and
//!   checks that stage 2 regenerates exactly the C stage 1 produced (B8b).
//!
//! The C compiler is `XULO_CC` when set, otherwise stage 1 picks the host
//! default (`clang`, or the standard LLVM install on Windows).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

const XUPM: &str = env!("CARGO_BIN_EXE_xupm");

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn repo_root() -> PathBuf {
    manifest_dir().join("../..")
}

fn enabled() -> bool {
    if std::env::var_os("XULO_STAGE1_BUILD").is_some() {
        return true;
    }
    if std::env::var_os("XULO_STAGE1_REQUIRED").is_some() {
        panic!(
            "XULO_STAGE1_REQUIRED is set but XULO_STAGE1_BUILD is not; the stage 1 gate must not skip"
        );
    }
    eprintln!(
        "skipping stage 1 build tests (set XULO_STAGE1_BUILD=1 after \
         `cargo build -p xulo-runtime`)"
    );
    false
}

fn runtime_lib(root: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os("XULO_RUNTIME_LIB") {
        return PathBuf::from(path);
    }
    let name = if cfg!(windows) {
        "xulo_runtime.lib"
    } else {
        "libxulo_runtime.a"
    };
    // Stage 1 is built with `--release` so its own self-compile runs at a
    // usable speed; the runtime library must match that optimization profile.
    let path = root.join("target").join("release").join(name);
    if !path.is_file() {
        // `cargo test` builds the crate's rlib but can prune the staticlib
        // artifact; produce it explicitly so the AOT link has a runtime.
        let status = Command::new(env!("CARGO"))
            .args(["build", "-p", "xulo-runtime", "--release"])
            .current_dir(root)
            .status()
            .expect("cargo build -p xulo-runtime --release should start");
        assert!(
            status.success(),
            "cargo build -p xulo-runtime --release failed with {status}"
        );
    }
    path
}

fn exe_suffix() -> &'static str {
    if cfg!(windows) { ".exe" } else { "" }
}

fn unique_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "xulo_stage1_{tag}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("temporary directory should be creatable");
    dir
}

fn render(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Windows text-mode stdout turns `\n` into `\r\n`; goldens are stored with
/// plain `\n` so the comparison does not depend on the host.
fn render_lines(bytes: &[u8]) -> String {
    render(bytes).replace("\r\n", "\n")
}

fn assert_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed ({}):\nstdout:\n{}\nstderr:\n{}",
        output.status,
        render(&output.stdout),
        render(&output.stderr)
    );
}

/// The bootstrap step: stage 0's `xupm build` produces the native stage 1
/// binary. Built once per test process because each test drives the same path.
fn stage1_binary() -> PathBuf {
    static STAGE1: OnceLock<PathBuf> = OnceLock::new();
    STAGE1
        .get_or_init(|| {
            let root = repo_root();
            let lib = runtime_lib(&root);
            assert!(
                lib.is_file(),
                "the AOT runtime library is still missing at {}; run `cargo build -p xulo-runtime` \
                 or set XULO_RUNTIME_LIB",
                lib.display()
            );
            let output = Command::new(XUPM)
                .args(["build", "xuloc/main.xulo", "--release"])
                .env("XULO_RUNTIME_LIB", &lib)
                .current_dir(&root)
                .output()
                .expect("xupm build should start");
            assert_success(&output, "xupm build xuloc/main.xulo");
            let stdout = render(&output.stdout);
            let line = stdout
                .lines()
                .find_map(|line| line.strip_prefix("build successful: "))
                .unwrap_or_else(|| panic!("xupm did not report the stage 1 binary:\n{stdout}"));
            let path = line
                .split_once(" (backend:")
                .map(|(path, _)| path)
                .unwrap_or(line);
            let path = PathBuf::from(path);
            assert!(
                path.is_file(),
                "stage 1 binary missing at {}",
                path.display()
            );
            path
        })
        .clone()
}

fn run_stage1(binary: &Path, root: &Path, mode: &str, file: &Path) -> Output {
    Command::new(binary)
        .arg(mode)
        .arg(file)
        .current_dir(root)
        .env_remove("XULO_STAGE1_MODE")
        .env_remove("XULO_STAGE1_FILE")
        .output()
        .unwrap_or_else(|error| panic!("cannot run {} {mode}: {error}", binary.display()))
}

fn build_stage1_program(
    binary: &Path,
    root: &Path,
    file: &Path,
    out_dir: &Path,
    stem: &str,
) -> PathBuf {
    let program = out_dir.join(format!("{stem}{}", exe_suffix()));
    let source = out_dir.join(format!("{stem}.c"));
    let output = Command::new(binary)
        .arg("build")
        .arg(file)
        .current_dir(root)
        .env("XULO_STAGE1_OUT", &program)
        .env("XULO_STAGE1_C", &source)
        .env_remove("XULO_STAGE1_MODE")
        .env_remove("XULO_STAGE1_FILE")
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "cannot run {} build {}: {error}",
                binary.display(),
                file.display()
            )
        });
    assert_success(&output, &format!("stage 1 build of {}", file.display()));
    assert!(
        program.is_file(),
        "stage 1 reported success but wrote no binary at {}",
        program.display()
    );
    assert!(
        source.is_file(),
        "stage 1 reported success but wrote no C at {}",
        source.display()
    );
    program
}

#[derive(Clone)]
struct Stage2 {
    binary: PathBuf,
    source: PathBuf,
}

/// Stage 1 compiled by itself. Shared by the tests that compare the two
/// compilers so a single self-build serves the whole run.
fn stage2_binary() -> Stage2 {
    static STAGE2: OnceLock<Stage2> = OnceLock::new();
    STAGE2
        .get_or_init(|| {
            let root = repo_root();
            let stage1 = stage1_binary();
            let work = unique_dir("stage2");
            let binary = build_stage1_program(
                &stage1,
                &root,
                Path::new("xuloc/main.xulo"),
                &work,
                "stage2",
            );
            Stage2 {
                binary,
                source: work.join("stage2.c"),
            }
        })
        .clone()
}

fn accept_corpus(root: &Path) -> Vec<PathBuf> {
    let dir = root.join("xuloc/corpus/accept");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "xulo"))
        .collect();
    files.sort();
    files
}

fn reject_corpus(root: &Path) -> Vec<PathBuf> {
    let dir = root.join("xuloc/corpus/reject");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| entry.expect("corpus entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "xulo"))
        .collect();
    files.sort();
    files
}

fn stage1_sources(root: &Path) -> Vec<PathBuf> {
    let dir = root.join("xuloc");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", dir.display()))
        .map(|entry| entry.expect("source entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "xulo"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 6,
        "expected at least the six stage 1 sources, found {files:?}"
    );
    files
}

fn run_program(program: &Path) -> Output {
    Command::new(program)
        .output()
        .unwrap_or_else(|error| panic!("cannot run {}: {error}", program.display()))
}

fn expected_path(file: &Path) -> PathBuf {
    file.with_extension("expected")
}

/// WP6: stage 1 checks the fixed corpus, compiles the accept files, and the
/// resulting programs print the checked-in output.
#[test]
fn stage1_compiles_the_bootstrap_corpus() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("corpus");

    for file in accept_corpus(&root) {
        let name = file.file_stem().unwrap().to_string_lossy().into_owned();
        let relative = file.strip_prefix(&root).unwrap().to_path_buf();

        let checked = run_stage1(&stage1, &root, "check", &relative);
        assert_success(
            &checked,
            &format!("stage 1 check of {}", relative.display()),
        );

        let program = build_stage1_program(&stage1, &root, &relative, &work, &name);
        let run = run_program(&program);
        assert_success(&run, &format!("the stage 1 build of {name}"));
        let expected = std::fs::read_to_string(expected_path(&file))
            .unwrap_or_else(|error| panic!("cannot read the golden output for {name}: {error}"));
        assert_eq!(
            render_lines(&run.stdout),
            render_lines(expected.as_bytes()),
            "the stage 1 build of {name} printed unexpected output"
        );
    }

    for file in reject_corpus(&root) {
        let relative = file.strip_prefix(&root).unwrap().to_path_buf();
        let checked = run_stage1(&stage1, &root, "check", &relative);
        assert!(
            !checked.status.success(),
            "stage 1 accepted rejected corpus file {}:\nstdout:\n{}",
            relative.display(),
            render(&checked.stdout)
        );
        let stderr = render(&checked.stderr);
        assert!(
            stderr.contains(": error[") && stderr.contains("] semantic:"),
            "stage 1 should report a coded diagnostic for {}:\n{stderr}",
            relative.display()
        );
    }
}

/// WP6: stage 1 compiles itself, producing a native stage 2 binary.
#[test]
fn stage1_builds_itself_into_stage2() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage2 = stage2_binary();
    // The self-built compiler must at least run its own CLI. A deeper check is
    // the T5c differential below.
    let parsed = run_stage1(&stage2.binary, &root, "parse", Path::new("xuloc/ast.xulo"));
    assert_success(&parsed, "stage 2 parse of xuloc/ast.xulo");
    assert!(
        render(&parsed.stdout).starts_with("Program @"),
        "stage 2 produced no tree:\n{}",
        render(&parsed.stdout)
    );
}

/// T5c: stage 1 and stage 2 agree on the bootstrap suite.
#[test]
fn stage1_and_stage2_agree_on_the_bootstrap_suite() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let stage2 = stage2_binary();
    let work = unique_dir("t5c");

    for file in stage1_sources(&root) {
        let relative = file.strip_prefix(&root).unwrap().to_path_buf();
        let tokens1 = run_stage1(&stage1, &root, "tokens", &relative);
        let tokens2 = run_stage1(&stage2.binary, &root, "tokens", &relative);
        assert!(tokens1.status.success() && tokens2.status.success());
        assert_eq!(
            render_lines(&tokens1.stdout),
            render_lines(&tokens2.stdout),
            "stage 1 and stage 2 token streams differ for {}",
            relative.display()
        );

        let parse1 = run_stage1(&stage1, &root, "parse", &relative);
        let parse2 = run_stage1(&stage2.binary, &root, "parse", &relative);
        assert!(parse1.status.success() && parse2.status.success());
        assert_eq!(
            render_lines(&parse1.stdout),
            render_lines(&parse2.stdout),
            "stage 1 and stage 2 parse trees differ for {}",
            relative.display()
        );
    }

    for file in accept_corpus(&root).into_iter().chain(reject_corpus(&root)) {
        let relative = file.strip_prefix(&root).unwrap().to_path_buf();
        let check1 = run_stage1(&stage1, &root, "check", &relative);
        let check2 = run_stage1(&stage2.binary, &root, "check", &relative);
        assert_eq!(
            check1.status.success(),
            check2.status.success(),
            "stage 1 and stage 2 disagree on the verdict for {}",
            relative.display()
        );
        assert_eq!(
            render_lines(&check1.stdout),
            render_lines(&check2.stdout),
            "stage 1 and stage 2 check output differs for {}",
            relative.display()
        );
        assert_eq!(
            render_lines(&check1.stderr),
            render_lines(&check2.stderr),
            "stage 1 and stage 2 diagnostics differ for {}",
            relative.display()
        );
    }

    let work2 = unique_dir("t5c_stage2");
    for file in accept_corpus(&root) {
        let relative = file.strip_prefix(&root).unwrap().to_path_buf();
        let name = file.file_stem().unwrap().to_string_lossy().into_owned();
        let built1 = build_stage1_program(&stage1, &root, &relative, &work, &format!("{name}_1"));
        let built2 = build_stage1_program(
            &stage2.binary,
            &root,
            &relative,
            &work2,
            &format!("{name}_2"),
        );
        let run1 = run_program(&built1);
        let run2 = run_program(&built2);
        assert_success(&run1, &format!("the stage 1 build of {name}"));
        assert_success(&run2, &format!("the stage 2 build of {name}"));
        assert_eq!(
            render(&run1.stdout),
            render(&run2.stdout),
            "stage 1 and stage 2 builds of {name} printed different output"
        );
    }
}

/// B8b: the C backend is deterministic, and stage 2 regenerates exactly the
/// translation unit that produced it.
#[test]
fn stage1_build_is_reproducible() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work_a = unique_dir("repro_a");
    let work_b = unique_dir("repro_b");

    let corpus = root.join("xuloc/corpus/accept/basics.xulo");
    let relative = corpus.strip_prefix(&root).unwrap().to_path_buf();
    let program_a = build_stage1_program(&stage1, &root, &relative, &work_a, "basics");
    let program_b = build_stage1_program(&stage1, &root, &relative, &work_b, "basics");
    let source_a = std::fs::read(work_a.join("basics.c")).expect("generated C");
    let source_b = std::fs::read(work_b.join("basics.c")).expect("generated C");
    assert_eq!(
        source_a, source_b,
        "two stage 1 builds of basics.xulo produced different C"
    );
    assert_eq!(
        render(&run_program(&program_a).stdout),
        render(&run_program(&program_b).stdout),
        "two stage 1 builds of basics.xulo behaved differently"
    );

    let stage2 = stage2_binary();
    let stage3 = build_stage1_program(
        &stage2.binary,
        &root,
        Path::new("xuloc/main.xulo"),
        &work_b,
        "stage3",
    );
    let stage2_c = std::fs::read(&stage2.source).expect("stage 1 translation unit");
    let stage3_c = std::fs::read(work_b.join("stage3.c")).expect("stage 2 translation unit");
    assert_eq!(
        stage2_c, stage3_c,
        "stage 2 regenerated different C for the compiler source"
    );
    assert!(stage3.is_file(), "the stage 3 binary should exist");
}
