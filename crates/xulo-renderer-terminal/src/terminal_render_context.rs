//! [`TerminalRenderContext`] — Canvas2D-style render context for terminal output.
//!
//! Maps the [`Painter`] API to a character grid. Coordinates are in cell units;
//! fills set background colors, strokes draw border characters, and text writes
//! characters directly.

use xulo_render_core::{
    Effects, Masking, Painter, RenderContext, TextAlign, TextBaseline, TextMetrics, TextRenderer,
};

use crate::terminal_backend::{Grid, TerminalSize};

/// Saved painter state for the `save`/`restore` stack.
struct SavedState {
    fill_color: String,
    stroke_color: String,
    stroke_width: f64,
    global_alpha: f64,
    font: String,
    text_align: TextAlign,
    text_baseline: TextBaseline,
    // Transform stored as (translate_x, translate_y) — rotation/scale are
    // approximated by rounding to cell boundaries.
    translate_x: f64,
    translate_y: f64,
}

/// A path segment accumulated during path construction.
#[derive(Debug, Clone)]
enum PathOp {
    MoveTo { x: f64, y: f64 },
    LineTo { x: f64, y: f64 },
    Rect { x: f64, y: f64, w: f64, h: f64 },
    ClosePath,
}

/// Canvas2D-style render context backed by a terminal character grid.
///
/// Drawing operations build a path, then `fill()` or `stroke()` rasterizes
/// it into the grid. Text is written directly at the current position.
pub struct TerminalRenderContext {
    grid: Grid,
    size: TerminalSize,
    // Current style state
    fill_color: String,
    stroke_color: String,
    stroke_width: f64,
    global_alpha: f64,
    font: String,
    text_align: TextAlign,
    text_baseline: TextBaseline,
    // Transform
    translate_x: f64,
    translate_y: f64,
    // State stack
    state_stack: Vec<SavedState>,
    // Current path
    path: Vec<PathOp>,
    // Current point (for line_to tracking)
    current_x: f64,
    current_y: f64,
    // Path start point (for close_path)
    path_start_x: f64,
    path_start_y: f64,
    has_subpath: bool,
}

impl TerminalRenderContext {
    /// Create a new context for the given terminal size.
    pub fn new(size: TerminalSize) -> Self {
        Self {
            grid: Grid::new(size.cols, size.rows),
            size,
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
            path: Vec::new(),
            current_x: 0.0,
            current_y: 0.0,
            path_start_x: 0.0,
            path_start_y: 0.0,
            has_subpath: false,
        }
    }

    /// Consume the context and return the underlying grid.
    pub fn into_grid(self) -> Grid {
        self.grid
    }

    /// Render all accumulated ops and return the ANSI string.
    pub fn to_ansi(&self) -> String {
        self.grid.to_ansi()
    }

    /// Render all accumulated ops and return plain text.
    pub fn to_plain(&self) -> String {
        self.grid.to_plain()
    }

