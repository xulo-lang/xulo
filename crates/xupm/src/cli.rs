use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use xulo_compiler::aot::{AotBackend, BuildProfile};
use xulo_core::error::XuloError;

use crate::manifest;

#[derive(Parser)]
#[command(
    name = "xupm",
    version,
    about = "Xulo package manager: resolve, fetch, and build projects"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create a new package in a new directory
    New { name: PathBuf },
    /// Create a package manifest in the current directory
    Init,
    /// Add a dependency to the manifest
    Add { spec: String },
    /// Remove a dependency from the manifest
    Remove { name: String },
    /// Fetch all dependencies into the local cache
    Fetch,
    /// Build the project's entry module
    Build {
        file: Option<PathBuf>,
        /// Select a package by name
        #[arg(short = 'p', long = "package", conflicts_with = "workspace")]
        package: Option<String>,
        /// Build every package in the workspace
        #[arg(long = "workspace")]
        workspace: bool,
        /// Build with the release optimization profile
        #[arg(long = "release")]
        release: bool,
    },
    /// Run the project's entry module
    Run {
        file: Option<PathBuf>,
        /// Select a package by name
        #[arg(short = 'p', long = "package")]
        package: Option<String>,
    },
    /// Type-check the project
    Check {
        file: Option<PathBuf>,
        /// Select a package by name
        #[arg(short = 'p', long = "package", conflicts_with = "workspace")]
        package: Option<String>,
        /// Check every package in the workspace
        #[arg(long = "workspace")]
        workspace: bool,
    },
    /// Run the project's tests
    Test {
        file: Option<PathBuf>,
        /// Select a package by name
        #[arg(short = 'p', long = "package", conflicts_with = "workspace")]
        package: Option<String>,
        /// Test every package in the workspace
        #[arg(long = "workspace")]
        workspace: bool,
    },
    /// Update the lockfile to the newest compatible versions
    Update,
    /// Print the resolved dependency tree
    Tree,
    /// Remove build artifacts
    Clean,
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();
    let Some(command) = cli.command else {
        Cli::command().print_help().ok();
        println!();
        return ExitCode::SUCCESS;
    };
    match execute(command) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("xupm: {message}");
            ExitCode::FAILURE
        }
    }
}

fn execute(command: Commands) -> Result<ExitCode, String> {
    match command {
        Commands::New { name } => new_project(&name),
        Commands::Build {
            file,
            package,
            workspace,
            release,
        } => build(file.as_deref(), package.as_deref(), workspace, release),
        Commands::Run { file, package } => run_project(file.as_deref(), package.as_deref()),
        Commands::Test {
            file,
            package,
            workspace,
        } => test_project(file.as_deref(), package.as_deref(), workspace),
        Commands::Check {
            file,
            package,
            workspace,
        } => check_project(file.as_deref(), package.as_deref(), workspace),
        Commands::Init => not_implemented("init"),
        Commands::Add { .. } => not_implemented("add"),
        Commands::Remove { .. } => not_implemented("remove"),
        Commands::Fetch => not_implemented("fetch"),
        Commands::Update => not_implemented("update"),
        Commands::Tree => not_implemented("tree"),
        Commands::Clean => not_implemented("clean"),
    }
}

/// The entry module and project root a command operates on. An explicit file
/// is used as the entry as-is; without one the entry comes from the manifest
/// of the nearest project. A workspace expands to one target per selected
/// package.
struct Target {
    root: PathBuf,
    name: String,
    entry: PathBuf,
}

/// Which packages a command operates on when no package is named.
#[derive(Clone, Copy)]
enum Mode {
    /// Run needs exactly one package: the root package, or an explicit `-p`.
    Run,
    /// Build/check/test: a root package defaults to itself; a virtual
    /// workspace defaults to every member; `--workspace` forces every member.
    All { all: bool },
}

