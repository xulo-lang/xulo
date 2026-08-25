//! Pure UI logic: the widget tree, layout engine, and paint commands that a
//! renderer backend (terminal, webview) consumes. This crate has no external
//! dependencies and knows nothing about pixels or the terminal — backends
//! implement [`FontMetrics`] and turn [`PaintOp`]s into output.

pub mod ctx;
pub mod layout;
pub mod painting;
pub mod widgets;

pub use ctx::UiContext;
pub use layout::{collect_button_rects, collect_input_rects, CellMetrics, FontMetrics, Placed, Theme, PAD_X, PAD_Y};
pub use painting::PaintOp;
pub use widgets::{Color, Rect, Size, Widget, CELL_H, CELL_W};
