//! WebView backend tests: the page built around the embedded raw wasm layout
//! engine (pure functions; the windowing path needs a display server).

use xulo_renderer_webview::{build_html, WebviewSize, CELL_W};
use xulo_ui::FontMetrics;

const SIZE: WebviewSize = WebviewSize::new(40, 12);
const WASM_B64: &str = "AGFzbQAAAA==";
const BG: (u8, u8, u8) = (32, 32, 32);

#[test]
fn page_embeds_tree_wasm_and_rasterizer() {
    let tree = r#"{"VStack":{"spacing":1,"children":[]}}"#;
    let html = build_html(tree, SIZE, WASM_B64, BG);
    // The base64 wasm is inlined and instantiated as a raw module.
    assert!(html.contains("AGFzbQAAAA=="), "wasm base64 inlined");
    assert!(
        html.contains("WebAssembly.instantiate(bytes, {})"),
        "raw instantiation"
    );
    // The tree reaches the wasm engine through its raw ABI.
    assert!(
        html.contains(r#"const TREE = {"VStack":{"spacing":1,"children":[]}};"#),
        "tree embedded"
    );
    assert!(
        html.contains("wasm.xulo_layout(ptr, enc.length, W, H)"),
        "layout call"
    );
    assert!(html.contains("wasm.xulo_result_ptr()"), "result readback");
    // Clicks are hit-tested in wasm and reported by button index.
    assert!(html.contains("const idx = wasm ? wasm.xulo_hit_test(x, y) : -1;"));
    assert!(html.contains("window.ipc.postMessage('click ' + idx);"));
    // Canvas sized to cells x CELL px.
    assert!(html.contains("const W = 320;"));
    assert!(html.contains("const H = 192;"));
}

#[test]
fn page_paints_background_before_wasm() {
    // The themed background fills the canvas synchronously, before the wasm
    // engine is ready — the window is never a black void.
    let html = build_html("null", SIZE, WASM_B64, (1, 2, 3));
    assert!(html.contains("background:rgb(1,2,3);"));
    assert!(html.contains("ctx.fillStyle = 'rgb(1,2,3)';\nctx.fillRect(0, 0, W, H);"));
}

#[test]
fn page_exposes_redraw() {
    let html = build_html("null", SIZE, WASM_B64, BG);
    assert!(html.contains("window.redraw = redraw;"));
    assert!(html.contains("function redraw(tree)"));
}

#[test]
fn cell_metrics_are_cell_px() {
    let metrics = xulo_renderer_webview::CellMetrics;
    assert_eq!(metrics.text_width("ab"), CELL_W * 2);
    assert_eq!(metrics.line_height(), xulo_ui::CELL_H);
}
