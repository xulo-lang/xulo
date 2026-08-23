//! WebView renderer backend: renders a widget tree by running the `xulo-ui`
//! layout/paint engine as WASM inside a native webview window (wry + winit).

pub mod webview_backend;

pub use webview_backend::{
    build_html, run, CellMetrics, ClickHandler, WebviewSize, CELL_H, CELL_W,
};