fn resolve_targets(
    file: Option<&Path>,
    package: Option<&str>,
    mode: Mode,
) -> Result<Vec<Target>, String> {
    if let Some(file) = file {
        if package.is_some() || matches!(mode, Mode::All { all: true }) {
            return Err(
                "an explicit file cannot be combined with `--package` or `--workspace`".into(),
            );
        }
        return Ok(vec![target_from_file(file)?]);
    }
    let cwd =
        std::env::current_dir().map_err(|e| format!("cannot determine current directory: {e}"))?;
    let project = manifest::discover(&cwd)?
        .ok_or_else(|| "could not find `xupm.toml` in this directory or any parent".to_string())?;
    targets_from_project(project, package, mode)
}

fn target_from_file(file: &Path) -> Result<Target, String> {
    let entry = absolute(file)?;
    if !entry.is_file() {
        return Err(format!("cannot find {}", entry.display()));
    }
    let project = manifest::discover(&entry)?;
    let (root, name) = match project {
        Some(project) => (project.root, project.manifest.name),
        None => {
            let root = entry
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            let name = entry
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or("program")
                .to_string();
            (root, name)
        }
    };
    Ok(Target { root, name, entry })
}

fn targets_from_project(
    project: manifest::Project,
    package: Option<&str>,
    mode: Mode,
) -> Result<Vec<Target>, String> {
    let dirs = project.package_dirs()?;
    if dirs.is_empty() {
        return Err(format!(
            "{} has no packages (`[package]` or `[workspace].members`)",
            project.root.display()
        ));
    }
    let mut packages = Vec::new();
    let mut available = Vec::new();
    for dir in dirs {
        let member = manifest::load(&dir)?;
        if !member.manifest.has_package {
            return Err(format!(
                "workspace member {} is not a package: missing [package]",
                dir.display()
            ));
        }
        if available.contains(&member.manifest.name) {
            return Err(format!(
                "two packages are named `{}`; workspace package names must be unique",
                member.manifest.name
            ));
        }
        available.push(member.manifest.name.clone());
        packages.push(member);
    }

    let selected: Vec<manifest::Project> = match package {
        Some(wanted) => {
            let found: Vec<_> = packages
                .into_iter()
                .filter(|member| member.manifest.name == wanted)
                .collect();
            if found.is_empty() {
                return Err(format!(
                    "no package named `{wanted}` in this workspace; packages: {}",
                    available.join(", ")
                ));
            }
            found
        }
        None => match mode {
            Mode::Run => {
                if project.manifest.has_package {
                    packages
                        .into_iter()
                        .filter(|member| member.root == project.root)
                        .collect()
                } else {
                    return Err(format!(
                        "workspace has {} packages; select one with `-p/--package <name>`: {}",
                        available.len(),
                        available.join(", ")
                    ));
                }
            }
            Mode::All { all: true } => packages,
            Mode::All { all: false } => {
                if project.manifest.has_package {
                    packages
                        .into_iter()
                        .filter(|member| member.root == project.root)
                        .collect()
                } else {
                    packages
                }
            }
        },
    };

    let mut targets = Vec::new();
    for member in selected {
        let name = member.manifest.name.clone();
        let entry = member.entry();
        if !entry.is_file() {
            return Err(format!(
                "entry module {} does not exist (package `{name}`)",
                entry.display()
            ));
        }
        targets.push(Target {
            root: member.root,
            name,
            entry,
        });
    }
    Ok(targets)
}

fn absolute(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .map_err(|e| format!("cannot determine current directory: {e}"))
    }
}

