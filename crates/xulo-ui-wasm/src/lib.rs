//! The `xulo-ui` layout/paint engine compiled to WASM.
//!
//! The webview backend embeds this module and drives it over a raw C-style ABI
//! (no wasm-bindgen): JS writes the serialized widget tree into wasm memory,
//! calls `xulo_layout`, and reads back the serialized paint commands plus the
//! button rectangles. Button clicks are hit-tested back to an index with
//! `xulo_hit_test`. Running the layout engine as wasm means geometry lives in
//! the browser, like egui/eframe's web backend.

use std::cell::RefCell;

use xulo_ui::{PaintOp, Rect, Size, Widget};

/// Lay a widget tree out against a pixel surface: paint commands plus the
/// button rectangles (in the same pre-order the framework keeps callbacks).
/// Pure, so it is testable on the host.
pub fn layout_tree(tree: &Widget, width: u32, height: u32) -> (Vec<PaintOp>, Vec<Rect>) {
    let ctx = xulo_ui::UiContext::new(Size { width, height }, Box::new(xulo_ui::CellMetrics));
    let ops = ctx.paint(tree);
    let placed = ctx.layout(tree);
    let mut buttons = Vec::new();
    xulo_ui::collect_button_rects(&placed, &mut buttons);
    (ops, buttons)
}

/// Hit-test `(x, y)` against `buttons`; returns the 0-based index or -1.
pub fn hit_index(buttons: &[Rect], x: f64, y: f64) -> i32 {
    buttons
        .iter()
        .position(|r| r.contains(x as u32, y as u32))
        .map_or(-1, |i| i as i32)
}

/// Serialize paint commands into the flat `[{op, color, x, y, w, h, text}]`
/// array the page's rasterizer consumes.
pub fn ops_to_json(ops: &[PaintOp]) -> String {
    let color_json = |c: &xulo_ui::Color| serde_json::json!({ "r": c.r, "g": c.g, "b": c.b });
    let rect_json =
        |r: &Rect| serde_json::json!({ "x": r.x, "y": r.y, "w": r.width, "h": r.height });
    let arr: Vec<serde_json::Value> = ops
        .iter()
        .map(|op| match op {
            PaintOp::Clear { color } => {
                serde_json::json!({ "op": "clear", "color": color_json(color) })
            }
            PaintOp::FillRect { rect, color } => {
                let mut v = rect_json(rect);
                v["op"] = serde_json::json!("fill");
                v["color"] = color_json(color);
                v
            }
            PaintOp::DrawText { rect, text, color } => {
                let mut v = rect_json(rect);
                v["op"] = serde_json::json!("text");
                v["text"] = serde_json::json!(text);
                v["color"] = color_json(color);
                v
            }
            PaintOp::DrawBorder { rect, color } => {
                let mut v = rect_json(rect);
                v["op"] = serde_json::json!("border");
                v["color"] = color_json(color);
                v
            }
        })
        .collect();
    serde_json::Value::Array(arr).to_string()
}

thread_local! {
    /// The last layout's serialized paint commands.
    static RESULT: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// The last layout's button rectangles.
    static BUTTONS: RefCell<Vec<Rect>> = const { RefCell::new(Vec::new()) };
}

/// Allocate `len` bytes of wasm memory (for JS to write the tree into).
#[unsafe(no_mangle)]
pub extern "C" fn xulo_alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Release memory previously handed out by [`xulo_alloc`].
// Safety: the pointer must come from `xulo_alloc`; the JS host guarantees this.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_dealloc(ptr: *mut u8, len: usize) {
    unsafe {
        std::mem::drop(Vec::from_raw_parts(ptr, 0, len));
    }
}

/// Lay the tree at `tree_ptr` (JSON bytes) out against `width`×`height`, store
/// the serialized paint commands (reachable via [`xulo_result_ptr`]), and
/// record the button rectangles for hit-testing. Returns the result length in
/// bytes.
// Safety: `tree_ptr` must point to `tree_len` readable bytes; the JS host
// writes them via `xulo_alloc` before calling.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_layout(
    tree_ptr: *const u8,
    tree_len: usize,
    width: u32,
    height: u32,
) -> usize {
    let json = unsafe { std::slice::from_raw_parts(tree_ptr, tree_len) };
    let json = String::from_utf8_lossy(json);
    let tree: Widget = match serde_json::from_str(&json) {
        Ok(tree) => tree,
        Err(_) => return 0,
    };
    let (ops, buttons) = layout_tree(&tree, width, height);
    BUTTONS.with(|b| *b.borrow_mut() = buttons);
    let result = ops_to_json(&ops);
    RESULT.with(|r| {
        *r.borrow_mut() = result.into_bytes();
        r.borrow().len()
    })
}

/// Pointer to the last layout's serialized paint commands.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_result_ptr() -> *const u8 {
    RESULT.with(|r| r.borrow().as_ptr())
}

/// Number of buttons in the last layout.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_button_count() -> usize {
    BUTTONS.with(|b| b.borrow().len())
}

/// Rectangle of the `index`-th button, written as `[x, y, w, h]` into `out`
/// (4 f64s). Returns 1 on success, 0 if `index` is out of range.
// Safety: `out` must point to 4 f64s of writable memory; the JS host provides it.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_button(index: usize, out: *mut f64) -> i32 {
    let rect = BUTTONS.with(|b| b.borrow().get(index).copied());
    match rect {
        Some(r) => {
            unsafe {
                *out.add(0) = r.x as f64;
                *out.add(1) = r.y as f64;
                *out.add(2) = r.width as f64;
                *out.add(3) = r.height as f64;
            }
            1
        }
        None => 0,
    }
}

/// Which button does `(x, y)` fall in? 0-based index, or -1.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_hit_test(x: f64, y: f64) -> i32 {
    let buttons = BUTTONS.with(|b| b.borrow().clone());
    hit_index(&buttons, x, y)
}
