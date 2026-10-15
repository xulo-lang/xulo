//! [`Canvas2dRenderContext`] — Canvas2D-style render context for webview output.
//!
//! Accumulates JavaScript code strings that implement the [`Painter`] API
//! using the browser's `CanvasRenderingContext2D`. The generated JS can be
//! embedded in an HTML page and executed inside a webview.

use xulo_render_core::{
    Effects, Masking, Painter, RenderContext, TextAlign, TextBaseline, TextMetrics, TextRenderer,
};

/// Saved painter state for the `save`/`restore` stack.
struct SavedState {
    fill_color: String,
    stroke_color: String,
    stroke_width: f64,
    global_alpha: f64,
    font: String,
    text_align: TextAlign,
    text_baseline: TextBaseline,
    translate_x: f64,
    translate_y: f64,
}

/// Canvas2D-style render context that generates JavaScript code.
///
/// Each draw call appends a JS statement to an internal `String` buffer.
/// The generated code expects a `ctx` variable holding a
/// `CanvasRenderingContext2D`.
pub struct Canvas2dRenderContext {
    js: String,
    fill_color: String,
    stroke_color: String,
    stroke_width: f64,
    global_alpha: f64,
    font: String,
    text_align: TextAlign,
    text_baseline: TextBaseline,
    translate_x: f64,
    translate_y: f64,
    state_stack: Vec<SavedState>,
    /// Path accumulator — collects sub-path commands.
    path_js: String,
    has_subpath: bool,
}

impl Canvas2dRenderContext {
    /// Create a new context.
    pub fn new() -> Self {
        Self {
            js: String::new(),
            fill_color: "#000000".to_string(),
            stroke_color: "#ffffff".to_string(),
            stroke_width: 1.0,
            global_alpha: 1.0,
            font: "12px monospace".to_string(),
            text_align: TextAlign::Left,
            text_baseline: TextBaseline::Top,
            translate_x: 0.0,
            translate_y: 0.0,
            state_stack: Vec::new(),
            path_js: String::new(),
            has_subpath: false,
        }
    }

    /// Consume the context and return the accumulated JavaScript code.
    pub fn finish(self) -> String {
        self.js
    }

    /// Get a reference to the accumulated JavaScript code.
    pub fn js(&self) -> &str {
        &self.js
    }

    /// Append a raw JS statement.
    fn emit(&mut self, code: &str) {
        self.js.push_str(code);
        self.js.push('\n');
    }

    /// Escape a string for embedding in JS.
    fn js_str(s: &str) -> String {
        let mut out = String::from('"');
        for ch in s.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if c.is_control() => {
                    out.push_str(&format!("\\u{:04x}", c as u32));
                }
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }
}

impl Default for Canvas2dRenderContext {
    fn default() -> Self {
        Self::new()
    }
}

// ── Painter implementation ─────────────────────────────────────────────────

impl Painter for Canvas2dRenderContext {
    fn save(&mut self) {
        self.emit("ctx.save();");
        self.state_stack.push(SavedState {
            fill_color: self.fill_color.clone(),
            stroke_color: self.stroke_color.clone(),
            stroke_width: self.stroke_width,
            global_alpha: self.global_alpha,
            font: self.font.clone(),
            text_align: self.text_align,
            text_baseline: self.text_baseline,
            translate_x: self.translate_x,
            translate_y: self.translate_y,
        });
    }

    fn restore(&mut self) {
        self.emit("ctx.restore();");
        if let Some(state) = self.state_stack.pop() {
            self.fill_color = state.fill_color;
            self.stroke_color = state.stroke_color;
            self.stroke_width = state.stroke_width;
            self.global_alpha = state.global_alpha;
            self.font = state.font;
            self.text_align = state.text_align;
            self.text_baseline = state.text_baseline;
            self.translate_x = state.translate_x;
            self.translate_y = state.translate_y;
        }
    }

    fn translate(&mut self, x: f64, y: f64) {
        self.emit(&format!("ctx.translate({x}, {y});"));
        self.translate_x += x;
        self.translate_y += y;
    }

    fn rotate(&mut self, angle: f64) {
        self.emit(&format!("ctx.rotate({angle});"));
    }

    fn scale(&mut self, x: f64, y: f64) {
        self.emit(&format!("ctx.scale({x}, {y});"));
    }

    fn set_fill_color(&mut self, color: &str) {
        self.fill_color = color.to_string();
        self.emit(&format!("ctx.fillStyle = {};", Self::js_str(color)));
    }

    fn set_global_alpha(&mut self, alpha: f64) {
        self.global_alpha = alpha.clamp(0.0, 1.0);
        self.emit(&format!("ctx.globalAlpha = {};", self.global_alpha));
    }

    fn set_stroke_color(&mut self, color: &str) {
        self.stroke_color = color.to_string();
        self.emit(&format!("ctx.strokeStyle = {};", Self::js_str(color)));
    }

    fn set_stroke_width(&mut self, width: f64) {
        self.stroke_width = width;
        self.emit(&format!("ctx.lineWidth = {width};"));
    }

    fn begin_path(&mut self) {
        self.emit("ctx.beginPath();");
        self.path_js.clear();
        self.has_subpath = false;
    }

    fn move_to(&mut self, x: f64, y: f64) {
        self.emit(&format!("ctx.moveTo({x}, {y});"));
        self.has_subpath = true;
    }

    fn line_to(&mut self, x: f64, y: f64) {
        self.emit(&format!("ctx.lineTo({x}, {y});"));
    }

