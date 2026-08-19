use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use clap::{Parser, Subcommand};

use xulo_core::ast::ImportSpec;
use xulo_core::diagnostics;
use xulo_core::error::XuloError;
use xulo_runtime::interpreter::{Interpreter, RunError};
use xulo_runtime::value::Value;

#[derive(Parser)]
#[command(
    name = "xulo",
    version,
    about = "Xulo: compile and run .xulo files via the native Rust interpreter (default) or through JavaScript + Node"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Compile and run a .xulo file in the native Rust interpreter (the
    /// default); --js compiles to JavaScript and runs it with node
    Run {
        file: PathBuf,
        /// Compile to JavaScript and run via node instead of the native interpreter
        #[arg(long, conflicts_with = "native")]
        js: bool,
        /// Run in the native Rust interpreter (the default) — kept for backward
        /// compatibility with scripts and tests written before the default flipped
        #[arg(long, hide = true, conflicts_with = "js")]
        native: bool,
    },
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
    // ANSI color only when the stream is a terminal and `NO_COLOR` is unset
    // (diagnostics::use_color also honors `NO_COLOR`).
    let stderr_is_tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    xulo_core::diagnostics::use_color(stderr_is_tty);
    let cli = Cli::parse();
    run_command(cli.command)
}

fn run_command(command: Commands) -> ExitCode {
    match command {
        Commands::Run { file, native, js } => run_file(&file, js, native),
        Commands::Build { file, out } => build_file(&file, out),
        Commands::Check { file } => check_file(&file),
        Commands::Fmt { file } => fmt_file(&file),
        Commands::Repl => repl(),
    }
}

/// Execute a `.xulo` file. The native Rust interpreter is the default; only
/// `--js` (and not the deprecated `--native`) routes through the node path.
fn run_file(file: &Path, js: bool, native: bool) -> ExitCode {
    if js && !native {
        return node_run(file);
    }
    native_run(file)
}

