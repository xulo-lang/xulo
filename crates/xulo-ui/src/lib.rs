//! Pure UI logic: the widget tree, layout engine, and paint commands that a
//! renderer backend (terminal, webview) consumes. This crate has no external
//! dependencies and knows nothing about pixels or the terminal — backends
//! implement [`FontMetrics`] and turn [`PaintOp`]s into output.

pub mod ctx;
pub mod layout;
pub mod painting;
pub mod taffy_layout;
pub mod widgets;
pub use ctx::UiContext;
pub use layout::{collect_button_rects, collect_input_rects, collect_interactive_rects, CellMetrics, Constraints, FontMetrics, HandwrittenLayout, LayoutEngine, Placed, Theme, PAD_X, PAD_Y};
pub use painting::{PaintOp, PaintingEngine, PaintOpPainter};
pub use taffy_layout::TaffyLayout;
pub use widgets::{from_props, Alignment, Color, Dimension, FontWeight, Justify, Props, PropValue, Rect, Size, StyleProps, UiCallback, Widget, CELL_H, CELL_W};
