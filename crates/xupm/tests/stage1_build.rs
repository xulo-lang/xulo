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
//! * uses stage 1 to build `xuloc/src/main.xulo` into a stage 2 binary (self-host);
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
                .args(["build", "xuloc/src/main.xulo", "--release"])
                .env("XULO_RUNTIME_LIB", &lib)
                .current_dir(&root)
                .output()
                .expect("xupm build should start");
            assert_success(&output, "xupm build xuloc/src/main.xulo");
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
                Path::new("xuloc/src/main.xulo"),
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
    let dir = root.join("xuloc/src");
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

/// Examples the C backend lowers end to end: the `std` surface stage 1
/// implements, plus templates, optional chaining, object literals, `?`
/// propagation, chained assignment, Unicode escapes, `Array::filled`,
/// closures (including mutable captures and returned closures), `defer`
/// (including loops and `?`), `std::fmt` (including the `Display`/`Debug`
/// consultation), const generic parameters, the synchronous
/// `spawn`/`lock`/service model, `std::regex`, `std::net`, `std::process`,
/// `std::http`, named function values, `extern "C"`, and module namespaces
/// (file-backed and inline modules, nested modules, `use` aliases/groups/globs,
/// and `pub` re-exports).
#[test]
fn stage1_runs_the_lowered_examples() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("std_examples");
    for rel in [
        "examples/std/path.xulo",
        "examples/std/hint.xulo",
        "examples/std/math_extra.xulo",
        "examples/std/math_time.xulo",
        "examples/std/time.xulo",
        "examples/std/std.xulo",
        "examples/std/os.xulo",
        "examples/std/json.xulo",
        "examples/std/fs.xulo",
        "examples/std/fs_extra.xulo",
        "examples/std/io.xulo",
        "examples/std/io_extra.xulo",
        "examples/std/fmt.xulo",
        "examples/std/regex.xulo",
        "examples/std/net.xulo",
        "examples/std/net_shutdown.xulo",
        "examples/std/process.xulo",
        "examples/std/http.xulo",
        "examples/concurrency/spawn_lock.xulo",
        "examples/ffi/ffi.xulo",
        "examples/basics/templates.xulo",
        "examples/basics/bindings.xulo",
        "examples/basics/lexical.xulo",
        "examples/functions/closures_hof.xulo",
        "examples/functions/defer.xulo",
        "examples/functions/params.xulo",
        "examples/errors/propagation.xulo",
        "examples/types/optional_chaining.xulo",
        "examples/types/structs.xulo",
        "examples/types/aliases.xulo",
        "examples/types/generics_impl.xulo",
        "examples/collections/collections.xulo",
        "examples/collections/arrays.xulo",
        "examples/collections/maps.xulo",
        "examples/collections/sets.xulo",
        "examples/patterns/destructuring.xulo",
        "examples/patterns/match_patterns.xulo",
        "examples/modules/main.xulo",
        "examples/modules/imports/main.xulo",
        "examples/modules/visibility/main.xulo",
        "examples/modules/inline/main.xulo",
    ] {
        // `std::fs`'s directory walkers are POSIX-only in the bootstrap runtime.
        if !cfg!(unix) && rel.starts_with("examples/std/fs") {
            continue;
        }
        let file = Path::new(rel);
        let name = file
            .file_stem()
            .expect("the example path has a stem")
            .to_string_lossy()
            .into_owned();
        let program = build_stage1_program(&stage1, &root, file, &work, &name);
        // `process.xulo` drives `sh`/`cat`/`sleep`; it is build-only on Windows.
        if !cfg!(unix) && rel == "examples/std/process.xulo" {
            continue;
        }
        let run = run_program(&program);
        assert_success(&run, &format!("running {rel}"));
        let expected = std::fs::read_to_string(expected_path(&root.join(file)))
            .unwrap_or_else(|error| panic!("cannot read the golden output for {rel}: {error}"));
        assert_eq!(
            render_lines(&run.stdout),
            render_lines(expected.as_bytes()),
            "the stage 1 build of {rel} printed unexpected output"
        );
        if let Some(problem) = xulo_examples::mismatch(&root.join(file), true, &run.stderr) {
            panic!("the stage 1 build of {rel} printed unexpected stderr:\n{problem}");
        }
    }
}

/// A root child importing a declaration from the package root through
/// `super::`. The root module id is the empty string, so this pins that module
/// resolution does not confuse it with the "not found" sentinel.
#[test]
fn stage1_resolves_super_imports_from_a_root_child() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("root_super");
    let file = work.join("root_super.xulo");
    std::fs::write(
        &file,
        r#"mod child {
    use super::helper;

    pub fn call(): int {
        helper()
    }
}

