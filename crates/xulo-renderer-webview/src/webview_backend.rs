//! WebView renderer backend.
//!
//! Renders a widget tree by running the `xulo-ui` layout/paint engine as WASM
//! inside a native webview window (wry). The page instantiates the wasm, feeds
//! it the serialized widget tree, and draws the returned paint commands onto a
//! `<canvas>`; mouse clicks are hit-tested in wasm and reported back over IPC.
//!
//! On Linux the window is a plain GTK window driving the WebKit main context
//! directly (the wry-recommended `build_gtk` path); other platforms use winit.

use std::cell::RefCell;
use std::rc::Rc;

/// Cell size in pixels: the webview's monospace glyph grid (shared with the
/// wasm engine via `xulo-ui`).
pub use xulo_ui::{CellMetrics, CELL_H, CELL_W};

/// A webview surface sized in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WebviewSize {
    pub cols: u32,
    pub rows: u32,
}

impl WebviewSize {
    pub const fn new(cols: u32, rows: u32) -> Self {
        Self { cols, rows }
    }

    pub fn pixels(&self) -> (u32, u32) {
        (self.cols * CELL_W, self.rows * CELL_H)
    }
}

/// Build the full page for `size` cells: it instantiates the embedded raw wasm
/// layout engine, feeds it the serialized widget tree (`tree_json`), lays the
/// tree out in wasm, and draws the returned paint commands onto a `<canvas>`.
/// The canvas is filled with `background` synchronously (before wasm is ready)
/// so the first paint is never black, and button clicks are hit-tested in wasm
/// and reported over IPC as `"click <index>"`.
pub fn build_html(
    tree_json: &str,
    size: WebviewSize,
    wasm_b64: &str,
    background: (u8, u8, u8),
) -> String {
    let (width, height) = size.pixels();
    let bg = format!("rgb({},{},{})", background.0, background.1, background.2);
    const PAGE: &str = r#"<!doctype html>
<html>
<head>
<meta charset="utf-8">
<style>html,body{margin:0;padding:0;background:__BG__;overflow:hidden;}</style>
</head>
<body>
<canvas id="screen"></canvas>
<script>
const t0 = performance.now();
const TREE = __TREE__;
const W = __W__;
const H = __H__;
const canvas = document.getElementById('screen');
const dpr = window.devicePixelRatio || 1;
canvas.width = W * dpr;
canvas.height = H * dpr;
canvas.style.width = W + 'px';
canvas.style.height = H + 'px';
const ctx = canvas.getContext('2d');
ctx.scale(dpr, dpr);
ctx.textBaseline = 'top';
// First paint is the themed background — never a black window while the wasm
// engine compiles and lays the tree out.
ctx.fillStyle = '__BG__';
ctx.fillRect(0, 0, W, H);
const col = (c) => `rgb(${c.r},${c.g},${c.b})`;
function draw(ops) {
  for (const op of ops) {
    switch (op.op) {
      case 'clear': ctx.fillStyle = col(op.color); ctx.fillRect(0, 0, W, H); break;
      case 'fill': ctx.fillStyle = col(op.color); ctx.fillRect(op.x, op.y, op.w, op.h); break;
      case 'text':
        ctx.fillStyle = col(op.color);
        ctx.font = '12px monospace';
        ctx.save(); ctx.beginPath(); ctx.rect(op.x, op.y, op.w, op.h); ctx.clip();
        ctx.fillText(op.text, op.x, op.y); ctx.restore();
        break;
      case 'border':
        ctx.strokeStyle = col(op.color); ctx.lineWidth = 1;
        ctx.strokeRect(op.x + 0.5, op.y + 0.5, op.w - 1, op.h - 1); break;
    }
  }
}
const bin = atob('__WASM_B64__');
const bytes = new Uint8Array(bin.length);
for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
let wasm = null;
WebAssembly.instantiate(bytes, {})
  .then(({ instance }) => { wasm = instance.exports; redraw(TREE); })
  .catch((e) => window.ipc.postMessage('BOOTERR ' + (e && e.message ? e.message : String(e))));
function redraw(tree) {
  if (!wasm) return;
  try {
    const t = (typeof tree === 'string') ? JSON.parse(tree) : tree;
    const text = JSON.stringify(t);
    const enc = new TextEncoder().encode(text);
    const ptr = wasm.xulo_alloc(enc.length);
    new Uint8Array(wasm.memory.buffer, ptr, enc.length).set(enc);
    const len = wasm.xulo_layout(ptr, enc.length, W, H);
    wasm.xulo_dealloc(ptr, enc.length);
    const rptr = wasm.xulo_result_ptr();
    const ops = JSON.parse(new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, rptr, len)));
    // WebKitGTK skips repainting an idle page, so a canvas update triggered from
    // evaluate_script isn't presented until the next interaction. Drawing inside
    // an animation frame forces a rendering update, so the click result shows
    // immediately instead of waiting for a mouse move.
    requestAnimationFrame(() => {
      draw(ops);
      if (!window.__perf) {
        window.__perf = true;
        window.ipc.postMessage('PERF firstdraw=' + Math.round(performance.now() - t0));
      }
    });
  } catch (e) {
    window.ipc.postMessage('RENDERERR ' + (e && e.message ? e.message : String(e)));
  }
}
window.redraw = redraw;
canvas.addEventListener('click', (e) => {
  const r = canvas.getBoundingClientRect();
  const x = e.clientX - r.left;
  const y = e.clientY - r.top;
  const idx = wasm ? wasm.xulo_hit_test(x, y) : -1;
  if (idx >= 0) window.ipc.postMessage('click ' + idx);
});
</script>
</body>
</html>"#;
    PAGE.replace("__TREE__", tree_json)
        .replace("__W__", &width.to_string())
        .replace("__H__", &height.to_string())
        .replace("__WASM_B64__", wasm_b64)
        .replace("__BG__", &bg)
}

