//! Terminal renderer backend: turns [`PaintOp`]s into a character grid.

pub mod terminal_backend;
pub mod terminal_platform;
pub mod terminal_render_context;

pub use terminal_backend::{
    CharMetrics, Grid, TerminalSize, render_ansi, render_plain, render_stdout,
};
pub use terminal_platform::TerminalPlatform;
pub use terminal_render_context::TerminalRenderContext;