pub fn helper(): int {
    7
}

fn main() {
    print(child::call())
}
"#,
    )
    .expect("the root-super probe should be writable");

    let checked = run_stage1(&stage1, &root, "check", &file);
    assert_success(&checked, "stage 1 check of a root-child super import");
    let program = build_stage1_program(&stage1, &root, &file, &work, "root_super");
    let run = run_program(&program);
    assert_success(&run, "running the root-child super probe");
    assert_eq!(render_lines(&run.stdout), "7\n");
}

/// D-02 (closed): the C runtime renders floats exactly like stage 0, including
/// magnitudes that `%g` would spell in exponent form and the signed-zero case.
#[test]
fn stage1_c_backend_matches_stage0_float_formatting() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("float_format");
    let source = work.join("floats.xulo");
    std::fs::write(
        &source,
        r#"fn main() {
    print(0.1 + 0.2)
    print(1.0 / 3.0)
    print(3.141592653589793)
    print(std::math::pow(10.0, 300.0))
    print(std::math::pow(10.0, -300.0))
    print(std::math::pow(10.0, 21.0))
    print(-0.0)
    print(9223372036854775807)
    print(-9223372036854775808)
    print(-9223372036854775807 - 1)
}
"#,
    )
    .expect("the float probe should be writable");

    let interpreted = Command::new(XUPM)
        .args(["run", source.to_str().expect("a UTF-8 path")])
        .current_dir(&root)
        .output()
        .expect("xupm run should start");
    assert_success(&interpreted, "the interpreter float probe");

    let program = build_stage1_program(&stage1, &root, &source, &work, "floats");
    let native = run_program(&program);
    assert_success(&native, "the stage 1 float probe");
    assert_eq!(
        render_lines(&native.stdout),
        render_lines(&interpreted.stdout),
        "the C backend's float text must match the interpreter"
    );
}

/// Closures: nested captures, a mutable capture shared by two closures, and a
/// parameter that shadows a captured name. Captured locals are boxed cells, so
/// a closure returned from its maker keeps the same cell.
#[test]
fn stage1_c_backend_lowers_closures() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("closures");
    let source = work.join("closures.xulo");
    std::fs::write(
        &source,
        r#"fn add3(a: number): fn(number): fn(number): number {
    fn(b: number): fn(number): number {
        fn(c: number): number { a + b + c }
    }
}

fn make_pair() {
    let mut n = 0
    let inc = fn(): number {
        n = n + 1
        n
    }
    let read = fn(): number { n }
    return [inc, read]
}

fn main() {
    let g = add3(1)(2)
    print(g(3))
    let pair = make_pair()
    print(pair[0]())
    print(pair[1]())
    print(pair[0]())
    let y = 10
    let f = fn(y: number): number { y + 1 }
    print(f(4))
    print(y)
}
"#,
    )
    .expect("the closure probe should be writable");

    let interpreted = Command::new(XUPM)
        .args(["run", source.to_str().expect("a UTF-8 path")])
        .current_dir(&root)
        .output()
        .expect("xupm run should start");
    assert_success(&interpreted, "the interpreter closure probe");

    let program = build_stage1_program(&stage1, &root, &source, &work, "closures");
    let native = run_program(&program);
    assert_success(&native, "the stage 1 closure probe");
    assert_eq!(
        render_lines(&native.stdout),
        render_lines(&interpreted.stdout),
        "the C backend's closure output must match the interpreter"
    );
}

/// Const generic parameters: a const argument is appended to the call, bound
/// as a function-local value, and can flow into `Array::filled`. Mixed type and
/// const parameters keep their declaration positions.
#[test]
fn stage1_c_backend_lowers_const_generics() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("const_generics");
    let source = work.join("const_generics.xulo");
    std::fs::write(
        &source,
        r#"fn pick<const N: int>(x: int): int {
    x + N
}

fn both<T, const N: int>(x: T): int {
    N
}

fn make<const N: int>(v: int): Array<int, N> {
    Array::filled<int, N>(v)
}

fn main() {
    print(pick<2>(10))
    print(both<string, 4>("x"))
    let a = make<3>(7)
    print(a.len, a[0])
}
"#,
    )
    .expect("the const-generic probe should be writable");

    let interpreted = Command::new(XUPM)
        .args(["run", source.to_str().expect("a UTF-8 path")])
        .current_dir(&root)
        .output()
        .expect("xupm run should start");
    assert_success(&interpreted, "the interpreter const-generic probe");

    let program = build_stage1_program(&stage1, &root, &source, &work, "const_generics");
    let native = run_program(&program);
    assert_success(&native, "the stage 1 const-generic probe");
    assert_eq!(
        render_lines(&native.stdout),
        render_lines(&interpreted.stdout),
        "the C backend's const-generic output must match the interpreter"
    );
}

