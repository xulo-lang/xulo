//! Orchestration: load/analyze/execute a program, then render its `View` with
//! a chosen backend. This is the eframe-like entry point for the UI stack.

use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use xulo_core::ast::{ImportSpec, Program};
use xulo_core::error::XuloError;
use xulo_runtime::interpreter::Interpreter;
use xulo_runtime::value::Value;

use crate::convert::widget_from_value;
use crate::interactive::{FrameBuilder, ReactiveUi};

/// The renderer backend to hand the layout to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    #[cfg(feature = "terminal")]
    Terminal,
    /// A native webview window (wry) that rasterizes the layout onto a canvas.
    #[cfg(feature = "webview")]
    Webview,
}

/// The outcome of executing a program.
pub struct ExecuteResult {
    /// The collected `print` lines from all executed modules.
    pub output: Vec<String>,
    /// The render tree of the entry `main` when it returns a `View`.
    pub root_view: Option<Value>,
}

/// Load, analyze, and execute `entry`, mirroring the CLI's module wiring:
/// modules run in dependency order, external (non-`type`-only) imports bind to
/// `null` placeholders, and the entry module's `main` (when it returns a
/// `View`) has its render value captured by the interpreter.
pub fn execute(entry: &Path) -> Result<ExecuteResult, XuloError> {
    let interp = Rc::new(Interpreter::new());
    let _ = execute_in(entry, &interp)?;
    Ok(ExecuteResult {
        output: interp.take_output(),
        root_view: interp.take_root_view(),
    })
}

/// Load, analyze, and execute `entry` into an existing interpreter (so the
/// caller keeps the interpreter alive for re-rendering). Returns the entry
/// program, whose `main` can later be re-invoked via
/// [`Interpreter::rerender_main`].
pub(crate) fn execute_in(entry: &Path, interp: &Rc<Interpreter>) -> Result<Program, XuloError> {
    let mut loaded = xulo_compiler::module::load(entry)?;
    let warnings = xulo_compiler::module::analyze(&mut loaded)?;
    for warning in warnings {
        let source = warning
            .file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok());
        eprintln!(
            "{}",
            xulo_core::diagnostics::render(&warning, source.as_deref())
        );
    }
    xulo_compiler::module::apply_trait_dispatch(&mut loaded);

    let placeholders: Vec<(String, Value)> = loaded
        .external_imports
        .iter()
        .filter(|i| !i.type_only)
        .flat_map(|i| import_binding_names(&i.spec))
        .map(|name| (name, Value::Null))
        .collect();

    let mut export_maps: Vec<HashMap<String, Value>> = Vec::with_capacity(loaded.modules.len());
    for (idx, module) in loaded.modules.iter().enumerate() {
        let mut imports: Vec<(String, Value)> = Vec::new();
        for binding in &module.imports {
            if binding.type_only {
                continue;
            }
            match resolve_import(binding, &export_maps, &loaded) {
                Ok(mut pairs) => imports.append(&mut pairs),
                Err(msg) => {
                    return Err(xulo_core::error::XuloError::new(
                        xulo_core::error::ErrorKind::Runtime,
                        msg,
                    ));
                }
            }
        }
        imports.extend_from_slice(&placeholders);
        let run_main = idx == loaded.entry && module.has_main;
        let exports = interp
            .exec_module(&module.program, &imports, run_main)
            .map_err(map_run_error)?;
        export_maps.push(
            exports
                .bindings
                .into_iter()
                .collect::<HashMap<String, Value>>(),
        );
    }
    Ok(loaded.modules[loaded.entry].program.clone())
}

/// Render `entry`'s `View` interactively: the terminal backend prints each
/// frame and reads button selections from stdin, the webview backend opens a
/// native window and handles real mouse clicks. Both re-render after every
/// click, preserving `@State` across renders.
pub fn run(entry: &Path, backend: Backend) -> Result<(), XuloError> {
    match backend {
        #[cfg(feature = "terminal")]
        Backend::Terminal => run_terminal_interactive(entry),
        #[cfg(feature = "webview")]
        Backend::Webview => run_webview_interactive(entry),
    }
}

/// Interactive terminal session: print the current frame, then read `1..N` to
/// click a button, `r`/empty to re-render, `q` to quit.
#[cfg(feature = "terminal")]
fn run_terminal_interactive(entry: &Path) -> Result<(), XuloError> {
    use xulo_renderer_terminal::{render_stdout, CharMetrics, TerminalSize};
    use xulo_ui::{Size, UiContext};

    let size = TerminalSize::default();
    let surface = Size {
        width: size.cols,
        height: size.rows,
    };
    let frame: FrameBuilder = Box::new(|root, surface| {
        let ctx = UiContext::new(surface, Box::new(CharMetrics));
        let ops = ctx.paint(root);
        let placed = ctx.layout(root);
        let mut buttons = Vec::new();
        xulo_ui::collect_button_rects(&placed, &mut buttons);
        (ops, buttons)
    });
    let mut ui = ReactiveUi::load(entry, surface, Some(frame))?;
    if !ui.output.is_empty() {
        println!("{}", ui.output.join("\n"));
    }
    loop {
        render_stdout(&ui.ops, size);
        let count = ui.buttons.len();
        eprint!("buttons 1..{count}, r=refresh, q=quit > ");
        let mut line = String::new();
        if std::io::stdin().read_line(&mut line).is_err() {
            break;
        }
        match line.trim() {
            "q" | "Q" => break,
            "r" | "R" | "" => {}
            number => {
                if let Ok(k) = number.parse::<usize>() {
                    if (1..=count).contains(&k) {
                        ui.click_button(k - 1)?;
                    }
                }
            }
        }
    }
    Ok(())
}

