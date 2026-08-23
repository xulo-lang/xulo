use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::rc::Rc;

use clap::{Parser, Subcommand};

use xulo_core::ast::ImportSpec;
use xulo_core::diagnostics;
use xulo_core::error::XuloError;
use xulo_runtime::interpreter::{Interpreter, NativeFn, RunError};
use xulo_runtime::value::Value;

#[derive(Parser)]
#[command(
    name = "xulo",
    version,
    about = "Xulo: check and run .xulo files via the native Rust interpreter"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Compile and run a .xulo file in the native Rust interpreter
    Run {
        file: PathBuf,
        /// Render the program's `View` with a backend instead of running headless
        #[arg(long, value_enum)]
        render: Option<RenderBackend>,
    },
    /// Only run lexical + syntax + semantic checks
    Check { file: PathBuf },
    /// Format a .xulo file in place (comments are not preserved)
    Fmt { file: PathBuf },
    /// Start an interactive REPL
    Repl,
    /// Build a .xulo file to a native executable
    Build {
        /// Source file to compile
        file: PathBuf,
        /// Output file path (default: same name without extension)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

/// A renderer backend for `xulo run --render`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum RenderBackend {
    /// Render to a character-cell ANSI terminal
    Terminal,
    /// Render to a native webview window (requires the `webview` feature)
    #[cfg(feature = "webview")]
    Webview,
}

pub fn run() -> ExitCode {
    // ANSI color only when the stream is a terminal and `NO_COLOR` is unset
    // (diagnostics::use_color also honors `NO_COLOR`).
    let stderr_is_tty = std::io::IsTerminal::is_terminal(&std::io::stderr());
    xulo_core::diagnostics::use_color(stderr_is_tty);
    let cli = Cli::parse();
    match cli.command {
        Some(command) => run_command(command),
        None => repl(),
    }
}

fn run_command(command: Commands) -> ExitCode {
    match command {
        Commands::Run { file, render } => match render {
            Some(RenderBackend::Terminal) => rendered_run(&file, xulo_framework::Backend::Terminal),
            #[cfg(feature = "webview")]
            Some(RenderBackend::Webview) => rendered_run(&file, xulo_framework::Backend::Webview),
            None => native_run(&file),
        },
        Commands::Check { file } => check_file(&file),
        Commands::Fmt { file } => fmt_file(&file),
        Commands::Repl => repl(),
        Commands::Build { file, output } => build_native(&file, output),
    }
}

/// Run a `.xulo` file and render its `View` with the given backend, delegating
/// to `xulo-framework` for the load/analyze/execute/render pipeline.
fn rendered_run(file: &Path, backend: xulo_framework::Backend) -> ExitCode {
    match xulo_framework::run(file, backend) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            let src_file = err.file.clone().unwrap_or_else(|| file.to_path_buf());
            let source = std::fs::read_to_string(&src_file).unwrap_or_default();
            print_compile_error(&err, &source, &src_file);
            ExitCode::from(1)
        }
    }
}

