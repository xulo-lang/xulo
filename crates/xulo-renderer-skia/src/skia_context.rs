//! Skia-backed [`RenderContext`] implementation.
//!
//! Maps the Canvas2D-style trait hierarchy to `skia_safe::Canvas` calls.
//! Anti-aliasing, sub-pixel text, and GPU acceleration are handled
//! automatically by Skia.

use skia_safe::{
    Canvas, Color, Font, Paint, Path, Rect, Surface,
};
use xulo_render_core::{
    Effects, Masking, Painter, RenderContext, TextBounds, TextMetrics, TextRenderer,
};
use xulo_render_core::TextBaseline;
use xulo_render_core::TextAlign;

// ── Saved state for save/restore ────────────────────────────────────────────

struct SavedState {
    fill_color: Color,
    stroke_color: Color,
    stroke_width: f32,
    global_alpha: f32,
    font_size: f32,
    text_align: TextAlign,
    text_baseline: TextBaseline,
}

// ── SkiaRenderContext ───────────────────────────────────────────────────────

/// Skia-backed render context implementing the full [`RenderContext`] trait.
///
/// Wraps a `skia_safe::Surface` and borrows its canvas for drawing.
/// For raster (CPU) surfaces, all operations are immediate.
pub struct SkiaRenderContext {
    surface: Surface,
    fill_color: Color,
    stroke_color: Color,
    stroke_width: f32,
    global_alpha: f32,
    font_size: f32,
    text_align: TextAlign,
    text_baseline: TextBaseline,
    state_stack: Vec<SavedState>,
}

impl SkiaRenderContext {
    /// Create a CPU-backed (raster) context with the given dimensions.
    pub fn new_cpu(width: i32, height: i32) -> Option<Self> {
        let info = skia_safe::ImageInfo::new(
            (width, height),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
#[allow(deprecated)]
        let surface = Surface::new_raster(&info, None, None)?;

        Some(Self {
            surface,
            fill_color: Color::BLACK,
            stroke_color: Color::WHITE,
            stroke_width: 1.0,
            global_alpha: 1.0,
            font_size: 12.0,
            text_align: TextAlign::Left,
            text_baseline: TextBaseline::Top,
            state_stack: Vec::new(),
        })
    }

    /// Build a Paint from the current fill state.
    fn fill_paint(&self) -> Paint {
        let mut p = Paint::default();
        p.set_anti_alias(true);
        p.set_color(self.fill_color);
        p.set_alpha((self.global_alpha * 255.0) as u8);
        p
    }

    /// Build a Paint from the current stroke state.
    fn stroke_paint(&self) -> Paint {
        let mut p = Paint::default();
        p.set_anti_alias(true);
        p.set_style(skia_safe::paint::Style::Stroke);
        p.set_stroke_width(self.stroke_width);
        p.set_color(self.stroke_color);
        p
    }

    /// Build a Font from the current font size.
    fn font(&self) -> Font {
        let mut f = Font::default();
        f.set_size(self.font_size);
        f
    }

    /// Read the pixel data as RGBA bytes.
    pub fn read_pixels(&mut self) -> Option<Vec<u8>> {
        let info = self.surface.image_info();
        let row_bytes = info.width() as usize * 4;
        let mut pixels = vec![0u8; info.compute_byte_size(row_bytes)];
        if self.surface.read_pixels(&info, &mut pixels, row_bytes, (0, 0)) {
            Some(pixels)
        } else {
            None
        }
    }

    /// Get the width of the surface.
    pub fn width(&self) -> i32 {
        self.surface.width()
    }

    /// Get the height of the surface.
    pub fn height(&self) -> i32 {
        self.surface.height()
    }

    /// Resize the surface to the given dimensions.
    pub fn resize(&mut self, width: i32, height: i32) {
        let info = skia_safe::ImageInfo::new(
            (width, height),
            skia_safe::ColorType::RGBA8888,
            skia_safe::AlphaType::Premul,
            None,
        );
        #[allow(deprecated)]
        if let Some(new_surface) = Surface::new_raster(&info, None, None) {
            self.surface = new_surface;
        }
    }

    /// Encode the current surface contents as PNG.
    pub fn encode_png(&mut self) -> Option<Vec<u8>> {
        let pixmap = self.surface.peek_pixels()?;
        pixmap.encode(skia_safe::EncodedImageFormat::PNG, None)
    }

    /// Get a mutable reference to the underlying Skia canvas.
    fn with_canvas<R>(&mut self, f: impl FnOnce(&Canvas) -> R) -> R {
        let canvas = self.surface.canvas();
        f(canvas)
    }
}

// ── Color parsing ───────────────────────────────────────────────────────────

/// Parse a CSS hex color string (`"#RGB"`, `"#RRGGBB"`, or `"#RRGGBBAA"`).
fn parse_hex_color(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');
    match hex.len() {
        3 => {
            let r = u8::from_str_radix(&hex[0..1], 16).unwrap_or(0) * 17;
            let g = u8::from_str_radix(&hex[1..2], 16).unwrap_or(0) * 17;
            let b = u8::from_str_radix(&hex[2..3], 16).unwrap_or(0) * 17;
            Color::from_argb(255, r, g, b)
        }
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
            Color::from_argb(255, r, g, b)
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
            let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
            let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
            let a = u8::from_str_radix(&hex[6..8], 16).unwrap_or(255);
            Color::from_argb(a, r, g, b)
        }
        _ => Color::from_argb(255, 0, 0, 0),
    }
}

