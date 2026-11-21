//! Test support for the `examples/` corpus.
//!
//! `examples/` is the stage 0 syntax gate and the bootstrap corpus: every
//! example is discovered here, its header directives are parsed here, and its
//! expected-output sidecars are resolved here. Both consumers
//! (`crates/xulo-cli/tests/examples.rs` and
//! `crates/xupm/tests/aot_differential.rs`) go through this module so the two
//! gates cannot drift apart.
//!
//! # Layout rule
//!
//! - A `.xulo` file directly inside a category directory is a single-file
//!   program.
//! - A directory that contains `main.xulo` is one multi-file program unit; its
//!   other `.xulo` files are support modules and must be named by a `mod`
//!   declaration in `main.xulo`.
//!
//! # Header directives
//!
//! Directives live in the leading comment block, one per line:
//!
//! ```text
//! // @no-aot all: `defer` has no AOT lowering yet
//! // @skip-aot all: the entry point is `xulo test`, not `main`
//! // @no-run windows: `extern "C"` FFI is Unix-only
//! // @error E0006
//! // @panic attempt to divide by zero
//! // @test-expect 5 passed, 0 failed
//! ```
//!
//! # Expected output
//!
//! `<stem>.expected` holds stdout, `<stem>.stderr.expected` holds stderr, and a
//! `<stem>.<platform>.expected` / `<stem>.<platform>.stderr.expected` file
//! overrides the shared one on that platform (`windows`, `linux`, `macos`).

use std::path::{Path, PathBuf};

/// A platform a directive applies to, or `All`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    All,
    Windows,
    Linux,
    Macos,
}

impl Platform {
    /// The platform of the running test binary.
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::Macos
        } else {
            Platform::Linux
        }
    }

    fn parse(text: &str) -> Option<Self> {
        match text {
            "all" => Some(Platform::All),
            "windows" => Some(Platform::Windows),
            "linux" => Some(Platform::Linux),
            "macos" => Some(Platform::Macos),
            _ => None,
        }
    }

    /// The spelling used in directives and sidecar names.
    pub fn name(self) -> &'static str {
        match self {
            Platform::All => "all",
            Platform::Windows => "windows",
            Platform::Linux => "linux",
            Platform::Macos => "macos",
        }
    }

    /// Whether this directive applies to the running host.
    pub fn applies_here(self) -> bool {
        self == Platform::All || self == Platform::current()
    }
}

/// A platform directive plus its justification.
#[derive(Clone, Debug)]
pub struct Gap {
    pub platform: Platform,
    pub reason: String,
}

/// The directives of one example, in written order.
#[derive(Clone, Debug, Default)]
pub struct Directives {
    /// Excluded from the AOT differential; the native build must still fail.
    pub no_aot: Vec<Gap>,
    /// Excluded from the AOT differential; the native build is not checked.
    pub skip_aot: Vec<Gap>,
    /// Compared as a normalized failure shape instead of byte for byte: both
    /// paths must fail with the same runtime message (T13 option c).
    pub failure_shape: Vec<Gap>,
    /// Not executed on this platform (`xulo check` only).
    pub no_run: Vec<Gap>,
    /// Expected static-error code from `xulo check`.
    pub error: Option<String>,
    /// Expected substring of stderr from a failing `xulo run`.
    pub panic: Option<String>,
    /// Expected `xulo test` summary line.
    pub test_expect: Option<String>,
}

impl Directives {
    fn first_here(gaps: &[Gap]) -> Option<&str> {
        gaps.iter()
            .find(|gap| gap.platform.applies_here())
            .map(|gap| gap.reason.as_str())
    }

    /// Why this example is outside the AOT differential here, as a recorded gap.
    pub fn no_aot_here(&self) -> Option<&str> {
        Self::first_here(&self.no_aot)
    }

    /// Why this example is outside the AOT differential here, as a skip.
    pub fn skip_aot_here(&self) -> Option<&str> {
        Self::first_here(&self.skip_aot)
    }

    /// Why this example compares a normalized failure shape here.
    pub fn failure_shape_here(&self) -> Option<&str> {
        Self::first_here(&self.failure_shape)
    }

    /// Why this example is check-only here.
    pub fn no_run_here(&self) -> Option<&str> {
        Self::first_here(&self.no_run)
    }
}

/// One program unit of the corpus.
pub struct Example {
    /// Path relative to `examples/`, always with `/` separators.
    pub rel: String,
    /// Absolute path of the entry file (`main.xulo` for a multi-file unit).
    pub entry: PathBuf,
    /// Source of the entry file.
    pub source: String,
    pub directives: Directives,
}

