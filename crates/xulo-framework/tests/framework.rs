//! End-to-end framework tests: a `.xulo` file is executed and its `View` is
//! rendered through the terminal backend.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

static DIR_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Manifest for temp programs: the UI package is a version-only dependency
/// (no local sources), so `use ui::…` resolves to the external face.
const MANIFEST_SRC: &str = r#"[dependencies]
"@xulo/ui" = { version = "1.2", alias = "ui" }
"#;

/// Write `src` to a unique temp `.xulo` file and return its path.
fn temp_program(src: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "xulo-framework-test-{}-{}",
        std::process::id(),
        DIR_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("xulo.toml"), MANIFEST_SRC).unwrap();
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
use ui::{Screen, VStack, HStack, Text, Button, Input};

fn Counter(): View {
    @State let count: Number = 0
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
    Screen(orientation: "portrait", bg: "#101010") {
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
        r#"mod lib;
use lib::label;
fn main(): View {
    VStack { Text(label()) }
}
"#,
    );
    write_sibling(
        &path,
        "lib.xulo",
        "pub fn label(): String { \"from lib\" }\n",
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
        r#"use ui::{Button, Text, VStack};
fn Counter(): View {
    @State let count: Number = 0
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
    use xulo_renderer_terminal::{CharMetrics, TerminalSize, render_plain};
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
            style: xulo_ui::StyleProps {
                color: Some(xulo_ui::Color::new(255, 0, 0)),
                ..xulo_ui::StyleProps::default()
            },
        }
    );
}

#[test]
fn non_view_main_is_rejected_for_rendering() {
    let path = temp_program("fn main(): Number { 42 }");
    let result = xulo_framework::render_to_string(&path, xulo_framework::Backend::Terminal);
    cleanup(&path);
    let err = result.expect_err("rendering a non-View main fails");
    // The run path enforces the entry-point rule (`E0609`) before executing:
    // a `main` that returns neither Unit nor View never starts.
    assert_eq!(err.code, Some("E0609"), "{}", err.message);
    assert!(
        err.message.contains("`main` must return Unit or View"),
        "{}",
        err.message
    );
}