fn new_project(path: &Path) -> Result<ExitCode, String> {
    if path.exists() {
        return Err(format!("destination `{}` already exists", path.display()));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid package path `{}`", path.display()))?;
    std::fs::create_dir_all(path).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
    let manifest = format!(
        "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nedition = \"2024\"\nentry = \"main.xulo\"\n",
        toml_escape(name)
    );
    write(&path.join(manifest::MANIFEST_FILE), &manifest)?;
    write(
        &path.join(manifest::DEFAULT_ENTRY),
        "fn main() {\n    print(\"Hello from xupm!\")\n}\n",
    )?;
    println!("created package `{name}` at {}", path.display());
    Ok(ExitCode::SUCCESS)
}

fn build(
    file: Option<&Path>,
    package: Option<&str>,
    all: bool,
    release: bool,
) -> Result<ExitCode, String> {
    let targets = resolve_targets(file, package, Mode::All { all })?;
    let mut failed = false;
    // Product builds use LLVM. Hosts without static LLVM (the stage-0 macOS
    // and Windows previews) select Cranelift explicitly instead of relying on
    // the warning-producing fallback.
    #[cfg(feature = "llvm")]
    let backend = AotBackend::Llvm;
    #[cfg(not(feature = "llvm"))]
    let backend = AotBackend::Cranelift;
    let profile = if release {
        BuildProfile::Release
    } else {
        BuildProfile::Debug
    };
    for target in &targets {
        let output = target
            .root
            .join(".xupm")
            .join("build")
            .join(artifact_name(target));
        match xulo_compiler::aot::build_executable_with_profile(
            &target.entry,
            &output,
            backend,
            profile,
        ) {
            Ok(build) => {
                print_warnings(&build.warnings);
                println!(
                    "build successful: {} (backend: {}, profile: {})",
                    build.executable.display(),
                    build.backend.name(),
                    if release { "release" } else { "debug" },
                );
            }
            Err(err) => {
                print_compile_error(&err, &target.entry);
                failed = true;
            }
        }
    }
    Ok(exit_status(failed))
}

fn run_project(file: Option<&Path>, package: Option<&str>) -> Result<ExitCode, String> {
    let targets = resolve_targets(file, package, Mode::Run)?;
    let [target] = targets.as_slice() else {
        return Err("run requires exactly one package".to_string());
    };
    Ok(xulo_cli::cli::run_file(&target.entry))
}

fn test_project(file: Option<&Path>, package: Option<&str>, all: bool) -> Result<ExitCode, String> {
    let targets = resolve_targets(file, package, Mode::All { all })?;
    let mut failed = false;
    for target in &targets {
        if xulo_cli::cli::test_file(&target.entry) != ExitCode::SUCCESS {
            failed = true;
        }
    }
    Ok(exit_status(failed))
}

fn check_project(
    file: Option<&Path>,
    package: Option<&str>,
    all: bool,
) -> Result<ExitCode, String> {
    let targets = resolve_targets(file, package, Mode::All { all })?;
    let mut failed = false;
    for target in &targets {
        if xulo_cli::cli::check_file(&target.entry) != ExitCode::SUCCESS {
            failed = true;
        }
    }
    Ok(exit_status(failed))
}

fn exit_status(failed: bool) -> ExitCode {
    if failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn print_warnings(warnings: &[XuloError]) {
    for warning in warnings {
        let source = warning
            .file
            .as_ref()
            .and_then(|file| std::fs::read_to_string(file).ok());
        eprintln!(
            "{}",
            xulo_core::diagnostics::render(warning, source.as_deref())
        );
    }
}

fn print_compile_error(err: &XuloError, entry: &Path) {
    let file = err.file.clone().unwrap_or_else(|| entry.to_path_buf());
    let source = std::fs::read_to_string(&file).unwrap_or_default();
    let err = err.clone().with_file(file);
    eprintln!("{}", xulo_core::diagnostics::render(&err, Some(&source)));
}

fn artifact_name(target: &Target) -> String {
    let sanitized: String = target
        .name
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let name = if sanitized.is_empty() {
        "program".to_string()
    } else {
        sanitized
    };
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name
    }
}

fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn write(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn not_implemented(command: &str) -> Result<ExitCode, String> {
    Err(format!("`{command}` is not implemented yet"))
}
