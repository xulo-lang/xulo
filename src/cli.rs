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
    /// Format a .xulo file in place (comments are not preserved)
    Fmt { file: PathBuf },
    /// Start an interactive REPL
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
        Commands::Fmt { file } => fmt_file(&file),
        Commands::Repl => repl(),
    }
}

fn run_file(file: &Path) -> ExitCode {
    let (js, warnings) = match compile_to_js(file) {
        Ok(out) => out,
        Err(code) => return code,
    };
    print_warnings(&warnings);

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
    let (js, warnings) = match compile_to_js(file) {
        Ok(out) => out,
        Err(code) => return code,
    };
    print_warnings(&warnings);
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

fn fmt_file(file: &Path) -> ExitCode {    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {}: {e}", file.display());
            return ExitCode::from(1);
        }
    };
    let formatted = match crate::formatter::format(&source) {
        Ok(f) => f,
        Err(e) => {
            let err = e.clone().with_file(file.to_path_buf());
            eprintln!("{}", diagnostics::render(&err, Some(&source)));
            return ExitCode::from(1);
        }
    };
    if formatted != source {
        match write_js(file, &formatted) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::from(1);
            }
        }
    }
    println!("formatted {}", file.display());
    ExitCode::SUCCESS
}

fn check_file(file: &Path) -> ExitCode {    match crate::module::compile_file(file) {
        Ok((_, warnings)) => {
            print_warnings(&warnings);
            println!("no errors");
            ExitCode::SUCCESS
        }
        Err(err) => {
            let src_file = err.file.clone().unwrap_or_else(|| file.to_path_buf());
            let source = std::fs::read_to_string(&src_file).unwrap_or_default();
            print_compile_error(&err, &source, &src_file);
            ExitCode::from(1)
        }
    }
}

fn repl() -> ExitCode {
    use std::io::{BufRead, Write};
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    println!("xulo REPL — enter code; run with an empty line or `run`, `exit` to quit");
    let mut entry = String::new();
    let mut session = String::new();
    loop {
        print!("{}", if entry.is_empty() { "xulo> " } else { "...> " });
        let _ = stdout.flush();
        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("read error: {e}");
                break;
            }
        }
        let trimmed = line.trim();
        if entry.is_empty() && trimmed.is_empty() {
            break;
        }
        if entry.is_empty() && matches!(trimmed, "exit" | ":quit" | ".exit" | ":q") {
            break;
        }
        if entry.is_empty() && matches!(trimmed, "clear" | ":reset") {
            session.clear();
            continue;
        }
        entry.push_str(&line);
        if unbalanced(&entry) {
            continue;
        }
        let run_now =
            trimmed.is_empty() || trimmed == "run" || entry.trim_end().ends_with('}');
        if !run_now {
            continue;
        }
        session.push_str(&entry);
        let raw = session.trim_start();
        let has_main = raw
            .split('\n')
            .any(|l| l.trim_start().starts_with("fn main"));
        let result = if has_main {
            compile_source(&session)
        } else {
            compile_source(&format!("fn main() {{\n{session}\n}}\n"))
        };
        match result {
            Ok((js, warnings)) => {
                print_warnings(&warnings);
                run_node(&js);
            }
            Err(()) => {
                // Roll back the failed entry so it is not re-run later.
                session.truncate(session.len().saturating_sub(entry.len()));
            }
        }
        entry.clear();
    }
    ExitCode::SUCCESS
}

fn unbalanced(src: &str) -> bool {
    let mut stack: Vec<char> = Vec::new();
    let mut in_str: Option<char> = None;
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(q) = in_str {
            if c == '\\' {
                chars.next();
                continue;
            }
            if c == q {
                in_str = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => in_str = Some(c),
            '(' | '[' | '{' => stack.push(c),
            ')' | ']' | '}' => {
                let open = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                if stack.pop() != Some(open) {
                    return true;
                }
            }
            _ => {}
        }
    }
    in_str.is_some() || !stack.is_empty()
}

fn run_node(js: &str) {
    let tmp = temp_js_path();
    if let Err(e) = write_js(&tmp, js) {
        eprintln!("{e}");
        return;
    }
    let status = Command::new("node")
        .arg(&tmp)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
    let _ = std::fs::remove_file(&tmp);
    if let Err(e) = status {
        eprintln!("failed to run node: {e}");
    }
}

fn compile_source(buffer: &str) -> Result<(String, Vec<(std::path::PathBuf, String)>), ()> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("xulo_repl_{}_{}.xulo", std::process::id(), nanos));
    if let Err(e) = write_js(&path, buffer) {
        eprintln!("{e}");
        return Err(());
    }
    let result = crate::module::compile_file(&path);
    let _ = std::fs::remove_file(&path);
    match result {
        Ok(out) => Ok(out),
        Err(err) => {
            let src_file = err.file.clone().unwrap_or(path);
            let source = std::fs::read_to_string(&src_file).unwrap_or_default();
            print_compile_error(&err, &source, &src_file);
            Err(())
        }
    }
}

fn compile_to_js(file: &Path) -> Result<(String, Vec<(std::path::PathBuf, String)>), ExitCode> {    match crate::module::compile_file(file) {
        Ok((js, warnings)) => Ok((js, warnings)),
        Err(err) => {
            let src_file = err.file.clone().unwrap_or_else(|| file.to_path_buf());
            let source = std::fs::read_to_string(&src_file).unwrap_or_default();
            print_compile_error(&err, &source, &src_file);
            Err(ExitCode::from(1))
        }
    }
}

fn print_warnings(warnings: &[(std::path::PathBuf, String)]) {
    for (file, message) in warnings {
        eprintln!("warning: {}: {message}", file.display());
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