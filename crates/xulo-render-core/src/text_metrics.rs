//! [`TextMetrics`] — text measurement.

/// Bounding box for a text run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextBounds {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub ascent: f64,
    pub descent: f64,
}

/// Text measurement — required by layout engines to compute intrinsic sizes.
pub trait TextMetrics {
    /// Measure the advance width of `text` with the current font.
    fn measure_text(&self, text: &str) -> f64;

    /// Measure the full bounding box of `text` rendered with `font`.
    fn text_bounds(&self, text: &str, font: &str) -> TextBounds;
}
