use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use clap::{Parser, Subcommand};

use crate::diagnostics;
use crate::error::XuloError;

#[derive(Parser)]
#[command(
    name = "xulo",
    version,
    about = "Xulo compiler: .xulo files -> JavaScript, run via Node.js"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Compile and run a .xulo file with node
    Run { file: PathBuf },
    /// Compile a .xulo file to a JavaScript file
    Build {
        file: PathBuf,
        /// Output .js path (defaults to the input stem + .js in the current dir)
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Only run lexical + syntax + semantic checks
    Check { file: PathBuf },
    /// Format a .xulo file (not yet implemented)
    Fmt { file: PathBuf },
    /// Start an interactive REPL (not yet implemented)
    Repl,
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();
    run_command(cli.command)
}

fn run_command(command: Commands) -> ExitCode {
    match command {
        Commands::Run { file } => run_file(&file),
        Commands::Build { file, out } => build_file(&file, out),
        Commands::Check { file } => check_file(&file),
        Commands::Fmt { file } => {
            eprintln!("xulo fmt is not implemented yet (no formatter for `{}`)", file.display());
            ExitCode::from(1)
        }
        Commands::Repl => {
            eprintln!("xulo repl is not implemented yet");
            ExitCode::from(1)
        }
    }
}

fn run_file(file: &Path) -> ExitCode {
    let js = match compile_to_js(file) {
        Ok(js) => js,
        Err(code) => return code,
    };

    let tmp = temp_js_path();
    if let Err(e) = write_js(&tmp, &js) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }

    let status = Command::new("node")
        .arg(&tmp)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();

    let _ = std::fs::remove_file(&tmp);

    match status {
        Ok(output) => ExitCode::from(output.code().unwrap_or(1) as u8),
        Err(e) => {
            eprintln!("failed to run node: {e}");
            ExitCode::from(1)
        }
    }
}

fn build_file(file: &Path, out: Option<PathBuf>) -> ExitCode {
    let js = match compile_to_js(file) {
        Ok(js) => js,
        Err(code) => return code,
    };
    // An ESM `import` at the top of the bundle requires a `.mjs` extension
    // (or a `"type": "module"` package.json) to run under Node.
    let has_external_imports = js.lines().any(|l| l.starts_with("import "));
    let out = out.unwrap_or_else(|| {
        let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned());
        let ext = if has_external_imports { "mjs" } else { "js" };
        PathBuf::from(format!("{}.{ext}", stem.unwrap_or_else(|| "out".into())))
    });
    if let Err(e) = write_js(&out, &js) {
        eprintln!("{e}");
        return ExitCode::from(1);
    }
    println!("wrote {}", out.display());
    ExitCode::SUCCESS
}

fn check_file(file: &Path) -> ExitCode {
    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {}: {e}", file.display());
            return ExitCode::from(1);
        }
    };
    match crate::module::compile_file(file) {
        Ok(_) => {
            println!("no errors");
            ExitCode::SUCCESS
        }
        Err(err) => {
            print_compile_error(&err, &source, file);
            ExitCode::from(1)
        }
    }
}

fn compile_to_js(file: &Path) -> Result<String, ExitCode> {
    match crate::module::compile_file(file) {
        Ok(js) => Ok(js),
        Err(err) => {
            let src_file = err.file.clone().unwrap_or_else(|| file.to_path_buf());
            let source = std::fs::read_to_string(&src_file).unwrap_or_default();
            print_compile_error(&err, &source, &src_file);
            Err(ExitCode::from(1))
        }
    }
}

fn print_compile_error(err: &XuloError, source: &str, file: &Path) {
    let err = err.clone().with_file(file.to_path_buf());
    eprintln!("{}", diagnostics::render(&err, Some(source)));
}

fn write_js(path: &Path, js: &str) -> Result<(), String> {
    let mut f = std::fs::File::create(path)
        .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    f.write_all(js.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn temp_js_path() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("xulo_{}_{}.mjs", std::process::id(), nanos))
}