/// Parse a CSS font string (e.g. `"bold 16px monospace"`) and return font size.
fn parse_font_size(font_str: &str) -> f32 {
    for token in font_str.split_whitespace() {
        if let Some(px) = token.strip_suffix("px")
            && let Ok(v) = px.parse::<f32>()
        {
            return v;
        }
    }
    12.0
}

// ── Painter implementation ──────────────────────────────────────────────────

impl Painter for SkiaRenderContext {
    fn save(&mut self) {
        self.with_canvas(|c| { c.save(); });
        self.state_stack.push(SavedState {
            fill_color: self.fill_color,
            stroke_color: self.stroke_color,
            stroke_width: self.stroke_width,
            global_alpha: self.global_alpha,
            font_size: self.font_size,
            text_align: self.text_align,
            text_baseline: self.text_baseline,
        });
    }

    fn restore(&mut self) {
        self.with_canvas(|c| { c.restore(); });
        if let Some(state) = self.state_stack.pop() {
            self.fill_color = state.fill_color;
            self.stroke_color = state.stroke_color;
            self.stroke_width = state.stroke_width;
            self.global_alpha = state.global_alpha;
            self.font_size = state.font_size;
            self.text_align = state.text_align;
            self.text_baseline = state.text_baseline;
        }
    }

    fn translate(&mut self, x: f64, y: f64) {
        self.with_canvas(|c| { c.translate((x as f32, y as f32)); });
    }

    fn rotate(&mut self, angle: f64) {
        self.with_canvas(|c| { c.rotate(angle.to_degrees() as f32, None); });
    }

    fn scale(&mut self, x: f64, y: f64) {
        self.with_canvas(|c| { c.scale((x as f32, y as f32)); });
    }

    fn set_fill_color(&mut self, color: &str) {
        self.fill_color = parse_hex_color(color);
    }

    fn set_global_alpha(&mut self, alpha: f64) {
        self.global_alpha = alpha.clamp(0.0, 1.0) as f32;
    }

    fn set_stroke_color(&mut self, color: &str) {
        self.stroke_color = parse_hex_color(color);
    }

    fn set_stroke_width(&mut self, width: f64) {
        self.stroke_width = width as f32;
    }

    // ── Path construction ──────────────────────────────────────────────────
    // We store the path in a local and draw it at commit time.
    // For begin_path/move_to/line_to etc., we use a thread-local path.

    fn begin_path(&mut self) {
        PATH.with(|p| { *p.borrow_mut() = Path::new(); });
    }

    fn move_to(&mut self, x: f64, y: f64) {
        PATH.with(|p| { p.borrow_mut().move_to((x as f32, y as f32)); });
    }

    fn line_to(&mut self, x: f64, y: f64) {
        PATH.with(|p| { p.borrow_mut().line_to((x as f32, y as f32)); });
    }