/// Run a `.xulo` file: lex -> parse -> semantic check -> the Rust
/// tree-walking interpreter. Local imports are loaded, analyzed in dependency
/// order, and executed module by module; external (non-`type`-only) imports
/// bind their names to `null` placeholders.
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
        match write_file(file, &formatted) {
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
    let mut loaded = match xulo_compiler::module::load(file) {
        Ok(l) => l,
        Err(err) => {
            let src_file = err.file.clone().unwrap_or_else(|| file.to_path_buf());
            let source = std::fs::read_to_string(&src_file).unwrap_or_default();
            print_compile_error(&err, &source, &src_file);
            return ExitCode::from(1);
        }
    };
    match xulo_compiler::module::analyze(&mut loaded) {
        Ok(warnings) => {
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
        "Welcome to xulo v{} (native interpreter, no Node).\n\
         Type `exit` to quit; an empty line or `run` executes; Ctrl-D (Unix) / Ctrl-C twice (Windows) leave; Tab completes.",
        env!("CARGO_PKG_VERSION")
    );
    let mut entry = String::new();
    let mut session = String::new();
    // First Ctrl-C at an idle prompt hints how to leave; a second one quits
    // (on Windows there is no Ctrl-D/Eof, so Ctrl-C is the way to exit).
    let mut interrupted = false;
    loop {
        let prompt = if entry.is_empty() { "xulo> " } else { "...> " };
        match rl.readline(prompt) {
            Ok(line) => {
                interrupted = false;
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
                // Ctrl-C cancels a pending partial entry, like a shell.
                if !entry.is_empty() {
                    entry.clear();
                    interrupted = false;
                    continue;
                }
                if interrupted {
                    break;
                }
                interrupted = true;
                println!("(To exit, press Ctrl+C again or Ctrl+D or type `exit`)");
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
    let code = entry.trim_end();
    let single_line = !code.contains('\n');
    // A freshly-typed single line that leaves a construct open (dangling
    // operator, `=`, `,`, `(`, backtick, ...) is a continuation: keep reading.
    if single_line && !code.ends_with('}') && ends_with_continuation(code) {
        return true;
    }
    // Everything else is complete now: execute immediately (a single-line
    // statement/expression, a closing block, or a buffered multi-line entry).
    let pending = entry.clone();
    entry.clear();
    if !repl_run(session, &pending) {
        // Compile failed: put the entry back so it can be edited and
        // re-run (the session was rolled back inside `repl_run`).
        entry.push_str(&pending);
    }
    true
}

/// Does `code` end in a way that suggests the next line continues it?
/// Operators, `=`, `,`, `.`, `(`, `[`, `` ` `` and the like all defer the
/// entry; a plain expression such as `4 > 5` does not.
fn ends_with_continuation(code: &str) -> bool {
    let last = code.chars().last();
    matches!(
        last,
        Some(
            '=' | '+'
                | '-'
                | '*'
                | '/'
                | '%'
                | '&'
                | '|'
                | '^'
                | '<'
                | '>'
                | ','
                | '.'
                | '('
                | '['
                | '{'
                | ':'
                | '\\'
                | '`'
                | '?'
        )
    )
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
        let echoed = format!(
            "fn main() {{\n{}{}\n}}\n",
            prior,
            echo_wrap(pending, repl_color())
        );
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
    let interp = Interpreter::new();
    if repl_color() {
        interp
            .root_env()
            .borrow_mut()
            .define("repl_echo", Value::Native(repl_echo_native()), false);
    }
    match interp.run(&ast) {
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

/// Should the REPL colorize echoed values? Node does this only when stdout is
/// a terminal; honor that plus `NO_COLOR`.
fn repl_color() -> bool {
    // `CLICOLOR_FORCE` forces colors even when stdout is not a terminal (used
    // by tests and scripts); otherwise colorize only on a real terminal,
    // honoring `NO_COLOR`.
    std::env::var_os("CLICOLOR_FORCE").is_some()
        || (std::io::IsTerminal::is_terminal(&std::io::stdout())
            && std::env::var_os("NO_COLOR").is_none())
}

/// The REPL's echo builtin: like `print`, but colorizes each argument by its
/// runtime type (strings green, numbers/booleans yellow, `null` grey) so the
/// interactive result looks like `node`'s `util.inspect` output.
fn repl_echo_native() -> NativeFn {
    let colors = repl_color();
    Rc::new(
        move |interp: &Interpreter, args: &[Value]| -> Result<Value, RunError> {
            let parts: Vec<String> = args.iter().map(|v| colorize_value(v, colors)).collect();
            interp.push_output(parts.join(" "));
            Ok(Value::Null)
        },
    )
}

/// Colorize a value with the node-style palette when `colors` is enabled.
fn colorize_value(v: &Value, colors: bool) -> String {
    let plain = v.format();
    if !colors {
        return plain;
    }
    let code = match v {
        Value::String(_) => "\x1b[32m",                     // green
        Value::Number(_) | Value::Boolean(_) => "\x1b[33m", // yellow
        Value::Null => "\x1b[1m\x1b[90m",                   // bold grey
        _ => return plain,
    };
    format!("{code}{plain}\x1b[39m\x1b[22m")
}

/// Wrap a single expression entry as `print(expr)` for value echo, or as the
/// colorizing `repl_echo(expr)` when the REPL echoes with colors.
fn echo_wrap(entry: &str, colors: bool) -> String {
    let name = if colors { "repl_echo" } else { "print" };
    format!("{name}({})", entry.trim())
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

fn write_file(path: &Path, content: &str) -> Result<(), String> {
    let mut f =
        std::fs::File::create(path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    f.write_all(content.as_bytes())
        .map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Build a .xulo file to a native executable using Cranelift AOT compilation
fn build_native(file: &Path, output: Option<PathBuf>) -> ExitCode {
    let source = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read {}: {}", file.display(), e);
            return ExitCode::from(1);
        }
    };

    // 编译到 IR
    let ir = match xulo_compiler::compile_to_ir(&source, file) {
        Ok(ir) => ir,
        Err(err) => {
            let src_file = err.file.clone().unwrap_or_else(|| file.to_path_buf());
            let source = std::fs::read_to_string(&src_file).unwrap_or_default();
            print_compile_error(&err, &source, &src_file);
            return ExitCode::from(1);
        }
    };

    // 使用 Cranelift AOT 生成目标文件
    let codegen = match xulo_compiler::aot::AotCodeGen::new() {
        Ok(cg) => cg,
        Err(err) => {
            eprintln!("error: failed to initialize AOT codegen: {}", err);
            return ExitCode::from(1);
        }
    };

    let product = match codegen.compile(&ir) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("error: code generation failed: {}", err);
            return ExitCode::from(1);
        }
    };

    // 确定输出路径
    let output_path = output.unwrap_or_else(|| {
        let stem = file.file_stem().unwrap_or_default();
        PathBuf::from(stem).with_extension("")
    });

    // 获取输出目录
    let output_dir = output_path.parent().unwrap_or(Path::new("."));
    if let Err(e) = std::fs::create_dir_all(output_dir) {
        eprintln!("error: cannot create output directory: {}", e);
        return ExitCode::from(1);
    }

    // 写入 .o 文件
    let obj_path = output_path.with_extension("o");
    let obj_file = match std::fs::File::create(&obj_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("error: cannot create {}: {}", obj_path.display(), e);
            return ExitCode::from(1);
        }
    };
    if let Err(e) = product.object.write_stream(obj_file) {
        eprintln!("error: cannot write object file: {}", e);
        return ExitCode::from(1);
    }

    // 查找 libxulo_runtime.a
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().and_then(|p| p.parent()).unwrap_or(Path::new("."));
    let runtime_lib = workspace_root.join("target/release/libxulo_runtime.a");

    if !runtime_lib.exists() {
        eprintln!("error: runtime library not found at {}", runtime_lib.display());
        eprintln!("hint: run `cargo build --release -p xulo-runtime` first");
        return ExitCode::from(1);
    }

    // 使用 cc 链接
    let exe_path = output_path.with_extension("");
    let link_status = std::process::Command::new("cc")
        .arg("-no-pie")
        .arg("-o").arg(&exe_path)
        .arg(&obj_path)
        .arg(&runtime_lib)
        .arg("-lpthread")
        .arg("-lm")
        .arg("-ldl")
        .status();

    match link_status {
        Ok(status) if status.success() => {
            // 清理 .o 文件
            let _ = std::fs::remove_file(&obj_path);
            println!("build successful: {}", exe_path.display());
            println!("\nRunning compiled program:");
            println!("-------------------------");

            // 运行生成的可执行文件 (使用绝对路径)
            let abs_exe = std::fs::canonicalize(&exe_path).unwrap_or(exe_path.clone());
            match std::process::Command::new(&abs_exe).status() {
                Ok(status) => {
                    if !status.success() {
                        eprintln!("program exited with status: {}", status);
                    }
                }
                Err(e) => {
                    eprintln!("error: failed to run {}: {}", abs_exe.display(), e);
                }
            }
        }
        Ok(status) => {
            eprintln!("error: linker failed with status: {}", status);
            let _ = std::fs::remove_file(&obj_path);
            return ExitCode::from(1);
        }
        Err(e) => {
            eprintln!("error: failed to run linker: {}", e);
            eprintln!("hint: make sure 'cc' (gcc/clang) is installed");
            let _ = std::fs::remove_file(&obj_path);
            return ExitCode::from(1);
        }
    }

    ExitCode::SUCCESS
}
