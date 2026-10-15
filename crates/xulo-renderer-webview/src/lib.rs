//! WebView renderer backend: renders a widget tree by running the `xulo-ui`
//! layout/paint engine as WASM inside a native webview window (wry + winit).

pub mod canvas2d_render_context;
pub mod html_builder;
pub mod webview_backend;

pub use canvas2d_render_context::Canvas2dRenderContext;
pub use html_builder::build_html_with_context;
pub use webview_backend::{
    CELL_H, CELL_W, CellMetrics, ClickHandler, InputHandler, WebviewSize, build_html, run,
};