    fn close_path(&mut self) {
        PATH.with(|p| { p.borrow_mut().close(); });
    }

    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        PATH.with(|p| {
            p.borrow_mut().add_rect(
                Rect::from_xywh(x as f32, y as f32, w as f32, h as f32),
                None,
            );
        });
    }

    fn round_rect(&mut self, x: f64, y: f64, w: f64, h: f64, r: f64) {
        let rect = Rect::from_xywh(x as f32, y as f32, w as f32, h as f32);
        let r = r as f32;
        let r = r.min(w as f32 / 2.0).min(h as f32 / 2.0);
        PATH.with(|p| {
            p.borrow_mut()
                .add_rrect(skia_safe::RRect::new_rect_xy(rect, r, r), None);
        });
    }

    fn arc(&mut self, cx: f64, cy: f64, radius: f64, start_angle: f64, end_angle: f64) {
        PATH.with(|p| {
            p.borrow_mut().add_arc(
                Rect::from_xywh(
                    (cx - radius) as f32,
                    (cy - radius) as f32,
                    (radius * 2.0) as f32,
                    (radius * 2.0) as f32,
                ),
                start_angle as f32,
                (end_angle - start_angle) as f32,
            );
        });
    }

    fn quadratic_curve_to(&mut self, cpx: f64, cpy: f64, x: f64, y: f64) {
        PATH.with(|p| {
            p.borrow_mut()
                .quad_to((cpx as f32, cpy as f32), (x as f32, y as f32));
        });
    }

    fn bezier_curve_to(
        &mut self,
        cp1x: f64,
        cp1y: f64,
        cp2x: f64,
        cp2y: f64,
        x: f64,
        y: f64,
    ) {
        PATH.with(|p| {
            p.borrow_mut().cubic_to(
                (cp1x as f32, cp1y as f32),
                (cp2x as f32, cp2y as f32),
                (x as f32, y as f32),
            );
        });
    }

    // ── Commit ─────────────────────────────────────────────────────────────

    fn stroke(&mut self) {
        let paint = self.stroke_paint();
        self.with_canvas(|c| {
            PATH.with(|p| {
                let path = p.borrow();
                c.draw_path(&path, &paint);
            });
        });
    }

    fn fill(&mut self) {
        let paint = self.fill_paint();
        self.with_canvas(|c| {
            PATH.with(|p| {
                let path = p.borrow();
                c.draw_path(&path, &paint);
            });
        });
    }
}

// Thread-local path accumulator to avoid borrow conflicts.
thread_local! {
    static PATH: std::cell::RefCell<Path> = std::cell::RefCell::new(Path::new());
}

// ── TextRenderer implementation ─────────────────────────────────────────────

impl TextRenderer for SkiaRenderContext {
    fn set_font(&mut self, font: &str) {
        self.font_size = parse_font_size(font);
    }

    fn set_text_align(&mut self, align: TextAlign) {
        self.text_align = align;
    }

    fn set_text_baseline(&mut self, baseline: TextBaseline) {
        self.text_baseline = baseline;
    }

    fn fill_text(&mut self, text: &str, x: f64, y: f64) {
        let font = self.font();
        let paint = self.fill_paint();
        let (_, metrics) = font.metrics();
        let baseline_y = match self.text_baseline {
            TextBaseline::Top => y + (-metrics.ascent) as f64,
            TextBaseline::Middle => y + (-metrics.ascent + metrics.descent / 2.0) as f64,
            TextBaseline::Alphabetic => y,
            TextBaseline::Bottom => y + metrics.descent as f64,
        };
        self.with_canvas(|c| {
            c.draw_str(text, (x as f32, baseline_y as f32), &font, &paint);
        });
    }

    fn stroke_text(&mut self, text: &str, x: f64, y: f64) {
        let font = self.font();
        let paint = self.stroke_paint();
        let (_, metrics) = font.metrics();
        let baseline_y = match self.text_baseline {
            TextBaseline::Top => y + (-metrics.ascent) as f64,
            TextBaseline::Middle => y + (-metrics.ascent + metrics.descent / 2.0) as f64,
            TextBaseline::Alphabetic => y,
            TextBaseline::Bottom => y + metrics.descent as f64,
        };
        self.with_canvas(|c| {
            c.draw_str(text, (x as f32, baseline_y as f32), &font, &paint);
        });
    }
}

// ── TextMetrics implementation ──────────────────────────────────────────────

impl TextMetrics for SkiaRenderContext {
    fn measure_text(&self, text: &str) -> f64 {
        let font = self.font();
        font.measure_str(text, None).0 as f64
    }