/// Interactive webview session: the page instantiates the embedded `xulo-ui`
/// wasm engine, which lays the widget tree out and draws it to a canvas. Mouse
/// clicks are hit-tested in wasm; each hit runs the button's `onClick`, the
/// tree is re-rendered, and `window.redraw(<tree-json>)` redraws it.
#[cfg(feature = "webview")]
fn run_webview_interactive(entry: &Path) -> Result<(), XuloError> {
    use std::cell::RefCell;

    use xulo_renderer_webview::{build_html, run, ClickHandler};
    use xulo_ui::Size;

    let size = webview_size();
    let ui = Rc::new(RefCell::new(ReactiveUi::load(
        entry,
        Size::default(),
        None,
    )?));
    {
        let ui = ui.borrow();
        if !ui.output.is_empty() {
            println!("{}", ui.output.join("\n"));
        }
    }
    let tree_json = serde_json::to_string(&ui.borrow().widget)
        .map_err(|e| XuloError::new(xulo_core::error::ErrorKind::Runtime, e.to_string()))?;
    let background = screen_background(&ui.borrow().widget);
    let html = build_html(&tree_json, size, crate::wasm_assets::WASM_B64, background);
    let on_click: ClickHandler = {
        let ui = ui.clone();
        Box::new(move |index| {
            let mut ui = ui.borrow_mut();
            match ui.click_button(index as usize) {
                Ok(()) => match serde_json::to_string(&ui.widget) {
                    Ok(json) => Some(format!("window.redraw({json})")),
                    Err(_) => None,
                },
                Err(err) => {
                    eprintln!("runtime error: {}", err.message);
                    None
                }
            }
        })
    };
    run(html, "xulo".into(), size, background, on_click)
        .map_err(|e| XuloError::new(xulo_core::error::ErrorKind::Runtime, e))
}

/// Render `entry`'s `View` and return it as a string (used by tests and
/// embedders). The format depends on the backend; for the webview backend this
/// is the generated page HTML.
pub fn render_to_string(entry: &Path, backend: Backend) -> Result<String, XuloError> {
    let result = execute(entry)?;
    let view = result.root_view.ok_or_else(|| {
        XuloError::new(
            xulo_core::error::ErrorKind::Runtime,
            "program did not produce a `View`; cannot render",
        )
    })?;
    render_view(&view, backend)
}

fn render_view(view: &Value, backend: Backend) -> Result<String, XuloError> {
    let root = widget_from_value(view);
    match backend {
        #[cfg(feature = "terminal")]
        Backend::Terminal => render_terminal(&root),
        #[cfg(feature = "webview")]
        Backend::Webview => webview_html(&root),
    }
}

#[cfg(feature = "webview")]
fn webview_size() -> xulo_renderer_webview::WebviewSize {
    xulo_renderer_webview::WebviewSize::new(80, 24)
}

/// Build the webview page HTML for `root`: the embedded `xulo-ui` wasm engine
/// will lay the tree out and draw it.
#[cfg(feature = "webview")]
fn webview_html(root: &xulo_ui::Widget) -> Result<String, XuloError> {
    let size = webview_size();
    let tree_json = serde_json::to_string(root)
        .map_err(|e| XuloError::new(xulo_core::error::ErrorKind::Runtime, e.to_string()))?;
    Ok(xulo_renderer_webview::build_html(
        &tree_json,
        size,
        crate::wasm_assets::WASM_B64,
        screen_background(root),
    ))
}

/// The `Screen` root's background color, or a neutral dark gray when the tree
/// has no `Screen`/background. Colors the webview window so it never shows a
/// black void while the page loads.
#[cfg(feature = "webview")]
fn screen_background(root: &xulo_ui::Widget) -> (u8, u8, u8) {
    match root {
        xulo_ui::Widget::Screen {
            background: Some(color),
            ..
        } => (color.r, color.g, color.b),
        _ => (32, 32, 32),
    }
}

#[cfg(feature = "terminal")]
fn render_terminal(root: &xulo_ui::Widget) -> Result<String, XuloError> {
    use xulo_renderer_terminal::{render_ansi, render_plain, CharMetrics, TerminalSize};
    use xulo_ui::{Size, UiContext};

    let size = TerminalSize::default();
    let colored = std::env::var_os("NO_COLOR").is_none()
        && std::io::IsTerminal::is_terminal(&std::io::stdout());
    let ctx = UiContext::new(
        Size {
            width: size.cols,
            height: size.rows,
        },
        Box::new(CharMetrics),
    );
    let ops = ctx.paint(root);
    Ok(if colored {
        render_ansi(&ops, size)
    } else {
        render_plain(&ops, size)
    })
}

/// Names an `import` statement binds (namespace or named bindings with their
/// aliases); a bare side-effect import binds nothing.
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

/// Convert a module-execution error into a printable [`XuloError`]: an
/// `Err` is already one, a thrown value becomes an uncaught-exception error.
pub(crate) fn map_run_error(e: xulo_runtime::interpreter::RunError) -> XuloError {
    match e {
        xulo_runtime::interpreter::RunError::Err(err) => err,
        xulo_runtime::interpreter::RunError::Throw(value) => XuloError::new(
            xulo_core::error::ErrorKind::Runtime,
            format!("uncaught exception: {}", value.format()),
        ),
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