impl Example {
    /// The unit directory, used as the sidecar location.
    pub fn dir(&self) -> &Path {
        self.entry.parent().unwrap_or(Path::new("."))
    }
}

/// The discovered corpus.
pub struct Corpus {
    /// Absolute path of `examples/`.
    pub dir: PathBuf,
    /// Every `.xulo` file under `examples/`, sorted.
    pub files: Vec<PathBuf>,
    /// Every program unit, sorted by `rel`.
    pub units: Vec<Example>,
}

impl Corpus {
    /// The unit whose entry is exactly `rel` (`basics/hello.xulo`).
    pub fn unit(&self, rel: &str) -> Option<&Example> {
        self.units.iter().find(|unit| unit.rel == rel)
    }
}

/// Discover the corpus under `<root>/examples`.
///
/// Panics with a path-bearing message when the layout rule is violated, so a
/// misplaced file fails the gate instead of disappearing from it.
pub fn load(root: &Path) -> Corpus {
    let dir = root.join("examples");
    let mut files = Vec::new();
    walk(&dir, &mut files);
    files.sort();

    let mut units = Vec::new();
    for file in &files {
        let parent = file.parent().expect("an example file has a parent");
        let main = parent.join("main.xulo");
        let is_main = file.file_name().is_some_and(|name| name == "main.xulo");
        if !is_main && main.is_file() {
            continue;
        }
        let source = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let directives = parse_directives(file, &source);
        let rel = relative(&dir, file);
        units.push(Example {
            rel,
            entry: file.clone(),
            source,
            directives,
        });
    }
    units.sort_by(|a, b| a.rel.cmp(&b.rel));
    check_support_modules(&dir, &files, &units);
    Corpus { dir, files, units }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("a directory entry is readable").path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if path.is_dir() {
            if name.starts_with('.') {
                continue;
            }
            walk(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "xulo") {
            out.push(path);
        }
    }
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// A support module that `main.xulo` never names is a moved or orphaned file.
fn check_support_modules(root: &Path, files: &[PathBuf], units: &[Example]) {
    for file in files {
        let parent = file.parent().expect("an example file has a parent");
        if !parent.join("main.xulo").is_file() {
            continue;
        }
        if file.file_name().is_some_and(|name| name == "main.xulo") {
            continue;
        }
        let stem = file.file_stem().unwrap_or_default().to_string_lossy();
        let unit = units
            .iter()
            .find(|unit| unit.dir() == parent)
            .unwrap_or_else(|| panic!("no unit for {}", file.display()));
        let named = unit
            .source
            .lines()
            .map(str::trim)
            .any(|line| line.starts_with("mod ") && line.contains(&format!("mod {stem}")));
        assert!(
            named,
            "{}: support module `{stem}` is not declared by {}",
            relative(root, file),
            relative(root, &unit.entry)
        );
    }
}

fn parse_directives(path: &Path, source: &str) -> Directives {
    let mut directives = Directives::default();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some(comment) = trimmed.strip_prefix("//") else {
            break;
        };
        let body = comment.trim();
        let Some(rest) = body.strip_prefix('@') else {
            continue;
        };
        let at = index + 1;
        if let Some(arg) = argument(rest, "no-aot") {
            directives.no_aot.push(parse_gap(path, at, "no-aot", arg));
        } else if let Some(arg) = argument(rest, "skip-aot") {
            directives
                .skip_aot
                .push(parse_gap(path, at, "skip-aot", arg));
        } else if let Some(arg) = argument(rest, "aot-failure-shape") {
            directives
                .failure_shape
                .push(parse_gap(path, at, "aot-failure-shape", arg));
        } else if let Some(arg) = argument(rest, "no-run") {
            directives.no_run.push(parse_gap(path, at, "no-run", arg));
        } else if let Some(arg) = argument(rest, "error") {
            directives.error = Some(non_empty(path, at, "error", arg));
        } else if let Some(arg) = argument(rest, "panic") {
            directives.panic = Some(non_empty(path, at, "panic", arg));
        } else if let Some(arg) = argument(rest, "test-expect") {
            directives.test_expect = Some(non_empty(path, at, "test-expect", arg));
        } else {
            panic!("{}:{at}: unknown directive `@{rest}`", path.display());
        }
    }
    directives
}

fn argument<'a>(rest: &'a str, name: &str) -> Option<&'a str> {
    let tail = rest.strip_prefix(name)?;
    if tail.is_empty() {
        Some("")
    } else if tail.starts_with(char::is_whitespace) {
        Some(tail.trim())
    } else {
        None
    }
}

