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
pub use layout::{
    CellMetrics, Constraints, FontMetrics, HandwrittenLayout, LayoutEngine, PAD_X, PAD_Y, Placed,
    Theme, collect_button_rects, collect_input_rects, collect_interactive_rects,
};
pub use painting::{PaintOp, PaintOpPainter, PaintingEngine};
pub use taffy_layout::TaffyLayout;
pub use widgets::{
    Alignment, CELL_H, CELL_W, Color, Dimension, FontWeight, Justify, PropValue, Props, Rect, Size,
    StyleProps, UiCallback, Widget, from_props,
};