/// `std::regex`: compilation errors, `test`/`find`/`find_all`, empty-match
/// iteration, flags (`i`/`s`/`m`), greedy/lazy and counted repetition, named
/// groups, templates, word boundaries, and Unicode scalar indices.
#[test]
fn stage1_c_backend_lowers_regex() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("regex");
    let source = work.join("regex.xulo");
    std::fs::write(
        &source,
        r#"use std::regex::RegExp;

fn show_find(re: RegExp, text: string) {
    let m = re.find(text)
    if m != null {
        print(m.value, m.index, m.end)
    } else {
        print("miss")
    }
}

fn main() {
    match RegExp::compile("a|ab") {
        Result::Ok(re) => show_find(re, "ab")
        Result::Err(e) => print(e.message)
    }
    match RegExp::compile("a*") {
        Result::Ok(re) => {
            let all = re.find_all("bab")
            for m in all {
                print(`[${m.value}]`)
            }
        }
        Result::Err(e) => print(e.message)
    }
    match RegExp::compile("a.c", "s") {
        Result::Ok(re) => print(re.test("a\nc"))
        Result::Err(e) => print(e.message)
    }
    match RegExp::compile("a.c") {
        Result::Ok(re) => print(re.test("a\nc"))
        Result::Err(e) => print(e.message)
    }
    match RegExp::compile("^b", "m") {
        Result::Ok(re) => print(re.test("a\nb"))
        Result::Err(e) => print(e.message)
    }
    match RegExp::compile("^b") {
        Result::Ok(re) => print(re.test("a\nb"))
        Result::Err(e) => print(e.message)
    }
    match RegExp::compile("a+?") {
        Result::Ok(re) => show_find(re, "aaa")
        Result::Err(e) => print(e.message)
    }
    match RegExp::compile("a{2,3}") {
        Result::Ok(re) => show_find(re, "aaaa")
        Result::Err(e) => print(e.message)
    }
    match RegExp::compile("(?P<word>\\w+)") {
        Result::Ok(re) => {
            let m = re.find("hi there")
            if m != null {
                print(m.group("word"), m.group(1))
            } else {
                print("miss")
            }
            print("x hi y".replace(re, "[$word]"))
        }
        Result::Err(e) => print(e.message)
    }
    match RegExp::compile("\\bcat\\b") {
        Result::Ok(re) => print(re.test("a cat!"), re.test("concat"))
        Result::Err(e) => print(e.message)
    }
    match RegExp::compile("(€+)") {
        Result::Ok(re) => {
            let m = re.find("a€€b")
            if m != null {
                print(m.value, m.index, m.end)
            } else {
                print("miss")
            }
        }
        Result::Err(e) => print(e.message)
    }
}
"#,
    )
    .expect("the regex probe should be writable");

    let interpreted = Command::new(XUPM)
        .args(["run", source.to_str().expect("a UTF-8 path")])
        .current_dir(&root)
        .output()
        .expect("xupm run should start");
    assert_success(&interpreted, "the interpreter regex probe");

    let program = build_stage1_program(&stage1, &root, &source, &work, "regex");
    let native = run_program(&program);
    assert_success(&native, "the stage 1 regex probe");
    assert_eq!(
        render_lines(&native.stdout),
        render_lines(&interpreted.stdout),
        "the C backend's regex output must match the interpreter"
    );
}

/// `std::net`: handle validation and argument domains. A foreign handle, a
/// listener used as a stream, a read size outside `1..=16777216`, and a second
/// close are all clean `Err` values rather than panics.
#[test]
fn stage1_c_backend_lowers_net_errors() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("net");
    let source = work.join("net_errors.xulo");
    std::fs::write(
        &source,
        r#"fn main() {
    match std::net::sock_read(999999, 8) {
        Result::Ok(_) => print("ok")
        Result::Err(e) => print(e.message != "")
    }
    match std::net::sock_read(999999, 0) {
        Result::Ok(_) => print("ok")
        Result::Err(e) => print(e.message != "")
    }
    match std::net::tcp_listen("127.0.0.1:0") {
        Result::Ok(server) => {
            match std::net::sock_write(server, "x") {
                Result::Ok(_) => print("ok")
                Result::Err(e) => print(e.message != "")
            }
            match std::net::sock_close(server) {
                Result::Ok(_) => print("closed")
                Result::Err(e) => print(e.message)
            }
            match std::net::sock_close(server) {
                Result::Ok(_) => print("again")
                Result::Err(e) => print(e.message != "")
            }
        }
        Result::Err(e) => print(e.message)
    }
}
"#,
    )
    .expect("the net error probe should be writable");

    let interpreted = Command::new(XUPM)
        .args(["run", source.to_str().expect("a UTF-8 path")])
        .current_dir(&root)
        .output()
        .expect("xupm run should start");
    assert_success(&interpreted, "the interpreter net error probe");

    let program = build_stage1_program(&stage1, &root, &source, &work, "net_errors");
    let native = run_program(&program);
    assert_success(&native, "the stage 1 net error probe");
    assert_eq!(
        render_lines(&native.stdout),
        render_lines(&interpreted.stdout),
        "the C backend's net error behavior must match the interpreter"
    );
}

