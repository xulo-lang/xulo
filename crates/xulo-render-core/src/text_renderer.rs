//! [`TextRenderer`] — font selection and text drawing.

/// Text alignment relative to the draw position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// Text baseline alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextBaseline {
    Top,
    Middle,
    Alphabetic,
    Bottom,
}

/// Font selection and text drawing.
///
/// Backends implement this to render text at a given position with the
/// current font settings.
pub trait TextRenderer {
    /// Set the current font (CSS-style string, e.g. `"12px monospace"` or
    /// `"bold 16px sans-serif"`).
    fn set_font(&mut self, font: &str);

    /// Set text horizontal alignment.
    fn set_text_align(&mut self, align: TextAlign);

    /// Set text baseline alignment.
    fn set_text_baseline(&mut self, baseline: TextBaseline);

    /// Fill text at `(x, y)` using the current fill color and font.
    fn fill_text(&mut self, text: &str, x: f64, y: f64);

    /// Stroke text at `(x, y)` using the current stroke color and font.
    fn stroke_text(&mut self, text: &str, x: f64, y: f64);
}