/// Parse a `"click <button-index>"` IPC message from the page.
fn parse_click(body: &str) -> Option<i32> {
    let mut parts = body.split_whitespace();
    if parts.next()? != "click" {
        return None;
    }
    parts.next()?.parse().ok()
}

/// A click handler: receives the index of the clicked button (0-based, tree
/// order) and returns a JS expression to evaluate for re-rendering (usually
/// `redraw(<tree-json>)`), or `None` when nothing changed.
pub type ClickHandler = Box<dyn Fn(i32) -> Option<String> + 'static>;

/// Build a webview over the given HTML. The IPC handler reports button clicks
/// to `on_click` and redraws the page through the webview stored in `slot`
/// (filled once the webview is created, after the builder returns).
fn build_webview(
    html: &str,
    background: (u8, u8, u8),
    on_click: ClickHandler,
    slot: Rc<RefCell<Option<wry::WebView>>>,
) -> wry::WebViewBuilder {
    wry::WebViewBuilder::new()
        .with_html(html.to_string())
        .with_background_color((background.0, background.1, background.2, 255))
        .with_ipc_handler(move |req| {
            let body = req.body().clone();
            if let Some(index) = parse_click(&body) {
                if let Some(js) = on_click(index) {
                    if let Some(webview) = slot.borrow().as_ref() {
                        let _ = webview.evaluate_script(&js);
                    }
                }
            } else if body.starts_with("PERF")
                || body.starts_with("BOOTERR")
                || body.starts_with("RENDERERR")
            {
                eprintln!("webview: {body}");
            }
        })
}

/// Open a native webview window rendering `html` and block until it is closed.
/// Canvas button clicks are reported to `on_click`, whose returned JS
/// expression (if any) is evaluated in the page to redraw it. `background`
/// colors the window itself so it is never a black void while the page loads.
/// Returns a user-facing error string on failure (e.g. no display server).
pub fn run(
    html: String,
    title: String,
    size: WebviewSize,
    background: (u8, u8, u8),
    on_click: ClickHandler,
) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        run_linux(html, &title, size, background, on_click)
    }
    #[cfg(not(target_os = "linux"))]
    {
        run_winit(html, &title, size, background, on_click)
    }
}