/// `std::process`: shell `run`, `Command::shell().run()`, and
/// `Command::shell().launch()` + `wait` + a spent second wait. Uses only
/// `echo`, which both `sh -c` and `cmd /C` provide.
#[test]
fn stage1_c_backend_lowers_process() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("process");
    let source = work.join("process.xulo");
    std::fs::write(
        &source,
        r#"fn main() {
    match std::process::run("echo process-ok") {
        Result::Ok(r) => {
            print(r.code)
            print(r.stdout)
        }
        Result::Err(e) => print(e.message)
    }
    match std::process::Command::shell("echo command-ok").run() {
        Result::Ok(r) => print(r.stdout)
        Result::Err(e) => print(e.message)
    }
    match std::process::Command::shell("echo child-ok").launch() {
        Result::Ok(child) => {
            match child.wait() {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
            match child.wait() {
                Result::Ok(_) => print("again")
                Result::Err(e) => print(e.message != "")
            }
        }
        Result::Err(e) => print(e.message)
    }
}
"#,
    )
    .expect("the process probe should be writable");

    let interpreted = Command::new(XUPM)
        .args(["run", source.to_str().expect("a UTF-8 path")])
        .current_dir(&root)
        .output()
        .expect("xupm run should start");
    assert_success(&interpreted, "the interpreter process probe");

    let program = build_stage1_program(&stage1, &root, &source, &work, "process");
    let native = run_program(&program);
    assert_success(&native, "the stage 1 process probe");
    assert_eq!(
        render_lines(&native.stdout),
        render_lines(&interpreted.stdout),
        "the C backend's process output must match the interpreter"
    );
}

/// `defer` runs at block exit in reverse registration order: on fallthrough,
/// `return`, `break`, `continue`, and `?`, after the block value is determined,
/// and for nested blocks innermost first.
#[test]
fn stage1_c_backend_lowers_defer() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("defer");
    let source = work.join("defer.xulo");
    std::fs::write(
        &source,
        r#"fn check(flag: bool): Result<int, Error> {
    defer print("check cleanup")
    if flag {
        return Result::Err(Error("boom"))
    }
    Result::Ok(7)
}

fn attempt(flag: bool): Result<unit, Error> {
    defer print("attempt cleanup")
    let value = check(flag)?
    print(value + 1)
    Result::Ok(null)
}

fn cycles() {
    for i in 0..<4 {
        defer print(`done ${i}`)
        if i == 1 {
            continue
        }
        if i == 3 {
            break
        }
        print(`body ${i}`)
    }
}

fn nested() {
    defer {
        print("outer one")
        print("outer two")
    }
    if true {
        defer print("inner")
        print("inner body")
    }
    print("after inner")
}

fn main() {
    attempt(false)
    attempt(true)
    cycles()
    nested()
    let x = if true {
        defer print("block cleanup")
        5
    } else {
        0
    }
    print(x)
}
"#,
    )
    .expect("the defer probe should be writable");

    let interpreted = Command::new(XUPM)
        .args(["run", source.to_str().expect("a UTF-8 path")])
        .current_dir(&root)
        .output()
        .expect("xupm run should start");
    assert_success(&interpreted, "the interpreter defer probe");

    let program = build_stage1_program(&stage1, &root, &source, &work, "defer");
    let native = run_program(&program);
    assert_success(&native, "the stage 1 defer probe");
    assert_eq!(
        render_lines(&native.stdout),
        render_lines(&interpreted.stdout),
        "the C backend's defer output must match the interpreter"
    );
}

