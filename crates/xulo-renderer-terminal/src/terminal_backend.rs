//! Terminal renderer backend.
//!
//! Consumes [`PaintOp`]s (already laid out) and draws them into a character
//! grid, which it emits either as plain text or with ANSI true-color escapes.
//! It has no external dependencies: the terminal size comes from the
//! `COLUMNS`/`LINES` environment variables (defaulting to 80x24), and the text
//! metric is one cell per character.

use std::io::Write;

use xulo_ui::{Color, FontMetrics, PaintOp, Rect};

/// The character-cell metrics this backend renders with.
#[derive(Debug, Clone, Copy)]
pub struct CharMetrics;

impl FontMetrics for CharMetrics {
    fn text_width(&self, text: &str) -> u32 {
        text.chars().count() as u32
    }
    fn line_height(&self) -> u32 {
        1
    }
}

/// Terminal size in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalSize {
    pub cols: u32,
    pub rows: u32,
}

impl TerminalSize {
    pub fn from_env() -> Self {
        let cols = std::env::var("COLUMNS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(80);
        let rows = std::env::var("LINES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(24);
        Self { cols, rows }
    }
}

impl Default for TerminalSize {
    fn default() -> Self {
        Self::from_env()
    }
}

/// The screen, rendered as a grid of cells with per-cell fg/bg colors.
pub struct Grid {
    chars: Vec<Vec<char>>,
    fg: Vec<Vec<Option<Color>>>,
    bg: Vec<Vec<Option<Color>>>,
    cols: usize,
    rows: usize,
}

impl Grid {
    pub fn new(cols: u32, rows: u32) -> Self {
        let cols = cols as usize;
        let rows = rows as usize;
        Self {
            chars: vec![vec![' '; cols]; rows],
            fg: vec![vec![None; cols]; rows],
            bg: vec![vec![None; cols]; rows],
            cols,
            rows,
        }
    }

    /// Rasterize a sequence of paint commands into the grid.
    pub fn from_ops(ops: &[PaintOp<'_>], size: TerminalSize) -> Self {
        let mut grid = Self::new(size.cols, size.rows);
        for op in ops {
            match op {
                PaintOp::Clear { color } => {
                    for row in grid.bg.iter_mut() {
                        row.fill(Some(*color));
                    }
                }
                PaintOp::FillRect { rect, color } => {
                    grid.fill_rect(*rect, *color);
                }
                PaintOp::DrawText { rect, text, color } => {
                    grid.draw_text(*rect, text, *color);
                }
                PaintOp::DrawBorder { rect, color } => {
                    grid.draw_border(*rect, *color);
                }
                PaintOp::Input { rect, text, placeholder: _, color, focused } => {
                    grid.draw_border(*rect, Color::GRAY);
                    let inner = Rect::new(
                        rect.x + 1,
                        rect.y + 1,
                        rect.width.saturating_sub(2),
                        rect.height.saturating_sub(2),
                    );
                    grid.draw_text(inner, text, *color);
                    // Show cursor indicator when focused
                    if *focused {
                        let cursor_x = inner.x + text.chars().count() as u32;
                        if cursor_x < rect.right().saturating_sub(1) {
                            grid.set_char(cursor_x, inner.y, '▏', *color);
                        }
                    }
                }
            }
        }
        grid
    }

    fn fill_rect(&mut self, rect: Rect, color: Color) {
        let clip = rect.intersect(&self.bounds()).unwrap_or(rect);
        for y in clip.y..clip.bottom() {
            for x in clip.x..clip.right() {
                if self.in_bounds(x, y) {
                    self.bg[y as usize][x as usize] = Some(color);
                }
            }
        }
    }

    fn draw_text(&mut self, rect: Rect, text: &str, color: Color) {
        let y = rect.y;
        if y >= self.rows as u32 {
            return;
        }
        for (i, ch) in text.chars().enumerate() {
            let x = rect.x + i as u32;
            if !self.in_bounds(x, y) {
                continue;
            }
            let row = y as usize;
            let col = x as usize;
            if self.chars[row][col] == ' ' {
                // Non-blank cells (e.g. a border character) win over text.
                self.chars[row][col] = ch;
                self.fg[row][col] = Some(color);
            }
        }
    }

    fn draw_border(&mut self, rect: Rect, color: Color) {
        let clip = rect.intersect(&self.bounds()).unwrap_or(rect);
        if clip.width < 2 || clip.height < 2 {
            return;
        }
        let (x0, y0, x1, y1) = (
            clip.x,
            clip.y,
            clip.right().saturating_sub(1),
            clip.bottom().saturating_sub(1),
        );
        for x in x0..=x1 {
            self.set_char(x, y0, '─', color);
            self.set_char(x, y1, '─', color);
        }
        for y in y0..=y1 {
            self.set_char(x0, y, '│', color);
            self.set_char(x1, y, '│', color);
        }
        self.set_char(x0, y0, '┌', color);
        self.set_char(x1, y0, '┐', color);
        self.set_char(x0, y1, '└', color);
        self.set_char(x1, y1, '┘', color);
    }

    fn set_char(&mut self, x: u32, y: u32, ch: char, color: Color) {
        if !self.in_bounds(x, y) {
            return;
        }
        let row = y as usize;
        let col = x as usize;
        self.chars[row][col] = ch;
        self.fg[row][col] = Some(color);
    }

    fn in_bounds(&self, x: u32, y: u32) -> bool {
        x < self.cols as u32 && y < self.rows as u32
    }

    fn bounds(&self) -> Rect {
        Rect::new(0, 0, self.cols as u32, self.rows as u32)
    }

    /// Plain text: rows joined with `\n`, trailing blanks trimmed per row, and
    /// trailing empty rows dropped. Color is ignored.
    pub fn to_plain(&self) -> String {
        let mut rows: Vec<String> = self
            .chars
            .iter()
            .map(|row| row.iter().collect::<String>().trim_end().to_string())
            .collect();
        while rows.last().is_some_and(|row| row.is_empty()) {
            rows.pop();
        }
        rows.join("\n")
    }

    /// ANSI true-color output: full grid (blanks keep their background), with
    /// per-cell foreground/background escapes emitted only on color changes.
    pub fn to_ansi(&self) -> String {
        let mut out = String::new();
        let mut prev_fg: Option<Color> = None;
        let mut prev_bg: Option<Color> = None;
        for row in 0..self.rows {
            for col in 0..self.cols {
                let fg = self.fg[row][col];
                let bg = self.bg[row][col];
                if fg != prev_fg || bg != prev_bg {
                    out.push_str("\x1b[0m");
                    if let Some(color) = bg {
                        out.push_str(&format!("\x1b[48;2;{};{};{}m", color.r, color.g, color.b));
                    }
                    if let Some(color) = fg {
                        out.push_str(&format!("\x1b[38;2;{};{};{}m", color.r, color.g, color.b));
                    }
                    prev_fg = fg;
                    prev_bg = bg;
                }
                out.push(self.chars[row][col]);
            }
            if row + 1 < self.rows {
                out.push('\n');
            }
        }
        out.push_str("\x1b[0m");
        out
    }
}

/// Rasterize `ops` into a grid and return it as plain text (no escapes).
pub fn render_plain(ops: &[PaintOp<'_>], size: TerminalSize) -> String {
    Grid::from_ops(ops, size).to_plain()
}

/// Rasterize `ops` into a grid and return it with ANSI true-color escapes.
pub fn render_ansi(ops: &[PaintOp<'_>], size: TerminalSize) -> String {
    Grid::from_ops(ops, size).to_ansi()
}

/// Rasterize `ops` and print them to stdout, clearing the screen first. ANSI
/// color is skipped when `NO_COLOR` is set or stdout is not a terminal.
pub fn render_stdout(ops: &[PaintOp<'_>], size: TerminalSize) {
    let grid = Grid::from_ops(ops, size);
    let colored = std::env::var_os("NO_COLOR").is_none()
        && std::io::IsTerminal::is_terminal(&std::io::stdout());
    let body = if colored {
        grid.to_ansi()
    } else {
        grid.to_plain()
    };
    let mut stdout = std::io::stdout().lock();
    if colored {
        let _ = write!(stdout, "\x1b[H\x1b[2J");
    }
    let _ = writeln!(stdout, "{body}");
    let _ = stdout.flush();
}