/// Compile a `.xulo` file to JavaScript and run the result under node.
fn node_run(file: &Path) -> ExitCode {
    let (js, warnings) = match compile_to_js(file) {
        Ok(out) => out,
        Err(code) => return code,
    };
    print_warnings(&warnings, None);

    // Write the temporary module next to the source so ESM bare specifiers
    // (e.g. `@xulo/ui`) resolve against node_modules walking up from there.
    let tmp = temp_js_path(file.parent());
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

/// Run a `.xulo` file natively: lex -> parse -> semantic check -> the Rust
/// tree-walking interpreter (no Node.js). Local imports are loaded, analyzed in
/// dependency order, and executed module by module; external (non-`type`-only)
/// imports are rejected.
fn native_run(file: &Path) -> ExitCode {
    let mut loaded = match xulo_compiler::module::load(file) {
        Ok(l) => l,
        Err(err) => {
            let src_file = err.file.clone().unwrap_or_else(|| file.to_path_buf());
            let source = std::fs::read_to_string(&src_file).unwrap_or_default();
            print_compile_error(&err, &source, &src_file);
            return ExitCode::from(1);
        }
    };
    let warnings = match xulo_compiler::module::analyze(&mut loaded) {
        Ok(w) => w,
        Err(err) => {
            let src_file = err.file.clone().unwrap_or_else(|| file.to_path_buf());
            let source = std::fs::read_to_string(&src_file).unwrap_or_default();
            print_compile_error(&err, &source, &src_file);
            return ExitCode::from(1);
        }
    };
    xulo_compiler::module::apply_trait_dispatch(&mut loaded);
    print_warnings(&warnings, None);

    // External packages (`@xulo/ui`) have no native values; their imported
    // names are bound to `null` placeholders so UI components still resolve by
    // name (they build the props-object shape) and headless programs can run.
    // Expression use of a missing external value fails only if a program
    // computes with it.
    let placeholders: Vec<(String, Value)> = loaded
        .external_imports
        .iter()
        .filter(|i| !i.type_only)
        .flat_map(|i| import_binding_names(&i.spec))
        .map(|name| (name, Value::Null))
        .collect();

    let interp = Interpreter::new();
    let mut export_maps: Vec<HashMap<String, Value>> = Vec::with_capacity(loaded.modules.len());

    for (idx, module) in loaded.modules.iter().enumerate() {
        let mut imports: Vec<(String, Value)> = Vec::new();
        let mut resolve_err: Option<String> = None;
        for binding in &module.imports {
            if binding.type_only {
                continue;
            }
            match resolve_import(binding, &export_maps, &loaded) {
                Ok(mut pairs) => imports.append(&mut pairs),
                Err(msg) => {
                    resolve_err = Some(msg);
                    break;
                }
            }
        }
        if let Some(msg) = resolve_err {
            eprintln!("error: {msg}");
            return ExitCode::from(1);
        }
        imports.extend_from_slice(&placeholders);
        let run_main = idx == loaded.entry && module.has_main;
        match interp.exec_module(&module.program, &imports, run_main) {
            Ok(exports) => {
                let mut map = HashMap::new();
                for (name, value) in exports.bindings {
                    map.insert(name, value);
                }
                export_maps.push(map);
            }
            Err(RunError::Err(err)) => {
                let src_file = err.file.clone().unwrap_or_else(|| module.file.clone());
                let source = std::fs::read_to_string(&src_file).unwrap_or_default();
                print_compile_error(&err, &source, &src_file);
                return ExitCode::from(1);
            }
            Err(RunError::Throw(v)) => {
                eprintln!("runtime error: uncaught exception: {}", v.format());
                return ExitCode::from(1);
            }
        }
    }
    let out = interp.take_output();
    if !out.is_empty() {
        println!("{}", out.join("\n"));
    }
    ExitCode::SUCCESS
}

/// Resolve one import binding against the exports of its already-executed
/// target module, returning the `(local name, value)` pairs to bind.
/// The binding names an `import` statement introduces (namespace or named
/// bindings with their aliases); a bare side-effect import binds nothing.
fn import_binding_names(spec: &ImportSpec) -> Vec<String> {
    match spec {
        ImportSpec::Namespace(ns) => vec![ns.clone()],
        ImportSpec::Named(bindings) => bindings
            .iter()
            .map(|(name, alias)| alias.clone().unwrap_or_else(|| name.clone()))
            .collect(),
        ImportSpec::Bare => Vec::new(),
    }
}

fn resolve_import(
    binding: &xulo_compiler::module::ImportBinding,
    export_maps: &[HashMap<String, Value>],
    loaded: &xulo_compiler::module::LoadedModules,
) -> Result<Vec<(String, Value)>, String> {
    let target = &loaded.modules[binding.target];
    let readable = target.file.display();
    let mut out = Vec::new();
    match &binding.spec {
        ImportSpec::Bare => {}
        ImportSpec::Namespace(ns) => {
            let fields: Vec<(String, Value)> = export_maps[binding.target]
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            out.push((
                ns.clone(),
                Value::Object(std::rc::Rc::new(std::cell::RefCell::new(fields))),
            ));
        }
        ImportSpec::Named(names) => {
            for (name, alias) in names {
                let local = alias.clone().unwrap_or_else(|| name.clone());
                match export_maps[binding.target].get(name) {
                    Some(value) => out.push((local, value.clone())),
                    None => {
                        return Err(format!(
                            "module `{readable}` has no runtime export named `{name}`"
                        ));
                    }
                }
            }
        }
    }
    Ok(out)
}

fn build_file(file: &Path, out: Option<PathBuf>) -> ExitCode {
    let (js, warnings) = match compile_to_js(file) {
        Ok(out) => out,
        Err(code) => return code,
    };
    print_warnings(&warnings, None);
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

fn fmt_file(file: &Path) -> ExitCode {
    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot read {}: {e}", file.display());
            return ExitCode::from(1);
        }
    };
    let formatted = match xulo_ide::format::format(&source) {
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

fn check_file(file: &Path) -> ExitCode {
    match xulo_compiler::module::compile_file(file) {
        Ok((_, warnings)) => {
            print_warnings(&warnings, None);
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

/// Keywords, type names, and REPL commands offered by `Tab` completion.
const REPL_CANDIDATES: &[&str] = &[
    "exit", "clear", "run", "fn", "let", "const", "return", "if", "else", "for", "in", "while",
    "match", "print", "type", "enum", "null", "true", "false", "and", "or", "await", "async",
    "try", "catch", "throw", "import", "pub", "use", "from", "as",
];

/// Completion/history helper for the REPL line editor.
#[derive(Default)]
struct ReplHelper;

impl rustyline::Helper for ReplHelper {}

impl rustyline::hint::Hinter for ReplHelper {
    type Hint = String;

    fn hint(&self, _line: &str, _pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        None
    }
}

impl rustyline::highlight::Highlighter for ReplHelper {}

impl rustyline::validate::Validator for ReplHelper {}

impl rustyline::completion::Completer for ReplHelper {
    type Candidate = rustyline::completion::Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<rustyline::completion::Pair>)> {
        let word_start = line[..pos]
            .rfind(char::is_whitespace)
            .map(|i| i + 1)
            .unwrap_or(0);
        let prefix = &line[word_start..pos];
        let candidates = REPL_CANDIDATES
            .iter()
            .filter(|c| c.starts_with(prefix))
            .map(|c| rustyline::completion::Pair {
                display: c.to_string(),
                replacement: c.to_string(),
            })
            .collect();
        Ok((word_start, candidates))
    }
}

/// Where command history is persisted. `XULO_HISTORY` overrides the default
/// `~/.xulo_history` when set.
fn history_path() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("XULO_HISTORY") {
        return Some(std::path::PathBuf::from(p));
    }
    std::env::var("HOME")
        .ok()
        .map(|home| std::path::Path::new(&home).join(".xulo_history"))
}

fn repl() -> ExitCode {
    let mut rl = match rustyline::Editor::<ReplHelper, rustyline::history::DefaultHistory>::new() {
        Ok(rl) => rl,
        Err(err) => {
            eprintln!("cannot start line editor: {err}");
            return ExitCode::from(1);
        }
    };
    // History is only for interactive use; skip it when stdin is piped so
    // tests and scripts do not touch (or write) the user's history file.
    let history = if std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        history_path()
    } else {
        None
    };
    if let Some(path) = &history {
        let _ = rl.load_history(path);
    }
    println!(
        "xulo REPL — native interpreter, no Node; an empty line or `run` executes, `exit` quits"
    );
    let mut entry = String::new();
    let mut session = String::new();
    loop {
        let prompt = if entry.is_empty() { "xulo> " } else { "...> " };
        match rl.readline(prompt) {
            Ok(line) => {
                // Keep the previously-accumulated lines alive until the entry
                // executes so history holds whole commands, not fragments.
                let cmd = entry.is_empty();
                if cmd && !line.trim().is_empty() {
                    let _ = rl.add_history_entry(line.as_str());
                }
                if !repl_step(&mut session, &mut entry, &line) {
                    break;
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                // Ctrl-C cancels the pending partial entry, like a shell.
                entry.clear();
            }
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(err) => {
                eprintln!("read error: {err}");
                break;
            }
        }
    }
    if let Some(path) = &history {
        let _ = rl.save_history(path);
    }
    ExitCode::SUCCESS
}

/// Handle one REPL input line. Returns `false` when the REPL should exit.
fn repl_step(session: &mut String, entry: &mut String, line: &str) -> bool {
    let trimmed = line.trim();
    if entry.is_empty() && trimmed.is_empty() {
        return false;
    }
    if entry.is_empty() && matches!(trimmed, "exit" | ":quit" | ".exit" | ":q") {
        return false;
    }
    if entry.is_empty() && matches!(trimmed, "clear" | ":reset") {
        session.clear();
        return true;
    }
    // `run` forces the pending entry to execute immediately (it is never
    // part of the code itself).
    if trimmed == "run" {
        if entry.trim().is_empty() {
            if session.is_empty() {
                return true;
            }
            repl_run(session, "");
        } else {
            let pending = entry.clone();
            entry.clear();
            if !repl_run(session, &pending) {
                entry.push_str(&pending);
            }
        }
        return true;
    }
    entry.push_str(line);
    entry.push('\n');
    if unbalanced(entry) {
        return true;
    }
    if !trimmed.is_empty() && trimmed != "run" && !entry.trim_end().ends_with('}') {
        return true;
    }
    let pending = entry.clone();
    entry.clear();
    if !repl_run(session, &pending) {
        // Compile failed: put the entry back so it can be edited and
        // re-run (the session was rolled back inside `repl_run`).
        entry.push_str(&pending);
    }
    true
}

/// Compile and run the REPL buffer natively (in-process interpreter, no Node).
/// `pending` is the freshly-typed entry to add this round. Returns `false`
/// when the entry failed at compile stage — the session has been rolled back
/// and the caller restores the entry for editing.
fn repl_run(session: &mut String, pending: &str) -> bool {
    session.push_str(pending);
    let raw = session.trim_start();
    let has_main = raw.split('\n').any(|l| has_main_decl(l.trim_start()));
    let echo = !has_main && looks_like_echo(pending);
    let compiled = if has_main {
        session.clone()
    } else {
        format!("fn main() {{\n{session}\n}}\n")
    };
    let rollback = if echo {
        let prior = &session[..session.len().saturating_sub(pending.len())];
        let echoed = format!("fn main() {{\n{}{}\n}}\n", prior, echo_wrap(pending));
        match repl_execute(&echoed, &echoed) {
            Ok(()) => false,
            // The entry was not a standalone expression; fall back to its
            // literal form so errors match what was typed.
            Err(true) => repl_execute(&compiled, &compiled).is_err(),
            // The echoed form ran but raised a runtime error: keep the entry.
            Err(false) => false,
        }
    } else {
        repl_execute(&compiled, &compiled).is_err()
    };
    if rollback {
        // Roll back the failed entry so it is not re-run later; the caller
        // restores it into the edit buffer.
        session.truncate(session.len().saturating_sub(pending.len()));
        false
    } else {
        true
    }
}

/// Lex, parse, semantic-check, and run a REPL buffer in-process. Warnings and
/// `print` output go straight to stderr/stdout. Returns `Err(true)` when the
/// buffer failed to compile (the entry must be rolled back), or `Err(false)`
/// when it compiled but raised a runtime error (the entry is kept).
fn repl_execute(buffer: &str, render_source: &str) -> Result<(), bool> {
    let tokens = match xulo_lexer::tokenize(buffer) {
        Ok(t) => t,
        Err(err) => {
            print_compile_error(&err, render_source, Path::new("<repl>"));
            return Err(true);
        }
    };
    let mut ast = match xulo_parser::parse_program(&tokens) {
        Ok(p) => p,
        Err(err) => {
            print_compile_error(&err, render_source, Path::new("<repl>"));
            return Err(true);
        }
    };
    let analysis = match xulo_semantic::analyze_with(&ast, &[], &[], &[]) {
        Ok(a) => a,
        Err(err) => {
            print_compile_error(&err, render_source, Path::new("<repl>"));
            return Err(true);
        }
    };
    print_warnings(&analysis.warnings, Some(render_source));
    let dispatch = analysis.trait_dispatch.clone();
    let concat = analysis.list_concat.clone();
    xulo_semantic::apply_trait_dispatch(&mut ast, &dispatch);
    xulo_semantic::apply_list_concat(&mut ast, &concat);
    match Interpreter::new().run(&ast) {
        Ok(out) => {
            if !out.is_empty() {
                println!("{}", out.join("\n"));
            }
            Ok(())
        }
        Err(err) => {
            print_compile_error(&err, render_source, Path::new("<repl>"));
            Err(false)
        }
    }
}

/// Does a statement line declare the `main` function? Recognizes the plain
/// and `pub` forms so the REPL does not double-wrap the session.
fn has_main_decl(line: &str) -> bool {
    line.starts_with("fn main") || line.starts_with("pub fn main")
}

/// Decide whether a REPL entry is a bare expression whose value should be
/// echoed. Declarations, definitions, control flow, and anything ending in a
/// semicolon or block brace are compiled as-is instead.
fn looks_like_echo(entry: &str) -> bool {
    let e = entry.trim();
    if e.is_empty() || e.ends_with(';') || e.ends_with('}') || e.starts_with('{') {
        return false;
    }
    for kw in [
        "let ",
        "const ",
        "fn ",
        "async fn ",
        "if ",
        "while ",
        "for ",
        "return ",
        "enum ",
        "import ",
        "from ",
        "pub ",
        "break",
        "continue",
        "@Effect ",
        "@State ",
        "View ",
    ] {
        if e.starts_with(kw) {
            return false;
        }
    }
    // An assignment (`x = 10`) must compile as a statement, not be echoed as
    // `print(x = 10)` (which is a parse error in expression position).
    if is_assignment(e) {
        return false;
    }
    !e.starts_with('=') && !e.contains('\n')
}

/// Does the entry have the shape `name = value` (a plain assignment)? Compound
/// operators (`+=`, `==`, `>=`, `=>`, `!=`) are not assignments.
fn is_assignment(entry: &str) -> bool {
    let chars: Vec<char> = entry.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c != '=' {
            continue;
        }
        // Skip `==`, `!=`, `<=`, `>=`, `=>`, `+=`, `-=`, `*=`, `/=`, `%=`.
        let left_adj = chars.get(i.wrapping_sub(1)).copied();
        let right_adj = chars.get(i + 1).copied();
        if matches!(right_adj, Some('=') | Some('>'))
            || matches!(
                left_adj,
                Some('=')
                    | Some('!')
                    | Some('<')
                    | Some('>')
                    | Some('+')
                    | Some('-')
                    | Some('*')
                    | Some('/')
                    | Some('%')
            )
        {
            continue;
        }
        return valid_assign_target(&chars[..i]);
    }
    false
}

/// Everything before `=` must be an identifier/`$` rest, optionally with
/// `.field` / `[index]` access chains (whitespace-insensitive).
fn valid_assign_target(chars: &[char]) -> bool {
    let stripped: String = chars.iter().filter(|c| !c.is_whitespace()).collect();
    let b = stripped.as_bytes();
    let mut i = 0;
    let is_ident_char = |c: u8, first: bool| {
        c == b'_' || c == b'$' || c.is_ascii_alphabetic() || (!first && c.is_ascii_digit())
    };
    if !b.first().is_some_and(|c| is_ident_char(*c, true)) {
        return false;
    }
    i += 1;
    while i < b.len() && is_ident_char(b[i], false) {
        i += 1;
    }
    loop {
        if i == b.len() {
            return true;
        }
        match b[i] {
            b'.' => {
                if i + 1 >= b.len() || !b[i + 1].is_ascii_alphabetic() && b[i + 1] != b'_' {
                    return false;
                }
                i += 2;
                while i < b.len() && is_ident_char(b[i], false) {
                    i += 1;
                }
                if i < b.len() && b[i] == b'(' {
                    return false;
                }
            }
            b'[' => {
                if i + 1 >= b.len() || b[i + 1] == b']' {
                    return false;
                }
                // `[index]`/`["key"]`: accept a matching bracket with balanced
                // non-bracket content (no nested brackets, no `(`).
                if let Some(close) = b[i + 1..].iter().position(|&c| c == b']') {
                    let inner = &b[i + 1..i + 1 + close];
                    if inner.iter().any(|&c| c == b'[' || c == b'(') {
                        return false;
                    }
                    i = i + 1 + close + 1;
                } else {
                    return false;
                }
            }
            _ => return false,
        }
    }
}

/// Wrap a single expression entry as `print(expr)` for value echo.
fn echo_wrap(entry: &str) -> String {
    format!("print({})", entry.trim())
}

fn unbalanced(src: &str) -> bool {
    let mut stack: Vec<char> = Vec::new();
    let mut in_str: Option<char> = None;
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        // A `//` line comment runs to the end of the line: brackets inside it
        // must not count as unterminated code.
        if in_str.is_none() && c == '/' && chars.peek() == Some(&'/') {
            for rest in chars.by_ref() {
                if rest == '\n' {
                    break;
                }
            }
            continue;
        }
        // A `/* ... */` block comment may span lines and contain brackets;
        // skip to the closing `*/`.
        if in_str.is_none() && c == '/' && chars.peek() == Some(&'*') {
            let mut prev = '\0';
            for rest in chars.by_ref() {
                if prev == '*' && rest == '/' {
                    break;
                }
                prev = rest;
            }
            continue;
        }
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

fn compile_to_js(file: &Path) -> Result<(String, Vec<XuloError>), ExitCode> {
    match xulo_compiler::module::compile_file(file) {
        Ok((js, warnings)) => Ok((js, warnings)),
        Err(err) => {
            let src_file = err.file.clone().unwrap_or_else(|| file.to_path_buf());
            let source = std::fs::read_to_string(&src_file).unwrap_or_default();
            print_compile_error(&err, &source, &src_file);
            Err(ExitCode::from(1))
        }
    }
}

fn print_warnings(warnings: &[XuloError], fallback_source: Option<&str>) {
    for w in warnings {
        let source = w
            .file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .or_else(|| fallback_source.map(str::to_string));
        eprintln!("{}", diagnostics::render(w, source.as_deref()));
    }
}

fn print_compile_error(err: &XuloError, source: &str, file: &Path) {
    let err = err.clone().with_file(file.to_path_buf());
    eprintln!("{}", diagnostics::render(&err, Some(source)));
}

fn write_js(path: &Path, js: &str) -> Result<(), String> {
    let mut f =
        std::fs::File::create(path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    f.write_all(js.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// A unique `.mjs` path. For `run`, place it in `src_dir` (the source's
/// directory) so node can resolve bare package specifiers from there; the
/// REPL has no source directory and falls back to the system temp dir.
fn temp_js_path(src_dir: Option<&Path>) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let name = format!("xulo_{}_{}.mjs", std::process::id(), nanos);
    match src_dir {
        Some(dir) => dir.join(name),
        None => std::env::temp_dir().join(name),
    }
}
