//! End-to-end framework tests: a `.xulo` file is executed and its `View` is
//! rendered through the terminal backend.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static DIR_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Write `src` to a unique temp `.xulo` file and return its path.
fn temp_program(src: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "xulo-framework-test-{}-{}",
        std::process::id(),
        DIR_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.xulo");
    std::fs::write(&path, src).unwrap();
    path
}

fn cleanup(path: &Path) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::remove_dir_all(dir);
    }
}

/// Write `src` into the temp dir of `path`.
fn write_sibling(path: &Path, name: &str, src: &str) {
    std::fs::write(path.parent().unwrap().join(name), src).unwrap();
}

const UI_SRC: &str = r##"
import { Screen, VStack, HStack, Text, Button, Input } from "@xulo/ui"

fn Counter(): View {
    @State let count: number = 0
    @Effect fn() { print("mounted") }
    VStack(spacing: 1) {
        Text("Count: " + str(count))
        HStack(spacing: 2) {
            Button(onClick: fn() { count = count + 1 }) { Text("+") }
            Button(onClick: fn() { count = count - 1 }) { Text("-") }
        }
    }
}

fn main(): View {
    Screen(orientation: "portrait", backgroundColor: "#101010") {
        Counter()
        Text("Hello, world!")
    }
}
"##;

#[test]
fn renders_program_view_through_terminal_backend() {
    let path = temp_program(UI_SRC);
    let result = xulo_framework::render_to_string(&path, xulo_framework::Backend::Terminal);
    cleanup(&path);
    let rendered = result.expect("program executes and renders");
    // print() output goes to the execute result, not the rendered grid.
    assert!(!rendered.contains("mounted"));
    let expected = "Count: 0\n\n┌─┐  ┌─┐\n│+│  │-│\n└─┘  └─┘\nHello, world!";
    assert_eq!(rendered.trim_end(), expected);
}

#[cfg(feature = "webview")]
#[test]
fn renders_program_view_to_webview_html() {
    let path = temp_program(UI_SRC);
    let result = xulo_framework::render_to_string(&path, xulo_framework::Backend::Webview);
    cleanup(&path);
    let html = result.expect("program executes and renders to html");
    assert!(html.contains("<canvas id=\"screen\">"));
    // The page runs the embedded wasm layout engine: raw instantiation + ABI.
    assert!(
        html.contains("WebAssembly.instantiate(bytes, {})"),
        "wasm engine bootstraps"
    );
    assert!(
        html.contains("wasm.xulo_layout(ptr, enc.length, W, H)"),
        "tree to wasm"
    );
    assert!(
        html.contains("wasm.xulo_hit_test(x, y)"),
        "clicks hit-tested in wasm"
    );
    // The serialized widget tree reaches the page (text + buttons present).
    assert!(html.contains("\"text\":\"Count: 0\""), "tree contains text");
    assert!(html.contains("\"Button\""));
    // print() output does not leak into the page.
    assert!(!html.contains("mounted"));
}

#[test]
fn execute_captures_output_and_view() {
    let path = temp_program(UI_SRC);
    let result = xulo_framework::execute(&path);
    cleanup(&path);
    let result = result.expect("program executes");
    assert_eq!(result.output, vec!["mounted"]);
    assert!(result.root_view.is_some());
}

#[test]
fn local_module_imports_wire_into_view() {
    // A multi-module program: the entry imports from a local module whose
    // exported function feeds the View. Exercised so module wiring (exports
    // collected per module) is covered end to end.
    let path = temp_program(
        r#"import { label } from "./lib.xulo"
fn main(): View {
    VStack { Text(label()) }
}
"#,
    );
    write_sibling(
        &path,
        "lib.xulo",
        "pub fn label(): string { \"from lib\" }\n",
    );
    let result = xulo_framework::render_to_string(&path, xulo_framework::Backend::Terminal);
    cleanup(&path);
    let rendered = result.expect("multi-module program executes and renders");
    assert_eq!(rendered.trim_end(), "from lib");
}

#[test]
fn clicking_button_rerenders_with_updated_state() {
    // The interactive session: clicking a button runs its `onClick` (which
    // mutates `@State`), then re-renders — the next frame shows the new count.
    let path = temp_program(
        r#"import { Button, Text, VStack } from "@xulo/ui"
fn Counter(): View {
    @State let count: number = 0
    VStack(spacing: 1) {
        Text("Count: " + str(count))
        Button(onClick: fn() { count = count + 1 }) { Text("+") }
    }
}
fn main(): View {
    Counter()
}
"#,
    );
    use xulo_renderer_terminal::{render_plain, CharMetrics, TerminalSize};
    use xulo_ui::{Size, UiContext};

    let surface = Size {
        width: 40,
        height: 12,
    };
    let frame: xulo_framework::FrameBuilder = Box::new(|root, surface| {
        let ctx = UiContext::new(surface, Box::new(CharMetrics));
        let placed = ctx.layout(root);
        let mut ops = Vec::new();
        xulo_ui::layout::paint(&placed, &ctx.theme, &mut ops);
        let mut buttons = Vec::new();
        xulo_framework::collect_button_rects(&placed, &mut buttons);
        let mut inputs = Vec::new();
        xulo_ui::collect_input_rects(&placed, &mut inputs);
        (ops, buttons, inputs)
    });
    let mut ui = xulo_framework::ReactiveUi::load(&path, surface, Some(frame)).unwrap();
    cleanup(&path);

    let size = TerminalSize { cols: 40, rows: 12 };
    assert!(render_plain(&ui.ops(), size).contains("Count: 0"));

    let (bx, by) = (ui.buttons[0].x, ui.buttons[0].y);
    ui.handle_click(bx, by).unwrap();
    assert!(
        render_plain(&ui.ops(), size).contains("Count: 1"),
        "click increments state across re-render"
    );

    ui.handle_click(bx, by).unwrap();
    assert!(render_plain(&ui.ops(), size).contains("Count: 2"));
}

#[test]
fn text_color_prop_becomes_widget_color() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use xulo_runtime::value::Value;

    let node = Value::Object(Rc::new(RefCell::new(vec![
        ("name".into(), Value::String(Rc::from("Text"))),
        (
            "props".into(),
            Value::Object(Rc::new(RefCell::new(vec![
                ("0".into(), Value::String(Rc::from("hello"))),
                ("color".into(), Value::String(Rc::from("#ff0000"))),
            ]))),
        ),
    ])));
    let widget = xulo_framework::widget_from_value(&node);
    assert_eq!(
        widget,
        xulo_ui::Widget::Text {
            text: "hello".into(),
            color: Some(xulo_ui::Color::new(255, 0, 0)),
        }
    );
}

#[test]
fn non_view_main_is_rejected_for_rendering() {
    let path = temp_program("fn main(): number { 42 }");
    let result = xulo_framework::render_to_string(&path, xulo_framework::Backend::Terminal);
    cleanup(&path);
    let err = result.expect_err("rendering a non-View main fails");
    assert!(
        err.message.contains("did not produce a `View`"),
        "{}",
        err.message
    );
}