    fn close_path(&mut self) {
        self.emit("ctx.closePath();");
    }

    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.emit(&format!("ctx.rect({x}, {y}, {w}, {h});"));
    }

    fn round_rect(&mut self, x: f64, y: f64, w: f64, h: f64, r: f64) {
        self.emit(&format!("ctx.roundRect({x}, {y}, {w}, {h}, {r});"));
    }

    fn arc(&mut self, cx: f64, cy: f64, radius: f64, start_angle: f64, end_angle: f64) {
        self.emit(&format!(
            "ctx.arc({cx}, {cy}, {radius}, {start_angle}, {end_angle});"
        ));
    }

    fn quadratic_curve_to(&mut self, cpx: f64, cpy: f64, x: f64, y: f64) {
        self.emit(&format!("ctx.quadraticCurveTo({cpx}, {cpy}, {x}, {y});"));
    }

    fn bezier_curve_to(&mut self, cp1x: f64, cp1y: f64, cp2x: f64, cp2y: f64, x: f64, y: f64) {
        self.emit(&format!(
            "ctx.bezierCurveTo({cp1x}, {cp1y}, {cp2x}, {cp2y}, {x}, {y});"
        ));
    }

    fn stroke(&mut self) {
        self.emit("ctx.stroke();");
    }

    fn fill(&mut self) {
        self.emit("ctx.fill();");
    }
}

// ── TextRenderer implementation ────────────────────────────────────────────

impl TextRenderer for Canvas2dRenderContext {
    fn set_font(&mut self, font: &str) {
        self.font = font.to_string();
        self.emit(&format!("ctx.font = {};", Self::js_str(font)));
    }

    fn set_text_align(&mut self, align: TextAlign) {
        self.text_align = align;
        let s = match align {
            TextAlign::Left => "left",
            TextAlign::Center => "center",
            TextAlign::Right => "right",
        };
        self.emit(&format!("ctx.textAlign = \"{s}\";"));
    }

    fn set_text_baseline(&mut self, baseline: TextBaseline) {
        self.text_baseline = baseline;
        let s = match baseline {
            TextBaseline::Top => "top",
            TextBaseline::Middle => "middle",
            TextBaseline::Alphabetic => "alphabetic",
            TextBaseline::Bottom => "bottom",
        };
        self.emit(&format!("ctx.textBaseline = \"{s}\";"));
    }

    fn fill_text(&mut self, text: &str, x: f64, y: f64) {
        self.emit(&format!("ctx.fillText({}, {x}, {y});", Self::js_str(text)));
    }

    fn stroke_text(&mut self, text: &str, x: f64, y: f64) {
        self.emit(&format!(
            "ctx.strokeText({}, {x}, {y});",
            Self::js_str(text)
        ));
    }
}

// ── TextMetrics implementation ─────────────────────────────────────────────

impl TextMetrics for Canvas2dRenderContext {
    fn measure_text(&self, text: &str) -> f64 {
        // Approximate: use character count × 8px (monospace assumption).
        text.chars().count() as f64 * 8.0
    }

    fn text_bounds(&self, text: &str, _font: &str) -> xulo_render_core::TextBounds {
        let w = text.chars().count() as f64 * 8.0;
        xulo_render_core::TextBounds {
            x: 0.0,
            y: 0.0,
            w,
            h: 16.0,
            ascent: 12.0,
            descent: 4.0,
        }
    }
}

// ── Masking implementation ─────────────────────────────────────────────────

impl Masking for Canvas2dRenderContext {
    fn clip(&mut self) {
        self.emit("ctx.clip();");
    }
}

// ── Effects implementation ─────────────────────────────────────────────────

impl Effects for Canvas2dRenderContext {
    fn set_shadow_offset(&mut self, x: f64, y: f64) {
        self.emit(&format!(
            "ctx.shadowOffsetX = {x}; ctx.shadowOffsetY = {y};"
        ));
    }

    fn set_shadow_blur(&mut self, radius: f64) {
        self.emit(&format!("ctx.shadowBlur = {radius};"));
    }

    fn set_shadow_color(&mut self, color: &str) {
        self.emit(&format!("ctx.shadowColor = {};", Self::js_str(color)));
    }

    fn clear_shadow(&mut self) {
        self.emit("ctx.shadowOffsetX = 0; ctx.shadowOffsetY = 0; ctx.shadowBlur = 0; ctx.shadowColor = 'transparent';");
    }
}

// ── RenderContext implementation ────────────────────────────────────────────

impl RenderContext for Canvas2dRenderContext {
    fn dpr(&self) -> f64 {
        // DPR is handled by the HTML page's devicePixelRatio.
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use xulo_render_core::Painter;

    #[test]
    fn emits_fill_rect() {
        let mut ctx = Canvas2dRenderContext::new();
        ctx.set_fill_color("#ff0000");
        ctx.begin_path();
        ctx.rect(10.0, 20.0, 100.0, 50.0);
        ctx.fill();
        let js = ctx.finish();
        assert!(js.contains("ctx.fillStyle = \"#ff0000\""));
        assert!(js.contains("ctx.rect(10, 20, 100, 50)"));
        assert!(js.contains("ctx.fill()"));
    }

    #[test]
    fn emits_round_rect() {
        let mut ctx = Canvas2dRenderContext::new();
        ctx.begin_path();
        ctx.round_rect(0.0, 0.0, 200.0, 100.0, 8.0);
        ctx.stroke();
        let js = ctx.finish();
        assert!(js.contains("ctx.roundRect(0, 0, 200, 100, 8)"));
        assert!(js.contains("ctx.stroke()"));
    }

    #[test]
    fn emits_save_restore() {
        let mut ctx = Canvas2dRenderContext::new();
        ctx.save();
        ctx.set_fill_color("#00ff00");
        ctx.restore();
        let js = ctx.finish();
        assert!(js.contains("ctx.save()"));
        assert!(js.contains("ctx.restore()"));
    }
}
