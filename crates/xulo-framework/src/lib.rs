//! Unified entry point for rendering Xulo UI programs (the eframe analogue).
//!
//! The framework orchestrates the pipeline end to end: load and analyze a
//! program (`xulo-compiler`), execute it (`xulo-runtime`, capturing the entry
//! `main`'s `View`), convert the render tree into `xulo-ui` widgets, then hand
//! them to a renderer backend — the terminal lays them out natively, the
//! webview ships the tree to the embedded `xulo-ui-wasm` engine which lays them
//! out and draws them in the browser.

pub mod convert;
pub mod interactive;
pub mod run;

#[cfg(feature = "webview")]
pub mod wasm_assets {
    //! The compiled `xulo-ui-wasm` layout engine, embedded at build time
    //! (see the crate's `build.rs`).

    /// The layout engine wasm, base64-encoded for inline embedding.
    pub const WASM_B64: &str = include_str!(concat!(env!("OUT_DIR"), "/xulo_wasm.b64"));
}

pub use convert::{widget_from_value, widget_tree_with_callbacks};
pub use interactive::{FrameBuilder, ReactiveUi};
pub use run::{execute, render_to_string, run, Backend, ExecuteResult};
pub use xulo_ui::collect_button_rects;
