use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};
use xulo_core::error::XuloError;

use crate::manifest::{self, Manifest};

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
    Build { file: Option<PathBuf> },
    /// Run the project's entry module
    Run { file: Option<PathBuf> },
    /// Type-check the project
    Check { file: Option<PathBuf> },
    /// Run the project's tests
    Test { file: Option<PathBuf> },
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
        Commands::Build { file } => build(file.as_deref()),
        Commands::Run { file } => run_project(file.as_deref()),
        Commands::Test { file } => test_project(file.as_deref()),
        Commands::Init => not_implemented("init"),
        Commands::Add { .. } => not_implemented("add"),
        Commands::Remove { .. } => not_implemented("remove"),
        Commands::Fetch => not_implemented("fetch"),
        Commands::Check { .. } => not_implemented("check"),
        Commands::Update => not_implemented("update"),
        Commands::Tree => not_implemented("tree"),
        Commands::Clean => not_implemented("clean"),
    }
}

/// The entry module and project root a command operates on. An explicit file
/// is used as the entry as-is; without one the entry comes from the manifest
/// of the nearest project.
struct Target {
    root: PathBuf,
    manifest: Option<Manifest>,
    entry: PathBuf,
}

fn resolve(file: Option<&Path>) -> Result<Target, String> {
    match file {
        Some(file) => {
            let entry = absolute(file)?;
            if !entry.is_file() {
                return Err(format!("cannot find {}", entry.display()));
            }
            let project = manifest::discover(&entry)?;
            let root = match &project {
                Some(project) => project.root.clone(),
                None => entry
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from(".")),
            };
            Ok(Target {
                root,
                manifest: project.map(|project| project.manifest),
                entry,
            })
        }
        None => {
            let cwd = std::env::current_dir()
                .map_err(|e| format!("cannot determine current directory: {e}"))?;
            let project = manifest::discover(&cwd)?.ok_or_else(|| {
                "could not find `xulo.toml` in this directory or any parent".to_string()
            })?;
            let entry = project.entry();
            if !entry.is_file() {
                return Err(format!("entry module {} does not exist", entry.display()));
            }
            Ok(Target {
                root: project.root,
                manifest: Some(project.manifest),
                entry,
            })
        }
    }
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

fn build(file: Option<&Path>) -> Result<ExitCode, String> {
    let target = resolve(file)?;
    let output = target
        .root
        .join(".xupm")
        .join("build")
        .join(artifact_name(&target));
    match xulo_compiler::aot::build_executable(&target.entry, &output) {
        Ok(build) => {
            print_warnings(&build.warnings);
            println!("build successful: {}", build.executable.display());
            Ok(ExitCode::SUCCESS)
        }
        Err(err) => {
            print_compile_error(&err, &target.entry);
            Ok(ExitCode::FAILURE)
        }
    }
}

fn run_project(file: Option<&Path>) -> Result<ExitCode, String> {
    let target = resolve(file)?;
    Ok(xulo_cli::cli::run_file(&target.entry))
}

fn test_project(file: Option<&Path>) -> Result<ExitCode, String> {
    let target = resolve(file)?;
    Ok(xulo_cli::cli::test_file(&target.entry))
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
    let raw = target
        .manifest
        .as_ref()
        .map(|manifest| manifest.name.as_str())
        .or_else(|| target.entry.file_stem().and_then(|stem| stem.to_str()))
        .unwrap_or("program");
    let sanitized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "program".to_string()
    } else {
        sanitized
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