fn non_empty(path: &Path, at: usize, name: &str, arg: &str) -> String {
    assert!(
        !arg.is_empty(),
        "{}:{at}: `@{name}` needs a value",
        path.display()
    );
    arg.to_string()
}

fn parse_gap(path: &Path, at: usize, name: &str, arg: &str) -> Gap {
    let Some((platform, reason)) = arg.split_once(':') else {
        panic!(
            "{}:{at}: `@{name} <platform>: <reason>` needs a platform and a reason",
            path.display()
        );
    };
    let platform = Platform::parse(platform.trim()).unwrap_or_else(|| {
        panic!(
            "{}:{at}: `@{name}` platform must be all, windows, linux, or macos",
            path.display()
        )
    });
    Gap {
        platform,
        reason: non_empty(path, at, name, reason.trim()),
    }
}

/// Normalize captured output for comparison: CRLF becomes LF.
pub fn normalize(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

/// The sidecar that holds `stdout` (`stderr == false`) or `stderr` for `entry`
/// on the running host, if it exists.
pub fn sidecar(entry: &Path, stderr: bool) -> Option<(PathBuf, String)> {
    for platform in [Platform::current(), Platform::All] {
        let path = sidecar_path(entry, platform, stderr);
        if path.is_file() {
            let text = std::fs::read(&path)
                .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
            return Some((path, normalize(&text)));
        }
    }
    None
}

/// Where `XULO_BLESS=1` writes the captured output.
///
/// An existing platform override keeps being the target, so blessing on
/// Windows never rewrites the expectations Linux and macOS share.
pub fn bless_target(entry: &Path, stderr: bool) -> PathBuf {
    let platform = sidecar_path(entry, Platform::current(), stderr);
    if platform.is_file() {
        return platform;
    }
    sidecar_path(entry, Platform::All, stderr)
}

fn sidecar_path(entry: &Path, platform: Platform, stderr: bool) -> PathBuf {
    let dir = entry.parent().unwrap_or(Path::new("."));
    let stem = entry.file_stem().unwrap_or_default().to_string_lossy();
    let suffix = if stderr {
        ".stderr.expected"
    } else {
        ".expected"
    };
    let name = match platform {
        Platform::All => format!("{stem}{suffix}"),
        platform => format!("{stem}.{}{suffix}", platform.name()),
    };
    dir.join(name)
}

/// The repository's `examples/` directory, from this crate's location.
pub fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

/// The recorded stdout of `examples/<rel>`, as lines.
///
/// This is the same sidecar the example gate compares against, so a test that
/// pins one example's output does not keep a second copy of it.
pub fn expected_lines(rel: &str) -> Vec<String> {
    let entry = examples_dir().join(rel);
    let (_, text) = sidecar(&entry, false)
        .unwrap_or_else(|| panic!("no stdout sidecar for {}", entry.display()));
    text.lines().map(str::to_string).collect()
}

/// The recorded stdout of `examples/<rel>` as one string, without the trailing
/// newline. Use this when a printed line itself contains a newline.
pub fn expected_text(rel: &str) -> String {
    let entry = examples_dir().join(rel);
    let (_, text) = sidecar(&entry, false)
        .unwrap_or_else(|| panic!("no stdout sidecar for {}", entry.display()));
    text.strip_suffix('\n').unwrap_or(&text).to_string()
}

/// Compare captured output with a sidecar, and explain the first divergence.
/// Returns `None` when the output matches (or when the sidecar is absent).
pub fn mismatch(entry: &Path, stderr: bool, actual: &[u8]) -> Option<String> {
    let (path, expected) = sidecar(entry, stderr)?;
    let actual = normalize(actual);
    if actual == expected {
        return None;
    }
    let mut report = format!("{} does not match {}\n", entry.display(), path.display());
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    for index in 0..expected_lines.len().max(actual_lines.len()) {
        let want = expected_lines.get(index).copied().unwrap_or("<missing>");
        let got = actual_lines.get(index).copied().unwrap_or("<missing>");
        if want != got {
            let first = index.saturating_sub(2);
            let last = (index + 2).min(expected_lines.len().max(actual_lines.len()));
            let mut detail = String::new();
            for line in first..last {
                let want = expected_lines.get(line).copied().unwrap_or("<missing>");
                let got = actual_lines.get(line).copied().unwrap_or("<missing>");
                let marker = if want == got { " " } else { "!" };
                let _ = std::fmt::Write::write_fmt(
                    &mut detail,
                    format_args!("{marker} expected: {want}\n{marker} actual:   {got}\n"),
                );
            }
            report.push_str(&format!("first difference at line {}\n{detail}", index + 1));
            break;
        }
    }
    Some(report)
}