    /// Get a reference to the underlying grid.
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// Parse a CSS hex color string into RGB values.
    fn parse_color(color: &str) -> (u8, u8, u8) {
        let s = color.strip_prefix('#').unwrap_or(color);
        match s.len() {
            6 => {
                let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(0);
                let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0);
                let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(0);
                (r, g, b)
            }
            3 => {
                let r = u8::from_str_radix(&s[0..1], 16).unwrap_or(0);
                let g = u8::from_str_radix(&s[1..2], 16).unwrap_or(0);
                let b = u8::from_str_radix(&s[2..3], 16).unwrap_or(0);
                (r * 17, g * 17, b * 17)
            }
            _ => (0, 0, 0),
        }
    }

    fn to_color(color: &str) -> xulo_ui::Color {
        let (r, g, b) = Self::parse_color(color);
        xulo_ui::Color::new(r, g, b)
    }

    /// Apply translation to a coordinate pair.
    fn apply_translate(&self, x: f64, y: f64) -> (u32, u32) {
        let tx = (x + self.translate_x).round() as i32;
        let ty = (y + self.translate_y).round() as i32;
        (tx.max(0) as u32, ty.max(0) as u32)
    }

    /// Rasterize the current path as a filled region into the grid.
    fn rasterize_fill(&mut self) {
        let fill = Self::to_color(&self.fill_color);
        let path = std::mem::take(&mut self.path);
        let tx = self.translate_x;
        let ty = self.translate_y;

        let mut min_x = u32::MAX;
        let mut min_y = u32::MAX;
        let mut max_x = 0u32;
        let mut max_y = 0u32;
        let mut rects: Vec<xulo_ui::Rect> = Vec::new();
        let mut has_non_rect = false;

        for op in &path {
            match op {
                PathOp::Rect { x, y, w, h } => {
                    let rx = (*x + tx).round().max(0.0) as u32;
                    let ry = (*y + ty).round().max(0.0) as u32;
                    let rw = w.round() as u32;
                    let rh = h.round() as u32;
                    min_x = min_x.min(rx);
                    min_y = min_y.min(ry);
                    max_x = max_x.max(rx + rw);
                    max_y = max_y.max(ry + rh);
                    rects.push(xulo_ui::Rect::new(rx, ry, rw, rh));
                }
                PathOp::MoveTo { x, y } => {
                    let px = (*x + tx).round().max(0.0) as u32;
                    let py = (*y + ty).round().max(0.0) as u32;
                    min_x = min_x.min(px);
                    min_y = min_y.min(py);
                    max_x = max_x.max(px);
                    max_y = max_y.max(py);
                    has_non_rect = true;
                }
                PathOp::LineTo { x, y } => {
                    let px = (*x + tx).round().max(0.0) as u32;
                    let py = (*y + ty).round().max(0.0) as u32;
                    min_x = min_x.min(px);
                    min_y = min_y.min(py);
                    max_x = max_x.max(px);
                    max_y = max_y.max(py);
                    has_non_rect = true;
                }
                _ => {
                    has_non_rect = true;
                }
            }
        }

        // Fill collected rects
        for rect in &rects {
            self.fill_grid_rect(*rect, fill);
        }

        // For non-rect paths, use scanline fill on the bounding box
        if has_non_rect {
            let edges = Self::collect_edges_static(&path, tx, ty);
            self.scanline_fill_with_edges(min_x, min_y, max_x, max_y, fill, &edges);
        }

        self.path = path;
    }

    /// Simple scanline fill for arbitrary paths within a bounding box.
    fn scanline_fill_with_edges(
        &mut self,
        min_x: u32,
        min_y: u32,
        max_x: u32,
        max_y: u32,
        color: xulo_ui::Color,
        edges: &[Edge],
    ) {
        if edges.is_empty() {
            return;
        }

        // Scanline fill: for each row, find intersections with edges and fill between pairs
        for y in min_y..max_y {
            let mut intersections: Vec<u32> = Vec::new();
            for edge in edges {
                if let Some(ix) = edge.intersect_horizontal(y) {
                    intersections.push(ix);
                }
            }
            intersections.sort();
            // Fill between pairs of intersection points
            let mut i = 0;
            while i + 1 < intersections.len() {
                let x0 = intersections[i].max(min_x);
                let x1 = intersections[i + 1].min(max_x);
                for x in x0..x1 {
                    let rect = xulo_ui::Rect::new(x, y, 1, 1);
                    self.fill_grid_rect(rect, color);
                }
                i += 2;
            }
        }
    }

    /// Collect edge segments from a static path.
    fn collect_edges_static(path: &[PathOp], tx: f64, ty: f64) -> Vec<Edge> {
        let mut edges = Vec::new();
        let mut start_x = 0.0;
        let mut start_y = 0.0;
        let mut cur_x = 0.0;
        let mut cur_y = 0.0;

        for op in path {
            match op {
                PathOp::MoveTo { x, y } => {
                    start_x = *x;
                    start_y = *y;
                    cur_x = *x;
                    cur_y = *y;
                }
                PathOp::LineTo { x, y } => {
                    let x0 = (cur_x + tx).round().max(0.0) as u32;
                    let y0 = (cur_y + ty).round().max(0.0) as u32;
                    let x1 = (*x + tx).round().max(0.0) as u32;
                    let y1 = (*y + ty).round().max(0.0) as u32;
                    edges.push(Edge { x0, y0, x1, y1 });
                    cur_x = *x;
                    cur_y = *y;
                }
                PathOp::ClosePath => {
                    if (cur_x - start_x).abs() > 0.01 || (cur_y - start_y).abs() > 0.01 {
                        let x0 = (cur_x + tx).round().max(0.0) as u32;
                        let y0 = (cur_y + ty).round().max(0.0) as u32;
                        let x1 = (start_x + tx).round().max(0.0) as u32;
                        let y1 = (start_y + ty).round().max(0.0) as u32;
                        edges.push(Edge { x0, y0, x1, y1 });
                    }
                    cur_x = start_x;
                    cur_y = start_y;
                }
                _ => {}
            }
        }
        edges
    }

    /// Fill a single grid cell rectangle.
    fn fill_grid_rect(&mut self, rect: xulo_ui::Rect, color: xulo_ui::Color) {
        let clip = rect.intersect(&self.grid_bounds()).unwrap_or(rect);
        for y in clip.y..clip.bottom() {
            for x in clip.x..clip.right() {
                if self.grid.in_bounds(x, y) {
                    self.grid.set_bg(x, y, Some(color));
                }
            }
        }
    }

    /// Stroke the current path as border characters.
    fn rasterize_stroke(&mut self) {
        let color = Self::to_color(&self.stroke_color);
        let path = std::mem::take(&mut self.path);
        let tx = self.translate_x;
        let ty = self.translate_y;

        let mut rects: Vec<xulo_ui::Rect> = Vec::new();
        let mut has_non_rect = false;

        for op in &path {
            if let PathOp::Rect { x, y, w, h } = op {
                let rx = (*x + tx).round().max(0.0) as u32;
                let ry = (*y + ty).round().max(0.0) as u32;
                let rw = w.round() as u32;
                let rh = h.round() as u32;
                rects.push(xulo_ui::Rect::new(rx, ry, rw, rh));
            } else {
                has_non_rect = true;
            }
        }

        // Stroke collected rects
        for rect in &rects {
            self.stroke_grid_rect(*rect, color);
        }

        // For non-rect paths, stroke the edges
        if has_non_rect {
            let edges = Self::collect_edges_static(&path, tx, ty);
            for edge in &edges {
                self.stroke_edge(edge, color);
            }
        }

        self.path = path;
    }

    /// Draw border characters around a rectangle.
    fn stroke_grid_rect(&mut self, rect: xulo_ui::Rect, color: xulo_ui::Color) {
        let clip = rect.intersect(&self.grid_bounds()).unwrap_or(rect);
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
            self.grid.set_char(x, y0, '─', color);
            self.grid.set_char(x, y1, '─', color);
        }
        for y in y0..=y1 {
            self.grid.set_char(x0, y, '│', color);
            self.grid.set_char(x1, y, '│', color);
        }
        self.grid.set_char(x0, y0, '┌', color);
        self.grid.set_char(x1, y0, '┐', color);
        self.grid.set_char(x0, y1, '└', color);
        self.grid.set_char(x1, y1, '┘', color);
    }

    /// Stroke a single edge segment (Bresenham-style line).
    fn stroke_edge(&mut self, edge: &Edge, color: xulo_ui::Color) {
        let x0 = edge.x0 as i32;
        let y0 = edge.y0 as i32;
        let x1 = edge.x1 as i32;
        let y1 = edge.y1 as i32;
        let dx = (x1 - x0).abs();
        let dy = -(y1 - y0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        let mut x = x0;
        let mut y = y0;

        loop {
            if x >= 0 && y >= 0 {
                let ux = x as u32;
                let uy = y as u32;
                if self.grid.in_bounds(ux, uy) {
                    self.grid.set_char(ux, uy, '·', color);
                }
            }
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    fn grid_bounds(&self) -> xulo_ui::Rect {
        xulo_ui::Rect::new(0, 0, self.size.cols, self.size.rows)
    }
}

/// A line segment for scanline rasterization.
#[derive(Debug, Clone, Copy)]
struct Edge {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
}

impl Edge {
    /// Find the x-coordinate where this edge intersects the given horizontal scanline.
    fn intersect_horizontal(&self, y: u32) -> Option<u32> {
        let min_y = self.y0.min(self.y1);
        let max_y = self.y0.max(self.y1);
        if y < min_y || y > max_y {
            return None;
        }
        if self.y0 == self.y1 {
            return Some(self.x0.min(self.x1));
        }
        let t = (y as f64 - self.y0 as f64) / (self.y1 as f64 - self.y0 as f64);
        let x = self.x0 as f64 + t * (self.x1 as f64 - self.x0 as f64);
        Some(x.round() as u32)
    }
}

// ── Painter implementation ─────────────────────────────────────────────────

impl Painter for TerminalRenderContext {
    fn save(&mut self) {
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
        self.translate_x += x;
        self.translate_y += y;
    }

    fn rotate(&mut self, _angle: f64) {
        // Terminal grid doesn't support rotation — ignore.
    }

    fn scale(&mut self, _x: f64, _y: f64) {
        // Terminal grid doesn't support scaling — ignore.
    }

    fn set_fill_color(&mut self, color: &str) {
        self.fill_color = color.to_string();
    }

    fn set_global_alpha(&mut self, alpha: f64) {
        self.global_alpha = alpha.clamp(0.0, 1.0);
    }

    fn set_stroke_color(&mut self, color: &str) {
        self.stroke_color = color.to_string();
    }

    fn set_stroke_width(&mut self, width: f64) {
        self.stroke_width = width;
    }

    fn begin_path(&mut self) {
        self.path.clear();
        self.has_subpath = false;
    }

    fn move_to(&mut self, x: f64, y: f64) {
        self.path.push(PathOp::MoveTo { x, y });
        self.current_x = x;
        self.current_y = y;
        self.path_start_x = x;
        self.path_start_y = y;
        self.has_subpath = true;
    }

    fn line_to(&mut self, x: f64, y: f64) {
        self.path.push(PathOp::LineTo { x, y });
        self.current_x = x;
        self.current_y = y;
    }

    fn close_path(&mut self) {
        self.path.push(PathOp::ClosePath);
        self.current_x = self.path_start_x;
        self.current_y = self.path_start_y;
    }

    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.path.push(PathOp::Rect { x, y, w, h });
    }

    fn round_rect(&mut self, x: f64, y: f64, w: f64, h: f64, _r: f64) {
        // Terminal doesn't support rounded corners — treat as regular rect.
        self.path.push(PathOp::Rect { x, y, w, h });
    }

    fn arc(&mut self, _cx: f64, _cy: f64, _radius: f64, _start_angle: f64, _end_angle: f64) {
        // Simplified: approximate arc as a point for now.
    }

    fn quadratic_curve_to(&mut self, _cpx: f64, _cpy: f64, x: f64, y: f64) {
        self.path.push(PathOp::LineTo { x, y });
        self.current_x = x;
        self.current_y = y;
    }

    fn bezier_curve_to(&mut self, _cp1x: f64, _cp1y: f64, _cp2x: f64, _cp2y: f64, x: f64, y: f64) {
        self.path.push(PathOp::LineTo { x, y });
        self.current_x = x;
        self.current_y = y;
    }

    fn stroke(&mut self) {
        self.rasterize_stroke();
        self.path.clear();
        self.has_subpath = false;
    }

    fn fill(&mut self) {
        self.rasterize_fill();
        self.path.clear();
        self.has_subpath = false;
    }
}

// ── TextRenderer implementation ────────────────────────────────────────────

impl TextRenderer for TerminalRenderContext {
    fn set_font(&mut self, font: &str) {
        self.font = font.to_string();
    }

    fn set_text_align(&mut self, align: TextAlign) {
        self.text_align = align;
    }

    fn set_text_baseline(&mut self, baseline: TextBaseline) {
        self.text_baseline = baseline;
    }

    fn fill_text(&mut self, text: &str, x: f64, y: f64) {
        let color = Self::to_color(&self.fill_color);
        let (mut px, py) = self.apply_translate(x, y);

        // Adjust x based on text alignment
        match self.text_align {
            TextAlign::Center => {
                let w = text.chars().count() as u32;
                px = px.saturating_sub(w / 2);
            }
            TextAlign::Right => {
                let w = text.chars().count() as u32;
                px = px.saturating_sub(w);
            }
            TextAlign::Left => {}
        }

        // Adjust y based on text baseline
        let row = match self.text_baseline {
            TextBaseline::Top => py,
            TextBaseline::Middle => py.saturating_sub(0),
            TextBaseline::Alphabetic => py.saturating_sub(0),
            TextBaseline::Bottom => py.saturating_sub(0),
        };

        for (i, ch) in text.chars().enumerate() {
            let cx = px + i as u32;
            if self.grid.in_bounds(cx, row) {
                self.grid.set_char(cx, row, ch, color);
            }
        }
    }

    fn stroke_text(&mut self, text: &str, x: f64, y: f64) {
        // For terminal, stroke text is the same as fill text.
        self.fill_text(text, x, y);
    }
}

// ── TextMetrics implementation ─────────────────────────────────────────────

impl TextMetrics for TerminalRenderContext {
    fn measure_text(&self, text: &str) -> f64 {
        text.chars().count() as f64
    }

    fn text_bounds(&self, text: &str, _font: &str) -> xulo_render_core::TextBounds {
        let w = text.chars().count() as f64;
        xulo_render_core::TextBounds {
            x: 0.0,
            y: 0.0,
            w,
            h: 1.0,
            ascent: 0.0,
            descent: 0.0,
        }
    }
}

// ── Masking implementation ─────────────────────────────────────────────────

impl Masking for TerminalRenderContext {
    fn clip(&mut self) {
        // Terminal rendering uses grid bounds for clipping — no-op here.
    }
}

// ── Effects implementation ─────────────────────────────────────────────────

impl Effects for TerminalRenderContext {}

// ── RenderContext implementation ────────────────────────────────────────────

impl RenderContext for TerminalRenderContext {
    fn dpr(&self) -> f64 {
        1.0
    }
}