/// Linux path: a plain GTK window whose main loop drives the WebKit main
/// context directly (wry's `build_gtk`), avoiding the winit event-loop
/// starvation that would otherwise delay page load and first paint.
#[cfg(target_os = "linux")]
fn run_linux(
    html: String,
    title: &str,
    size: WebviewSize,
    background: (u8, u8, u8),
    on_click: ClickHandler,
) -> Result<(), String> {
    use gtk::prelude::*;
    use wry::WebViewBuilderExtUnix;

    // wry's webkitgtk path expects the X11 GTK display (it downcasts
    // `gdk::Display` to `GdkX11Display`). Keep GDK on X11 so the display
    // matches, instead of GTK auto-selecting Wayland.
    unsafe {
        std::env::set_var("GDK_BACKEND", "x11");
    }
    gtk::init().map_err(|e| format!("cannot init gtk: {e}"))?;

    let (w, h) = size.pixels();
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title(title);
    window.set_default_size(w as i32, h as i32);
    // wry packs webviews into a `GtkBox` with expand+fill (a `GtkFixed` sizes
    // the webview to 1x1 unless bounds are given), so a box makes the page fill
    // the whole window.
    let box_ = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.add(&box_);
    box_.show_all();

    let slot = Rc::new(RefCell::new(None::<wry::WebView>));
    let webview = build_webview(&html, background, on_click, slot.clone())
        .build_gtk(&box_)
        .map_err(|e| format!("cannot create webview: {e}"))?;
    *slot.borrow_mut() = Some(webview);

    window.show_all();
    window.connect_delete_event(|_, _| {
        gtk::main_quit();
        gtk::glib::Propagation::Proceed
    });
    gtk::main();
    Ok(())
}

/// Non-Linux path: a winit window with wry's `build(&window)` embedding.
#[cfg(not(target_os = "linux"))]
fn run_winit(
    html: String,
    title: &str,
    size: WebviewSize,
    background: (u8, u8, u8),
    on_click: ClickHandler,
) -> Result<(), String> {
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::window::{Window, WindowAttributes};

    struct App {
        html: Option<String>,
        title: String,
        size: WebviewSize,
        background: (u8, u8, u8),
        on_click: Option<ClickHandler>,
        /// The built webview, reachable from the IPC handler (which is created
        /// before the webview exists) so a click can redraw the page.
        webview_slot: Rc<RefCell<Option<wry::WebView>>>,
        window: Option<Window>,
    }

    impl ApplicationHandler for App {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            if self.webview_slot.borrow().is_some() {
                return;
            }
            let (w, h) = self.size.pixels();
            use winit::dpi::LogicalSize;
            let attrs = WindowAttributes::default()
                .with_title(self.title.clone())
                .with_inner_size(LogicalSize::new(w as f64, h as f64));
            let window = match event_loop.create_window(attrs) {
                Ok(window) => window,
                Err(err) => {
                    eprintln!("webview: cannot create window: {err}");
                    event_loop.exit();
                    return;
                }
            };
            let html = self.html.take().expect("html consumed once");
            let on_click = self.on_click.take().expect("click handler consumed once");
            let slot = self.webview_slot.clone();
            let webview = match build_webview(&html, self.background, on_click, slot.clone())
                .build(&window)
            {
                Ok(webview) => webview,
                Err(err) => {
                    eprintln!("webview: cannot create webview: {err}");
                    event_loop.exit();
                    return;
                }
            };
            *self.webview_slot.borrow_mut() = Some(webview);
            self.window = Some(window);
        }

        fn window_event(
            &mut self,
            event_loop: &ActiveEventLoop,
            _id: winit::window::WindowId,
            event: WindowEvent,
        ) {
            if matches!(event, WindowEvent::CloseRequested) {
                event_loop.exit();
            }
        }
    }

    let event_loop = EventLoop::new().map_err(|e| format!("cannot create event loop: {e}"))?;
    let mut app = App {
        html: Some(html),
        title: title.to_string(),
        size,
        background,
        on_click: Some(on_click),
        webview_slot: Rc::new(RefCell::new(None)),
        window: None,
    };
    event_loop
        .run_app(&mut app)
        .map_err(|e| format!("event loop error: {e}"))
}
