//! Core render context traits and platform abstraction for Xulo.
//!
//! This crate defines the backend-agnostic trait hierarchy inspired by
//! uzor's `RenderContext` design:
//!
//! - [`Painter`] — Canvas2D-style path construction, fill, stroke, transforms
//! - [`TextRenderer`] — font selection and text drawing
//! - [`TextMetrics`] — text measurement for layout
//! - [`Masking`] — clip region management
//! - [`Effects`] — visual effects (shadow, blur)
//! - [`RenderContext`] — compound supertrait composing all the above
//!
//! Platform abstraction:
//!
//! - [`platform::PlatformBackend`] — window creation, event polling
//! - [`platform::SystemIntegration`] — clipboard, system theme
//! - [`types::WindowConfig`], [`types::PlatformEvent`], etc.

pub mod effects;
pub mod hub;
pub mod masking;
pub mod painter;
pub mod platform;
pub mod render_context;
pub mod text_metrics;
pub mod text_renderer;
pub mod types;

pub use effects::Effects;
pub use hub::RenderHub;
pub use masking::Masking;
pub use painter::Painter;
pub use platform::{MockPlatform, PlatformBackend, SystemIntegration};
pub use render_context::RenderContext;
pub use text_metrics::{TextBounds, TextMetrics};
pub use text_renderer::{TextBaseline, TextRenderer, TextAlign};
pub use types::{
    EventResult, PlatformError, RenderBackend, WindowConfig, WindowId, PlatformEvent,
};
