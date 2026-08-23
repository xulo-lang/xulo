//! Terminal renderer backend: turns [`PaintOp`]s into a character grid.

pub mod terminal_backend;

pub use terminal_backend::{
    CharMetrics, Grid, TerminalSize, render_ansi, render_plain, render_stdout,
};