/// `std::fmt::format`: every verb, flags, width, precision, quoting, scalar
/// output, and the `Display`/`Debug` consultation for user structs. The
/// compiled output must match the interpreter byte for byte.
#[test]
fn stage1_c_backend_lowers_fmt() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("fmt");
    let source = work.join("fmt.xulo");
    std::fs::write(
        &source,
        r#"fn main() {
    print(std::fmt::format("%6.2f", [3.14159]))
    print(std::fmt::format("%e", [1234.5]))
    print(std::fmt::format("%E", [0.000123]))
    print(std::fmt::format("%f", [-0.0]))
    print(std::fmt::format("%e", [-0.0]))
    print(std::fmt::format("%g", [100000.0]))
    print(std::fmt::format("%g", [1234567.0]))
    print(std::fmt::format("%g", [std::math::pow(10.0, 20.0)]))
    print(std::fmt::format("%g", [0.1]))
    print(std::fmt::format("%.0f", [2.5]))
    print(std::fmt::format("%.1f", [2.25]))
    print(std::fmt::format("%x", [-255]))
    print(std::fmt::format("%x", [-9223372036854775808]))
    print(std::fmt::format("%#b", [5]))
    print(std::fmt::format("%q", ["a\n"]))
    print(std::fmt::format("%q", ["q\"b\\c\td"]))
    print(std::fmt::format("%c", [8364]))
    print(std::fmt::format("%3s|", ["€"]))
    print(std::fmt::format("%.3s", [[1, 2, 3]]))
    print(std::fmt::format("100%%"))
    print(std::fmt::format("%v", [[1, [2, 3]]]))
    print(std::fmt::format("%d %d", [1, 2]))
}
"#,
    )
    .expect("the fmt probe should be writable");

    let interpreted = Command::new(XUPM)
        .args(["run", source.to_str().expect("a UTF-8 path")])
        .current_dir(&root)
        .output()
        .expect("xupm run should start");
    assert_success(&interpreted, "the interpreter fmt probe");

    let program = build_stage1_program(&stage1, &root, &source, &work, "fmt");
    let native = run_program(&program);
    assert_success(&native, "the stage 1 fmt probe");
    assert_eq!(
        render_lines(&native.stdout),
        render_lines(&interpreted.stdout),
        "the C backend's fmt output must match the interpreter"
    );
}

/// D-01/D-03 (closed): checked integral arithmetic and the runtime-failure
/// prefix. Stage 0's native runtime traps with `panic: <message>`; the C
/// backend now does the same instead of wrapping or printing the bare message.
#[test]
fn stage1_c_backend_traps_like_the_stage0_runtime() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("runtime_traps");
    let cases = [
        (
            "overflow",
            "fn main() {\n    let a: int = 9223372036854775807\n    let b: int = 1\n    print(a + b)\n}\n",
            "integer overflow",
        ),
        (
            "division",
            "fn main() {\n    let a: int = 1\n    let b: int = 0\n    print(a / b)\n}\n",
            "division by zero",
        ),
        ("panic", "fn main() {\n    panic(\"boom\")\n}\n", "boom"),
    ];
    for (name, source, message) in cases {
        let file = work.join(format!("{name}.xulo"));
        std::fs::write(&file, source).expect("the trap probe should be writable");
        let program = build_stage1_program(&stage1, &root, &file, &work, name);
        let run = run_program(&program);
        assert_eq!(
            run.status.code(),
            Some(1),
            "{name} must exit 1, stdout:\n{}",
            render(&run.stdout)
        );
        assert_eq!(
            render_lines(&run.stderr),
            format!("panic: {message}\n"),
            "{name} must report the stage 0 runtime message"
        );
    }
}

