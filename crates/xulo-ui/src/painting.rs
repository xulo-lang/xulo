//! Backend-agnostic drawing commands.
//!
//! A laid-out widget tree is flattened into a sequence of [`PaintOp`]s by
//! `crate::layout::paint`. Renderers consume only these ops, so they never see
//! the widget tree or the layout rules.

use crate::layout::{Placed, Theme};
use crate::widgets::{Color, Rect};

/// Painting engine interface. Implementors take a [`Placed`] tree and
/// produce [`PaintOp`]s for a specific rendering backend (terminal, webview/Skia).
pub trait PaintingEngine {
    fn paint<'a>(&self, placed: &Placed<'a>, theme: &Theme, ops: &mut Vec<PaintOp<'a>>);
}

/// The built-in `PaintOp`-based painter (used by all current backends).
pub struct PaintOpPainter;

impl PaintingEngine for PaintOpPainter {
    fn paint<'a>(&self, placed: &Placed<'a>, theme: &Theme, ops: &mut Vec<PaintOp<'a>>) {
        crate::layout::paint(placed, theme, ops);
    }
}

/// A backend-agnostic drawing command in layout units.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PaintOp<'a> {
    /// Fill the whole surface with `color` (clear).
    Clear { color: Color },
    /// Fill a rectangle with `color`.
    FillRect {
        rect: Rect,
        color: Color,
        /// Corner radius in layout units (0 = sharp corners).
        border_radius: u32,
        /// Opacity from 0.0 (transparent) to 1.0 (opaque).
        opacity: f32,
    },
    /// Draw `text`, clipped to `rect`. The rect's top-left is the text origin.
    DrawText {
        rect: Rect,
        text: &'a str,
        color: Color,
        /// Font size in pixels (default 12).
        font_size: u32,
        /// Whether to render bold text.
        bold: bool,
        /// Opacity from 0.0 (transparent) to 1.0 (opaque).
        opacity: f32,
    },
    /// Outline `rect` with `color`.
    DrawBorder {
        rect: Rect,
        color: Color,
        /// Corner radius in layout units (0 = sharp corners).
        border_radius: u32,
        /// Opacity from 0.0 (transparent) to 1.0 (opaque).
        opacity: f32,
    },
    /// An editable text input field. `focused` indicates whether this input
    /// currently has keyboard focus (backends render a cursor when true).
    Input {
        rect: Rect,
        text: &'a str,
        placeholder: &'a str,
        color: Color,
        focused: bool,
        /// Corner radius in layout units (0 = sharp corners).
        border_radius: u32,
        /// Background color (if set).
        background_color: Option<Color>,
        /// Border color (if set).
        border_color: Option<Color>,
        /// Opacity from 0.0 (transparent) to 1.0 (opaque).
        opacity: f32,
    },
    /// Begin a scrollable region. Content after this op (until the matching
    /// `ScrollEnd`) is clipped to `viewport` and translated by `(offset_x,
    /// offset_y)`. The renderer must save canvas state, apply clip + translate,
    /// draw children, then restore on `ScrollEnd`.
    ScrollStart {
        viewport: Rect,
        offset_x: i32,
        offset_y: i32,
    },
    /// End a scrollable region started by `ScrollEnd`.
    ScrollEnd,
}
