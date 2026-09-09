//! Skia-based rendering backend for Xulo.
//!
//! Implements the [`RenderContext`] trait hierarchy using `skia-safe`,
//! providing hardware-accelerated 2D rendering with anti-aliasing,
//! gradients, shadows, and sub-pixel text.
//!
//! # Modules
//!
//! - [`skia_context`] — `SkiaRenderContext` implementing `RenderContext`
//! - [`webview_bridge`] — CPU bitmap rendering for webview display

pub mod skia_context;
pub mod webview_bridge;

pub use skia_context::SkiaRenderContext;
pub use webview_bridge::SkiaBitmapBridge;
