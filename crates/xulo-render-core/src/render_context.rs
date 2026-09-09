//! [`RenderContext`] — compound supertrait composition.
//!
//! `RenderContext` composes all required capability traits. Use
//! `&dyn RenderContext` to pass a render context without specifying the
//! concrete backend.

use super::effects::Effects;
use super::masking::Masking;
use super::painter::Painter;
use super::text_metrics::TextMetrics;
use super::text_renderer::TextRenderer;

/// Platform-agnostic rendering context — the full drawing surface.
///
/// Composes all required capability traits. Backends implement this
/// single trait; callers use `&dyn RenderContext` or `&mut dyn RenderContext`.
pub trait RenderContext: Painter + TextRenderer + TextMetrics + Masking + Effects {
    /// Device pixel ratio for crisp rendering. Returns `1.0` for
    /// non-HiDPI surfaces.
    fn dpr(&self) -> f64;
}
