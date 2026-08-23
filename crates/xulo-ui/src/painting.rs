//! Backend-agnostic drawing commands.
//!
//! A laid-out widget tree is flattened into a sequence of [`PaintOp`]s by
//! `crate::layout::paint`. Renderers consume only these ops, so they never see
//! the widget tree or the layout rules.

use crate::widgets::{Color, Rect};

/// A backend-agnostic drawing command in layout units.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PaintOp {
    /// Fill the whole surface with `color` (clear).
    Clear { color: Color },
    /// Fill a rectangle with `color`.
    FillRect { rect: Rect, color: Color },
    /// Draw `text`, clipped to `rect`. The rect's top-left is the text origin.
    DrawText {
        rect: Rect,
        text: String,
        color: Color,
    },
    /// Outline `rect` with `color`.
    DrawBorder { rect: Rect, color: Color },
}