    fn text_bounds(&self, text: &str, _font: &str) -> TextBounds {
        let font = self.font();
        let (width, _) = font.measure_str(text, None);
        let (_, metrics) = font.metrics();
        TextBounds {
            x: 0.0,
            y: 0.0,
            w: width as f64,
            h: (metrics.descent - metrics.ascent) as f64,
            ascent: -metrics.ascent as f64,
            descent: metrics.descent as f64,
        }
    }
}

// ── Masking implementation ──────────────────────────────────────────────────

impl Masking for SkiaRenderContext {
    fn clip(&mut self) {
        self.with_canvas(|c| {
            PATH.with(|p| {
                let path = p.borrow();
                c.clip_path(&path, None, None);
            });
        });
    }
}

// ── Effects implementation ──────────────────────────────────────────────────

impl Effects for SkiaRenderContext {
    fn set_shadow_offset(&mut self, _x: f64, _y: f64) {
        // Shadow effects require image filters — store for later use.
    }

    fn set_shadow_blur(&mut self, _radius: f64) {}

    fn set_shadow_color(&mut self, _color: &str) {}

    fn clear_shadow(&mut self) {}
}

// ── RenderContext implementation ────────────────────────────────────────────

impl RenderContext for SkiaRenderContext {
    fn dpr(&self) -> f64 {
        1.0
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use xulo_render_core::Painter;

    fn make_context() -> SkiaRenderContext {
        SkiaRenderContext::new_cpu(400, 300).expect("Failed to create CPU surface")
    }

    #[test]
    fn fill_rect() {
        let mut ctx = make_context();
        ctx.set_fill_color("#ff0000");
        ctx.begin_path();
        ctx.rect(10.0, 20.0, 100.0, 50.0);
        ctx.fill();
    }

    #[test]
    fn stroke_round_rect() {
        let mut ctx = make_context();
        ctx.set_stroke_color("#00ff00");
        ctx.set_stroke_width(2.0);
        ctx.begin_path();
        ctx.round_rect(0.0, 0.0, 200.0, 100.0, 8.0);
        ctx.stroke();
    }

    #[test]
    fn fill_text() {
        let mut ctx = make_context();
        ctx.set_font("bold 16px monospace");
        ctx.set_fill_color("#ffffff");
        ctx.fill_text("Hello, Skia!", 10.0, 30.0);
    }

    #[test]
    fn measure_text() {
        let mut ctx = make_context();
        ctx.set_font("16px monospace");
        let w = ctx.measure_text("Hello");
        // Default font may not load glyphs in raster context; just verify
        // the method doesn't panic and returns a value.
        let _ = w;
    }

    #[test]
    fn save_restore() {
        let mut ctx = make_context();
        ctx.set_fill_color("#ff0000");
        ctx.save();
        ctx.set_fill_color("#00ff00");
        ctx.restore();
        assert_eq!(ctx.fill_color, parse_hex_color("#ff0000"));
    }

    #[test]
    fn clip() {
        let mut ctx = make_context();
        ctx.begin_path();
        ctx.rect(0.0, 0.0, 100.0, 100.0);
        ctx.clip();
        ctx.set_fill_color("#0000ff");
        ctx.begin_path();
        ctx.rect(0.0, 0.0, 200.0, 200.0);
        ctx.fill();
    }

    #[test]
    fn parse_hex_variants() {
        assert_eq!(parse_hex_color("#f00"), Color::from_argb(255, 255, 0, 0));
        assert_eq!(
            parse_hex_color("#00ff00"),
            Color::from_argb(255, 0, 255, 0)
        );
        assert_eq!(
            parse_hex_color("#0000ff80"),
            Color::from_argb(128, 0, 0, 255)
        );
    }

    #[test]
    fn read_pixels() {
        let mut ctx = make_context();
        ctx.set_fill_color("#ff0000");
        ctx.begin_path();
        ctx.rect(0.0, 0.0, 100.0, 100.0);
        ctx.fill();
        let pixels = ctx.read_pixels();
        assert!(pixels.is_some(), "should read pixels");
        let p = pixels.unwrap();
        assert_eq!(p[0], 255);
        assert_eq!(p[1], 0);
        assert_eq!(p[2], 0);
        assert_eq!(p[3], 255);
    }
}