#[test]
fn multiple_components_preserve_state_across_rerender() {
    // When one component updates its state, other components should keep theirs.
    let path = temp_program(
        r#"use ui::{Button, Text, VStack, Input};
fn Counter(): View {
    @State let count: Number = 0
    VStack(spacing: 1) {
        Text("Count: " + str(count))
        Button(onClick: fn() { count = count + 1 }) { Text("+") }
    }
}
fn Greeting(): View {
    @State let name: String = "World"
    VStack(spacing: 1) {
        Text("Hello, " + name)
        Button(onClick: fn() { name = "Xulo" }) { Text("Change Name") }
    }
}
fn main(): View {
    VStack(spacing: 2) {
        Counter()
        Greeting()
    }
}
"#,
    );
    use xulo_renderer_terminal::{CharMetrics, TerminalSize, render_plain};
    use xulo_ui::{Size, UiContext};

    let surface = Size {
        width: 60,
        height: 20,
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

    let size = TerminalSize { cols: 60, rows: 20 };
    let rendered = render_plain(&ui.ops(), size);
    println!("Initial:\n{}", rendered);
    assert!(rendered.contains("Count: 0"), "initial count should be 0");
    assert!(
        rendered.contains("Hello, World"),
        "initial greeting should be 'Hello, World'"
    );

    // Click the + button to increment Counter's state
    let (bx, by) = (ui.buttons[0].x, ui.buttons[0].y);
    ui.handle_click(bx, by).unwrap();

    let rendered = render_plain(&ui.ops(), size);
    println!("After clicking +:\n{}", rendered);
    assert!(
        rendered.contains("Count: 1"),
        "count should be 1 after click, got:\n{}",
        rendered
    );
    // Greeting should still show "Hello, World" (not reset)
    assert!(
        rendered.contains("Hello, World"),
        "greeting should still be 'Hello, World' after Counter rerender, got:\n{}",
        rendered
    );

    // Now click the "Change Name" button to update Greeting's state
    let (bx2, by2) = (ui.buttons[1].x, ui.buttons[1].y);
    ui.handle_click(bx2, by2).unwrap();

    let rendered = render_plain(&ui.ops(), size);
    println!("After clicking Change Name:\n{}", rendered);
    assert!(
        rendered.contains("Hello, Xulo"),
        "greeting should be 'Hello, Xulo' after click, got:\n{}",
        rendered
    );
    // Counter should still show 1 (not reset)
    assert!(
        rendered.contains("Count: 1"),
        "count should still be 1 after Greeting rerender, got:\n{}",
        rendered
    );
}

#[test]
fn input_change_preserves_state() {
    // Typing in an Input should update @State, and re-render should keep it.
    let path = temp_program(
        r#"use ui::{Button, Text, VStack, Input};
fn Counter(): View {
    @State let count: Number = 0
    VStack(spacing: 1) {
        Text("Count: " + str(count))
        Button(onClick: fn() { count = count + 1 }) { Text("+") }
    }
}
fn NameField(): View {
    @State let name: String = ""
    VStack(spacing: 1) {
        Input(value: $name, width: 200, placeholder: "Enter your name")
        Text("Hello, " + name)
    }
}
fn main(): View {
    VStack(spacing: 2) {
        Counter()
        NameField()
    }
}
"#,
    );
    use xulo_renderer_terminal::{CharMetrics, TerminalSize, render_plain};
    use xulo_ui::{Size, UiContext};

    let surface = Size {
        width: 60,
        height: 20,
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

    let size = TerminalSize { cols: 60, rows: 20 };

    // Type "World" into the input (index 0)
    ui.input_change(0, "World".to_string()).unwrap();

    let rendered = render_plain(&ui.ops(), size);
    println!("After typing 'World':\n{}", rendered);
    assert!(
        rendered.contains("Hello, World"),
        "greeting should show 'Hello, World' after input, got:\n{}",
        rendered
    );
    // Counter should still show 0
    assert!(
        rendered.contains("Count: 0"),
        "count should still be 0 after NameField input, got:\n{}",
        rendered
    );

    // Click + button
    let (bx, by) = (ui.buttons[0].x, ui.buttons[0].y);
    ui.handle_click(bx, by).unwrap();

    let rendered = render_plain(&ui.ops(), size);
    println!("After clicking +:\n{}", rendered);
    assert!(
        rendered.contains("Count: 1"),
        "count should be 1 after click, got:\n{}",
        rendered
    );
    // NameField should still show "Hello, World"
    assert!(
        rendered.contains("Hello, World"),
        "greeting should still be 'Hello, World' after Counter rerender, got:\n{}",
        rendered
    );
}

#[test]
fn input_state_not_reset_by_button_click() {
    // Reproduce: type in Input, then click Button — Input should keep its value.
    let path = temp_program(
        "use ui::{Screen, VStack, Text, Button, Input};\n\
         fn Counter(): View {\n\
         @State let count: Number = 0\n\
         VStack(spacing: 1) {\n\
         Text(\"Count: \" + str(count))\n\
         Button(onClick: fn() { count = count + 1 }) { Text(\"+\") }\n\
         }\n\
         }\n\
         fn NameField(): View {\n\
         @State let name: String = \"\"\n\
         VStack(spacing: 1) {\n\
         Input(value: $name, width: 200, placeholder: \"Enter your name\")\n\
         Text(\"Hello, \" + name)\n\
         }\n\
         }\n\
         fn main(): View {\n\
         Screen {\n\
         Counter()\n\
         NameField()\n\
         }\n\
         }\n",
    );
    use xulo_renderer_terminal::{CharMetrics, TerminalSize, render_plain};
    use xulo_ui::{Size, UiContext};

    let surface = Size {
        width: 60,
        height: 24,
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

    let size = TerminalSize { cols: 60, rows: 24 };

    // Step 1: Type "World" into the input
    ui.input_change(0, "World".to_string()).unwrap();
    let rendered = render_plain(&ui.ops(), size);
    println!("Step 1 - After typing 'World':\n{}", rendered);
    assert!(
        rendered.contains("Hello, World"),
        "Step 1: greeting should show 'Hello, World', got:\n{}",
        rendered
    );

    // Step 2: Click the + button (Counter state change)
    let (bx, by) = (ui.buttons[0].x, ui.buttons[0].y);
    ui.handle_click(bx, by).unwrap();
    let rendered = render_plain(&ui.ops(), size);
    println!("Step 2 - After clicking +:\n{}", rendered);
    assert!(
        rendered.contains("Count: 1"),
        "Step 2: count should be 1, got:\n{}",
        rendered
    );
    // THIS IS THE CRITICAL CHECK: NameField state should survive
    assert!(
        rendered.contains("Hello, World"),
        "Step 2: greeting should STILL be 'Hello, World' after Counter rerender, got:\n{}",
        rendered
    );
}

const FILL_UI_SRC: &str = r##"
use ui::{Screen, VStack, Fill, Text};

fn main(): View {
    Screen(bg: "#101010") {
        VStack(height: 24) {
            Text("top")
            Fill {
                Text("inside fill")
            }
            Text("bottom")
        }
    }
}
"##;

#[test]
fn fill_children_preserved_through_framework() {
    let path = temp_program(FILL_UI_SRC);
    let ui = xulo_framework::ReactiveUi::load(
        &path,
        xulo_ui::Size {
            width: 320,
            height: 192,
        },
        None,
    )
    .unwrap();
    cleanup(&path);

    // Render through the layout engine to check paint ops
    let ctx = xulo_ui::UiContext::new(
        xulo_ui::Size {
            width: 320,
            height: 192,
        },
        Box::new(xulo_ui::CellMetrics),
    );
    let placed = ctx.layout(&ui.widget);
    let mut ops = Vec::new();
    xulo_ui::layout::paint(&placed, &xulo_ui::Theme::default(), &mut ops);

    let texts: Vec<&str> = ops
        .iter()
        .filter_map(|op| match op {
            xulo_ui::PaintOp::DrawText { text, .. } => Some(*text),
            _ => None,
        })
        .collect();

    assert!(
        texts.contains(&"top"),
        "should have 'top', got: {:?}",
        texts
    );
    assert!(
        texts.contains(&"inside fill"),
        "should have 'inside fill', got: {:?}",
        texts
    );
    assert!(
        texts.contains(&"bottom"),
        "should have 'bottom', got: {:?}",
        texts
    );
}

#[cfg(feature = "webview")]
#[test]
fn dump_webview_html_for_layout_example() {
    let entry = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/layout.xulo");
    let result = xulo_framework::execute(&entry).unwrap();
    let view = result.root_view.unwrap();
    let interp = std::rc::Rc::new(xulo_runtime::interpreter::Interpreter::new());
    let (widget, _cbs) = xulo_framework::widget_tree_with_callbacks(&view, &interp);
    let json = serde_json::to_string(&widget).unwrap();
    let html = xulo_renderer_webview::build_html_with_context(
        &json,
        xulo_renderer_webview::WebviewSize::new(80, 24),
        (10, 10, 26),
    );
    std::fs::write("/tmp/xulo_test.html", &html).unwrap();
    eprintln!("Widget JSON: {}", &json);
    eprintln!("HTML length: {} bytes", html.len());
    // Print the JS function section
    if let Some(start) = html.find("function render(") {
        let end = (start + 2000).min(html.len());
        let chunk = &html[start..end];
        eprintln!("JS render function:\n{}", chunk);
    }
}