/// D-04 (closed): the bootstrap runtime has one signed 64-bit slot, so `u64`
/// (annotations and literals above `i64::MAX`) is rejected with `E0005`
/// instead of being silently treated as `int`.
#[test]
fn stage1_rejects_u64_in_the_bootstrap_subset() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("u64");
    let cases = [
        (
            "annotation",
            "fn main() {\n    let x: u64 = 1\n    print(x)\n}\n",
            "u64",
        ),
        (
            "literal",
            "fn main() {\n    print(18446744073709551615)\n}\n",
            "i64::MAX",
        ),
        (
            "hex-literal",
            "fn main() {\n    print(0x8000000000000000)\n}\n",
            "i64::MAX",
        ),
    ];
    for (name, source, needle) in cases {
        let file = work.join(format!("{name}.xulo"));
        std::fs::write(&file, source).expect("the u64 probe should be writable");
        let output = Command::new(&stage1)
            .args(["build", file.to_str().expect("a UTF-8 path")])
            .current_dir(&root)
            .env(
                "XULO_STAGE1_OUT",
                work.join(format!("{name}{}", exe_suffix())),
            )
            .env("XULO_STAGE1_C", work.join(format!("{name}.c")))
            .env_remove("XULO_STAGE1_MODE")
            .env_remove("XULO_STAGE1_FILE")
            .output()
            .expect("stage 1 should start");
        assert_eq!(
            output.status.code(),
            Some(1),
            "{name} must be rejected, stdout:\n{}\nstderr:\n{}",
            render(&output.stdout),
            render(&output.stderr)
        );
        let stderr = render_lines(&output.stderr);
        assert!(
            stderr.contains("error[E0005]"),
            "{name} must be a codegen rejection, got:\n{stderr}"
        );
        assert!(
            stderr.contains(needle),
            "{name} must name `{needle}`, got:\n{stderr}"
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
    let parsed = run_stage1(
        &stage2.binary,
        &root,
        "parse",
        Path::new("xuloc/src/ast.xulo"),
    );
    assert_success(&parsed, "stage 2 parse of xuloc/src/ast.xulo");
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
        Path::new("xuloc/src/main.xulo"),
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

/// The binary path reported on the `build successful:` line.
fn reported_build_path(output: &Output) -> PathBuf {
    let stdout = render(&output.stdout);
    let line = stdout
        .lines()
        .find_map(|line| line.strip_prefix("build successful: "))
        .unwrap_or_else(|| panic!("xupm did not report the built binary:\n{stdout}"));
    Path::new(line.split_once(" (backend:").map_or(line, |(path, _)| path)).to_path_buf()
}

/// drift-governance §5.3: build the compiler with an explicit stage 0 backend.
fn build_stage1_with(backend: &str) -> PathBuf {
    let root = repo_root();
    let lib = runtime_lib(&root);
    let output = Command::new(XUPM)
        .args([
            "build",
            "xuloc/src/main.xulo",
            "--release",
            "--backend",
            backend,
        ])
        .env("XULO_RUNTIME_LIB", &lib)
        .current_dir(&root)
        .output()
        .expect("xupm build should start");
    assert_success(&output, &format!("xupm build --backend {backend}"));
    reported_build_path(&output)
}

/// §5.3: the two stage 0 backends must produce the same stage 1 behavior and
/// the same emitted C. Only a binary built with the `llvm` feature can run it.
#[test]
fn backend_cross_check() {
    if !enabled() {
        return;
    }
    if !cfg!(feature = "llvm") {
        eprintln!("skipping the backend cross-check: this binary was built without `llvm`");
        return;
    }
    let root = repo_root();
    let work = unique_dir("backend");
    // Both builds write `.xupm/build/xuloc`, so keep the first one aside.
    let llvm_stage1 = {
        let built = build_stage1_with("llvm");
        let copy = work.join(format!("stage1_llvm{}", exe_suffix()));
        std::fs::copy(&built, &copy).expect("copy the llvm stage 1 binary");
        copy
    };
    let cranelift_stage1 = build_stage1_with("cranelift");

    for file in stage1_sources(&root) {
        let case = relative_case(&root, &file);
        for command in ["tokens", "parse"] {
            let left = observe(&llvm_stage1, &root, command, Path::new(&case), None, None);
            let right = observe(
                &cranelift_stage1,
                &root,
                command,
                Path::new(&case),
                None,
                None,
            );
            assert_eq!(
                left, right,
                "the stage 0 backends disagree on {case} ({command})"
            );
        }
    }

    let c_path = work.join("cross.c");
    let binary_path = work.join(format!("cross{}", exe_suffix()));
    for file in accept_corpus(&root).into_iter().chain(reject_corpus(&root)) {
        let case = relative_case(&root, &file);
        let left = observe(&llvm_stage1, &root, "check", Path::new(&case), None, None);
        let right = observe(
            &cranelift_stage1,
            &root,
            "check",
            Path::new(&case),
            None,
            None,
        );
        assert_eq!(
            left, right,
            "the stage 0 backends disagree on {case} (check)"
        );
    }

    for file in accept_corpus(&root) {
        let case = relative_case(&root, &file);
        let case_path = Path::new(&case);
        let left = observe(
            &llvm_stage1,
            &root,
            "build",
            case_path,
            Some(&c_path),
            Some(&binary_path),
        );
        let llvm_run = std::fs::metadata(&binary_path)
            .ok()
            .filter(|_| left.exit == Some(0))
            .map(|_| run_program(&binary_path));
        let right = observe(
            &cranelift_stage1,
            &root,
            "build",
            case_path,
            Some(&c_path),
            Some(&binary_path),
        );
        let cranelift_run = std::fs::metadata(&binary_path)
            .ok()
            .filter(|_| right.exit == Some(0))
            .map(|_| run_program(&binary_path));
        assert_eq!(
            left, right,
            "the stage 0 backends disagree on {case} (build)"
        );
        if let (Some(left_run), Some(right_run)) = (llvm_run, cranelift_run) {
            assert_eq!(
                render_lines(&left_run.stdout),
                render_lines(&right_run.stdout),
                "the stage 0 backends produced differently behaving programs for {case}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// drift-governance §5.4: the stage 0 / stage 1 differential gate
// ---------------------------------------------------------------------------

/// Rows from `docs/drift-governance.md` §8 that may explain a stage 0 / stage 1
/// difference. Each row is `(id, case glob, command)`; `*` matches any suffix
/// of the case, and `*` as the command matches every command. A difference that
/// matches no row fails the gate.
const DRIFT_LEDGER: &[(&str, &str, &str)] = &[];

/// The §6 observation for one engine and one command.
#[derive(Debug, PartialEq, Eq)]
struct Observation {
    exit: Option<i32>,
    stdout: String,
    stderr: String,
    /// Emitted C, compared only for `build` (§5.1's primary object).
    c_source: Option<Vec<u8>>,
}

/// Locate the ledger row that explains a difference, if any.
fn ledger_row<'a>(
    ledger: &'a [(&str, &'a str, &'a str)],
    case: &str,
    command: &str,
) -> Option<&'a str> {
    ledger.iter().find_map(|(id, glob, row_command)| {
        let case_matches = match glob.strip_suffix('*') {
            Some(prefix) => case.starts_with(prefix),
            None => case == *glob,
        };
        (case_matches && (*row_command == "*" || *row_command == command)).then_some(*id)
    })
}

fn relative_case(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .expect("case lives inside the repository")
        .to_string_lossy()
        .replace('\\', "/")
}

fn observe(
    binary: &Path,
    root: &Path,
    command: &str,
    file: &Path,
    c_source: Option<&Path>,
    binary_out: Option<&Path>,
) -> Observation {
    let mut cmd = Command::new(binary);
    cmd.arg(command).arg(file).current_dir(root);
    cmd.env_remove("XULO_STAGE1_MODE")
        .env_remove("XULO_STAGE1_FILE");
    if let Some(path) = c_source {
        let _ = std::fs::remove_file(path);
        cmd.env("XULO_STAGE1_C", path);
    }
    if let Some(path) = binary_out {
        let _ = std::fs::remove_file(path);
        cmd.env("XULO_STAGE1_OUT", path);
    }
    let output = cmd
        .output()
        .unwrap_or_else(|error| panic!("cannot run {} {command}: {error}", binary.display()));
    Observation {
        exit: output.status.code(),
        stdout: render_lines(&output.stdout),
        stderr: render_lines(&output.stderr),
        c_source: c_source.and_then(|path| std::fs::read(path).ok()),
    }
}

/// Compare one case/command pair and record an unmatched difference.
fn record_difference(
    case: &str,
    command: &str,
    left: &Observation,
    right: &Observation,
    differences: &mut Vec<String>,
    matched: &mut Vec<&'static str>,
) {
    if left == right {
        return;
    }
    match ledger_row(DRIFT_LEDGER, case, command) {
        Some(id) => {
            if !matched.contains(&id) {
                matched.push(id);
            }
            eprintln!("drift: {id} explains a difference in {case} ({command})");
        }
        None => {
            let mut fields = Vec::new();
            if left.exit != right.exit {
                fields.push(format!("exit {:?} != {:?}", left.exit, right.exit));
            }
            if left.stdout != right.stdout {
                fields.push("stdout".to_string());
            }
            if left.stderr != right.stderr {
                fields.push("stderr".to_string());
            }
            if left.c_source != right.c_source {
                fields.push("emitted C".to_string());
            }
            differences.push(format!("{case} ({command}): {}", fields.join(", ")));
        }
    }
}

/// One §5.1 input on the stage 0 / stage 1 axis.
struct DriftCase {
    case: String,
    /// Whether `build` is compared for this input. The compiler sources are
    /// built by the self-host tests, which would multiply the gate's runtime.
    build: bool,
    /// Whether a successful build is expected to produce a runnable program.
    runs: bool,
}

fn drift_cases(root: &Path) -> Vec<DriftCase> {
    let mut cases: Vec<DriftCase> = stage1_sources(root)
        .iter()
        .map(|file| DriftCase {
            case: relative_case(root, file),
            build: false,
            runs: false,
        })
        .collect();
    cases.extend(accept_corpus(root).iter().map(|file| DriftCase {
        case: relative_case(root, file),
        build: true,
        runs: true,
    }));
    cases.extend(reject_corpus(root).iter().map(|file| DriftCase {
        case: relative_case(root, file),
        build: true,
        runs: false,
    }));
    cases
}

/// D-06: a C compiler that runs and fails must surface as `E0005` with both
/// streams, not as a crash. The compiler under test plays the failing compiler:
/// it rejects the C compiler's arguments and exits 1 with a usage line.
#[test]
fn a_failing_c_compiler_is_reported_as_e0005() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage1 = stage1_binary();
    let work = unique_dir("cc_fail");
    let output = Command::new(&stage1)
        .arg("build")
        .arg(Path::new("xuloc/corpus/accept/basics.xulo"))
        .current_dir(&root)
        .env("XULO_CC", &stage1)
        .env("XULO_STAGE1_C", work.join("basics.c"))
        .env(
            "XULO_STAGE1_OUT",
            work.join(format!("basics{}", exe_suffix())),
        )
        .env_remove("XULO_STAGE1_MODE")
        .env_remove("XULO_STAGE1_FILE")
        .output()
        .expect("stage 1 should start");
    assert_eq!(
        output.status.code(),
        Some(1),
        "a failing C compiler must exit 1 instead of crashing (status {:?})",
        output.status
    );
    let stderr = render_lines(&output.stderr);
    assert!(
        stderr.contains("error[E0005]"),
        "the failure must be reported as E0005, got:\n{stderr}"
    );
    assert!(
        stderr.contains("usage:"),
        "the compiler's own output must be forwarded, got:\n{stderr}"
    );
}

#[test]
fn drift_ledger_matching_is_precise() {
    let ledger = [
        ("D-05", "xuloc/corpus/semantics/*", "*"),
        ("D-01", "xuloc/corpus/accept/basics.xulo", "build"),
    ];
    assert_eq!(
        ledger_row(&ledger, "xuloc/corpus/semantics/accept/x.xulo", "build"),
        Some("D-05")
    );
    assert_eq!(
        ledger_row(&ledger, "xuloc/corpus/accept/basics.xulo", "build"),
        Some("D-01")
    );
    assert_eq!(
        ledger_row(&ledger, "xuloc/corpus/accept/fibonacci.xulo", "build"),
        None
    );
    assert_eq!(
        ledger_row(&ledger, "xuloc/corpus/accept/basics.xulo", "check"),
        None
    );
}

/// The C-backend build of the compiler. `XULO_STAGE1_STAGE2` reuses a cached
/// binary (§9's stage 2 cache) instead of self-building one.
fn c_backend_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("XULO_STAGE1_STAGE2") {
        let path = PathBuf::from(path);
        assert!(
            path.is_file(),
            "XULO_STAGE1_STAGE2 points to a missing stage 2 binary: {}",
            path.display()
        );
        return path;
    }
    stage2_binary().binary
}

/// drift-governance §5.4: every §5.1 input runs through stage 0 (the AOT build
/// of the compiler source, which shares the Rust semantics) and through stage
/// 1's C backend. A difference must be explained by a ledger row.
#[test]
fn drift_gate_reconciles_stage0_and_stage1() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let stage0 = stage1_binary();
    let stage1 = c_backend_binary();
    let stage1 = &stage1;
    let work = unique_dir("drift");
    let c_path = work.join("drift.c");
    let binary_path = work.join(format!("drift{}", exe_suffix()));
    let mut differences: Vec<String> = Vec::new();
    let mut matched: Vec<&'static str> = Vec::new();

    for case in drift_cases(&root) {
        let file = Path::new(&case.case);
        for command in ["tokens", "parse", "check"] {
            let left = observe(&stage0, &root, command, file, None, None);
            let right = observe(stage1, &root, command, file, None, None);
            record_difference(
                &case.case,
                command,
                &left,
                &right,
                &mut differences,
                &mut matched,
            );
        }
        if !case.build {
            continue;
        }

        let left = observe(
            &stage0,
            &root,
            "build",
            file,
            Some(&c_path),
            Some(&binary_path),
        );
        let right = observe(
            stage1,
            &root,
            "build",
            file,
            Some(&c_path),
            Some(&binary_path),
        );
        record_difference(
            &case.case,
            "build",
            &left,
            &right,
            &mut differences,
            &mut matched,
        );

        if case.runs && left.exit == Some(0) && right.exit == Some(0) {
            let built = Path::new(&binary_path);
            if built.is_file() {
                let ran = run_program(built);
                assert_success(&ran, &format!("running the build of {}", case.case));
            }
        }
    }

    // A ledger row that explains nothing is stale (full runs only, because the
    // fast tier does not exercise the whole matrix).
    if std::env::var_os("XULO_STAGE1_FULL").is_some() {
        for (id, glob, command) in DRIFT_LEDGER {
            if !matched.contains(id) {
                differences.push(format!(
                    "ledger row {id} ({glob}, {command}) explains no difference and is stale"
                ));
            }
        }
    }

    assert!(
        differences.is_empty(),
        "the stage 0 / stage 1 gate found {} unexplained difference(s):\n{}",
        differences.len(),
        differences.join("\n")
    );
}
