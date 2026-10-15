//! Two-pass layout: measure intrinsic sizes bottom-up, place top-down.
//!
//! Layout is a single recursive walk. Leaf widgets ([`Widget::Text`],
//! [`Widget::Button`], [`Widget::Input`], [`Widget::Unknown`]) size themselves
//! from their text via [`FontMetrics`]. Container widgets decide how children
//! get their width:
//!
//! - [`Widget::VStack`] / [`Widget::Screen`]: children **fill** the container
//!   width (long text truncates).
//! - [`Widget::HStack`]: children keep their **intrinsic** width, so a nested
//!   `VStack` shrinks to its widest child instead of swallowing the row.
//!
//! The result is a [`Placed`] tree of rectangles, which [`paint`] flattens into
//! [`PaintOp`]s using a [`Theme`].

use crate::painting::PaintOp;
use crate::widgets::{
    Alignment, Axes, Color, Dimension, FontWeight, Justify, Rect, Size, StyleProps, Widget,
};

use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Scroll offsets keyed by ScrollView index (order of appearance in tree).
    static SCROLL_OFFSETS: RefCell<HashMap<usize, (i32, i32)>> = RefCell::new(HashMap::new());
    /// Counter for assigning indices to ScrollView widgets during layout.
    static SCROLL_COUNTER: RefCell<usize> = const { RefCell::new(0) };
}

/// Set the scroll offset for a ScrollView by its index.
pub fn set_scroll_offset(index: usize, x: i32, y: i32) {
    SCROLL_OFFSETS.with(|s| s.borrow_mut().insert(index, (x, y)));
}

/// Reset all scroll offsets and the counter (call before re-layout).
pub fn reset_scroll_state() {
    SCROLL_OFFSETS.with(|s| s.borrow_mut().clear());
    SCROLL_COUNTER.with(|s| *s.borrow_mut() = 0);
}

/// Get the current scroll offset for a ScrollView index.
fn scroll_offset(index: usize) -> (i32, i32) {
    SCROLL_OFFSETS.with(|s| s.borrow().get(&index).copied().unwrap_or((0, 0)))
}

/// Assign and return the next ScrollView index.
pub(crate) fn next_scroll_index() -> usize {
    SCROLL_COUNTER.with(|s| {
        let idx = *s.borrow();
        *s.borrow_mut() = idx + 1;
        idx
    })
}

/// Constraints passed from parent to child during layout (yoga-layout compatible).
///
/// These replace the old `override_h` / `width_bound` parameters with a
/// proper constraint-propagation model. The layout engine uses these to
/// determine available space for flex-grow, cross-axis stretch, and sizing.
#[derive(Debug, Clone, Copy)]
pub struct Constraints {
    /// Minimum width the child must occupy. `None` means no constraint.
    pub min_width: Option<u32>,
    /// Maximum width the child may occupy.
    pub max_width: u32,
    /// Minimum height the child must occupy. `None` means no constraint.
    pub min_height: Option<u32>,
    /// Maximum height the child may occupy. `None` means no maximum.
    pub max_height: Option<u32>,
}

/// Resolve an `Option<Dimension>` to a concrete `u32`.
///
/// - `Dimension::Pixels(n)` → `n`
/// - `Dimension::Full` → `parent_size`
/// - `None` → `fallback`
fn resolve_dim(dim: Option<Dimension>, parent_size: u32, fallback: u32) -> u32 {
    match dim {
        Some(Dimension::Pixels(n)) => n,
        Some(Dimension::Full) => parent_size,
        None => fallback,
    }
}

/// For containers (VStack, HStack, Screen): resolve height/width with
/// constraint support. `None` means use the constraint (parent size),
/// falling back to content size only when no constraint is set.
fn resolve_container_dim(
    dim: Option<Dimension>,
    constraint: Option<u32>,
    content_size: u32,
) -> u32 {
    match dim {
        Some(Dimension::Pixels(n)) => n,
        Some(Dimension::Full) => constraint.unwrap_or(content_size),
        None => constraint.unwrap_or(content_size),
    }
}

impl Constraints {
    /// Unconstrained: min=None, max=u32::MAX.
    pub fn unconstrained() -> Self {
        Self {
            min_width: None,
            max_width: u32::MAX,
            min_height: None,
            max_height: None,
        }
    }

    /// Width-constrained: max_width set, no height constraint.
    pub fn width(max: u32) -> Self {
        Self {
            min_width: None,
            max_width: max,
            min_height: None,
            max_height: None,
        }
    }

    /// Both width and height constrained.
    pub fn bounded(max_w: u32, max_h: u32) -> Self {
        Self {
            min_width: None,
            max_width: max_w,
            min_height: None,
            max_height: Some(max_h),
        }
    }

    /// With an explicit max_height override (used by Screen to propagate surface height).
    pub fn with_height(self, h: u32) -> Self {
        Self {
            min_width: self.min_width,
            max_width: self.max_width,
            min_height: self.min_height,
            max_height: Some(h),
        }
    }

    /// Clamp a width to the constraints.
    pub fn clamp_width(&self, w: u32) -> u32 {
        let w = w.min(self.max_width);
        match self.min_width {
            Some(min) => w.max(min),
            None => w,
        }
    }

    /// Clamp a height to the constraints.
    pub fn clamp_height(&self, h: u32) -> u32 {
        let h = h.min(self.max_height.unwrap_or(u32::MAX));
        match self.min_height {
            Some(min) => h.max(min),
            None => h,
        }
    }
}

/// Horizontal padding inside buttons and inputs, in layout units.
pub const PAD_X: u32 = 1;

/// Vertical padding inside buttons and inputs (border rows), in layout units.
pub const PAD_Y: u32 = 1;

/// Default font size in pixels for text rendering.
pub const DEFAULT_FONT_SIZE: u32 = 12;

/// Compute the starting offset and gap between children for justify-content.
///
/// Returns `(offset, gap)` where `offset` is the space before the first child
/// and `gap` is the space between consecutive children.
fn justify_offsets(justify: Option<Justify>, free_space: u32, n: u32) -> (u32, u32) {
    match justify {
        Some(Justify::Start) | None => (0, 0),
        Some(Justify::Center) => (free_space / 2, 0),
        Some(Justify::End) => (free_space, 0),
        Some(Justify::SpaceBetween) => {
            if n <= 1 {
                (0, 0)
            } else {
                (0, free_space / (n - 1))
            }
        }
        Some(Justify::SpaceAround) => {
            let gap = free_space / n;
            (gap / 2, gap)
        }
        Some(Justify::SpaceEvenly) => {
            let gap = free_space / (n + 1);
            (gap, gap)
        }
    }
}

/// Measures text the way the backend renders it. The terminal backend measures
/// in character cells (`text_width` = displayed columns, `line_height` = 1).
pub trait FontMetrics {
    fn text_width(&self, text: &str) -> u32;
    fn line_height(&self) -> u32;
}

/// The pixel cell metric used by pixel-resolution backends (webview, wasm): one
/// layout unit is one pixel, text is an 8×16 monospace glyph grid.
#[derive(Debug, Clone, Copy)]
pub struct CellMetrics;

impl FontMetrics for CellMetrics {
    fn text_width(&self, text: &str) -> u32 {
        text.chars().count() as u32 * crate::widgets::CELL_W
    }
    fn line_height(&self) -> u32 {
        crate::widgets::CELL_H
    }
}

/// Collect the rectangles of every `Button` in a placed tree, in pre-order
/// (matching the order the framework keeps click callbacks in).
pub fn collect_button_rects(placed: &Placed<'_>, out: &mut Vec<Rect>) {
    if matches!(placed.widget, Widget::Button { .. }) {
        out.push(placed.rect);
    }
    for child in &placed.children {
        collect_button_rects(child, out);
    }
}

/// Collect the rectangles of every `Input` in a placed tree, in pre-order.
pub fn collect_input_rects(placed: &Placed<'_>, out: &mut Vec<Rect>) {
    if matches!(placed.widget, Widget::Input { .. }) {
        out.push(placed.rect);
    }
    for child in &placed.children {
        collect_input_rects(child, out);
    }
}

/// Collect the rectangles of every `Button` and `Input` in a placed tree in a
/// single pre-order walk, returning `(buttons, inputs)`.
pub fn collect_interactive_rects(placed: &Placed<'_>) -> (Vec<Rect>, Vec<Rect>) {
    let mut buttons = Vec::new();
    let mut inputs = Vec::new();
    collect_interactive_walk(placed, &mut buttons, &mut inputs);
    (buttons, inputs)
}

fn collect_interactive_walk(placed: &Placed<'_>, buttons: &mut Vec<Rect>, inputs: &mut Vec<Rect>) {
    match placed.widget {
        Widget::Button { .. } => buttons.push(placed.rect),
        Widget::Input { .. } => inputs.push(placed.rect),
        _ => {}
    }
    for child in &placed.children {
        collect_interactive_walk(child, buttons, inputs);
    }
}

/// A widget with the rectangle it occupies. Children mirror the widget tree.
#[derive(Debug)]
pub struct Placed<'a> {
    pub widget: &'a Widget,
    pub rect: Rect,
    pub children: Vec<Placed<'a>>,
    /// Index into the scroll offset map (only set for `ScrollView` nodes).
    pub scroll_index: Option<usize>,
}

/// The visual palette used when painting a laid-out tree.
#[derive(Debug, Clone)]
pub struct Theme {
    pub background: Color,
    pub text: Color,
    pub accent: Color,
    pub border: Color,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: Color::DARK,
            text: Color::WHITE,
            accent: Color::ACCENT,
            border: Color::GRAY,
        }
    }
}

/// Layout engine interface. Implementors take a [`Widget`] tree and produce
/// a [`Placed`] tree of positioned rectangles.
///
/// Two built-in backends:
/// - [`HandwrittenLayout`] — the original hand-written Flexbox (default).
/// - [`crate::taffy_layout::TaffyLayout`] — Taffy-backed CSS Flexbox.
pub trait LayoutEngine {
    fn layout<'a>(&self, root: &'a Widget, surface: Size, metrics: &dyn FontMetrics) -> Placed<'a>;
}

/// The original hand-written Flexbox layout engine.
pub struct HandwrittenLayout;

impl LayoutEngine for HandwrittenLayout {
    fn layout<'a>(&self, root: &'a Widget, surface: Size, metrics: &dyn FontMetrics) -> Placed<'a> {
        layout(root, surface, metrics)
    }
}

/// Lay out `root` against `surface` with `metrics`, returning the placed tree.
pub fn layout<'a>(root: &'a Widget, surface: Size, metrics: &dyn FontMetrics) -> Placed<'a> {
    reset_scroll_state();
    if let Widget::Screen { .. } = root {
        // Screen always fills the whole surface. Place it with the surface
        // height as constraint so the VStack/HStack inside gets the full height
        // for flex-grow layout.
        let constraints = Constraints::width(surface.width).with_height(surface.height);
        let mut placed = place(root, (0, 0), surface.width, metrics, constraints);
        placed.rect.height = surface.height;
        placed
    } else {
        let constraints = Constraints::width(surface.width);
        place(root, (0, 0), surface.width, metrics, constraints)
    }
}

/// Flatten a placed tree into paint commands.
pub fn paint<'a>(placed: &Placed<'a>, theme: &Theme, ops: &mut Vec<PaintOp<'a>>) {
    match placed.widget {
        Widget::Screen {
            background, style, ..
        } => {
            let bg = style
                .background_color
                .or(*background)
                .unwrap_or(theme.background);
            ops.push(PaintOp::Clear { color: bg });
            for child in &placed.children {
                paint(child, theme, ops);
            }
        }
        Widget::VStack { style, .. } | Widget::HStack { style, .. } => {
            if let Some(bg) = style.background_color {
                ops.push(PaintOp::FillRect {
                    rect: placed.rect,
                    color: bg,
                    border_radius: style.border_radius.unwrap_or(0),
                    opacity: style.opacity.unwrap_or(1.0),
                });
            }
            for child in &placed.children {
                paint(child, theme, ops);
            }
        }
        Widget::Text { text, color, style } => {
            let text_color = style.color.or(*color).unwrap_or(theme.text);
            let font_size = style.font_size.unwrap_or(DEFAULT_FONT_SIZE);
            let bold = matches!(style.font_weight, Some(FontWeight::Bold));
            let br = style.border_radius.unwrap_or(0);
            let op = style.opacity.unwrap_or(1.0);
            if let Some(bg) = style.background_color {
                ops.push(PaintOp::FillRect {
                    rect: placed.rect,
                    color: bg,
                    border_radius: br,
                    opacity: op,
                });
            }
            ops.push(PaintOp::DrawText {
                rect: placed.rect,
                text,
                color: text_color,
                font_size,
                bold,
                opacity: op,
            });
        }
        Widget::Button { label, style } => {
            let border_color = style.border_color.unwrap_or(theme.border);
            let label_color = style.color.unwrap_or(theme.accent);
            let br = style.border_radius.unwrap_or(0);
            let font_size = style.font_size.unwrap_or(DEFAULT_FONT_SIZE);
            let bold = matches!(style.font_weight, Some(FontWeight::Bold));
            let op = style.opacity.unwrap_or(1.0);
            if let Some(bg) = style.background_color {
                ops.push(PaintOp::FillRect {
                    rect: placed.rect,
                    color: bg,
                    border_radius: br,
                    opacity: op,
                });
            }
            ops.push(PaintOp::DrawBorder {
                rect: placed.rect,
                color: border_color,
                border_radius: br,
                opacity: op,
            });
            ops.push(PaintOp::DrawText {
                rect: inset(placed.rect, style),
                text: label,
                color: label_color,
                font_size,
                bold,
                opacity: op,
            });
        }
        Widget::Input {
            value,
            placeholder,
            style,
            ..
        } => {
            let text_color = if value.is_empty() {
                Color::GRAY
            } else {
                style.color.unwrap_or(theme.text)
            };
            let br = style.border_radius.unwrap_or(0);
            let op = style.opacity.unwrap_or(1.0);
            if let Some(bg) = style.background_color {
                ops.push(PaintOp::FillRect {
                    rect: placed.rect,
                    color: bg,
                    border_radius: br,
                    opacity: op,
                });
            }
            if style.border_color.is_some() {
                ops.push(PaintOp::DrawBorder {
                    rect: placed.rect,
                    color: style.border_color.unwrap_or(theme.border),
                    border_radius: br,
                    opacity: op,
                });
            }
            ops.push(PaintOp::Input {
                rect: placed.rect,
                text: value,
                placeholder,
                color: text_color,
                focused: false,
                border_radius: br,
                background_color: style.background_color,
                border_color: style.border_color,
                opacity: op,
            });
        }
        Widget::Unknown { kind, style } => {
            let border_color = style.border_color.unwrap_or(theme.border);
            let text_color = style.color.unwrap_or(theme.text);
            let br = style.border_radius.unwrap_or(0);
            let font_size = style.font_size.unwrap_or(DEFAULT_FONT_SIZE);
            let bold = matches!(style.font_weight, Some(FontWeight::Bold));
            let op = style.opacity.unwrap_or(1.0);
            if let Some(bg) = style.background_color {
                ops.push(PaintOp::FillRect {
                    rect: placed.rect,
                    color: bg,
                    border_radius: br,
                    opacity: op,
                });
            }
            ops.push(PaintOp::DrawBorder {
                rect: placed.rect,
                color: border_color,
                border_radius: br,
                opacity: op,
            });
            ops.push(PaintOp::DrawText {
                rect: placed.rect,
                text: kind,
                color: text_color,
                font_size,
                bold,
                opacity: op,
            });
        }
        Widget::Fill { style, .. } => {
            // Fill is transparent — only paint its background if set, then children.
            if let Some(bg) = style.background_color {
                ops.push(PaintOp::FillRect {
                    rect: placed.rect,
                    color: bg,
                    border_radius: style.border_radius.unwrap_or(0),
                    opacity: style.opacity.unwrap_or(1.0),
                });
            }
            for child in &placed.children {
                paint(child, theme, ops);
            }
        }
        Widget::ScrollView { axes, .. } => {
            let scroll_idx = placed.scroll_index.unwrap_or(0);
            let (sx, sy) = scroll_offset(scroll_idx);
            let (ox, oy) = match axes {
                Axes::Vertical => (0, sy),
                Axes::Horizontal => (sx, 0),
                Axes::Both => (sx, sy),
            };
            ops.push(PaintOp::ScrollStart {
                viewport: placed.rect,
                offset_x: ox,
                offset_y: oy,
            });
            for child in &placed.children {
                paint(child, theme, ops);
            }
            ops.push(PaintOp::ScrollEnd);
        }
    }
}

fn inset(rect: Rect, style: &StyleProps) -> Rect {
    let (px, py) = style.effective_padding();
    let width = rect.width.saturating_sub(px * 2);
    let height = rect.height.saturating_sub(py * 2);
    Rect::new(rect.x + px, rect.y + py, width, height)
}

/// Returns `true` when a widget is a decorated Text that should expand as a
/// flex child in HStack (has bg, padding, width, or height).
fn is_flex_text(widget: &Widget) -> bool {
    matches!(widget, Widget::Text { style, .. } if style.is_text_decorated())
}

/// Extract `flex_grow` from any widget. Returns `0.0` for widgets that
/// have no `flex_grow` set (meaning they don't grow). `Fill` always
/// returns `1.0` as a convenience default.
fn widget_flex_grow(widget: &Widget) -> f32 {
    match widget {
        Widget::Fill { .. } => 1.0,
        Widget::Text { style, .. }
        | Widget::Button { style, .. }
        | Widget::Input { style, .. }
        | Widget::VStack { style, .. }
        | Widget::HStack { style, .. }
        | Widget::Screen { style, .. }
        | Widget::ScrollView { style, .. }
        | Widget::Unknown { style, .. } => style.flex_grow.unwrap_or(0.0),
    }
}

/// Apply a widget's `min_width`/`max_width`/`min_height`/`max_height` constraints
/// to a computed rectangle, clamping it within the declared bounds.
fn apply_size_constraints(rect: Rect, style: &StyleProps) -> Rect {
    let mut w = rect.width;
    let mut h = rect.height;
    if let Some(min) = style.min_width {
        w = w.max(min);
    }
    if let Some(max) = style.max_width {
        w = w.min(max);
    }
    if let Some(min) = style.min_height {
        h = h.max(min);
    }
    if let Some(max) = style.max_height {
        h = h.min(max);
    }
    if w == rect.width && h == rect.height {
        rect
    } else {
        Rect::new(rect.x, rect.y, w, h)
    }
}

/// Intrinsic size of `widget` given a width bound (`width_bound`).
fn measure(widget: &Widget, width_bound: u32, metrics: &dyn FontMetrics) -> Size {
    match widget {
        Widget::Text { text, style, .. } => {
            if style.is_text_decorated() {
                let pad = style.padding.unwrap_or(0);
                let w = metrics.text_width(text).saturating_add(pad * 2);
                let h = metrics.line_height().saturating_add(pad * 2);
                Size {
                    width: style
                        .width
                        .map(|d| d.resolve(width_bound))
                        .unwrap_or(w)
                        .min(width_bound),
                    height: style.height.map(|d| d.resolve(h)).unwrap_or(h),
                }
            } else {
                Size {
                    width: style
                        .width
                        .map(|d| d.resolve(width_bound))
                        .unwrap_or(metrics.text_width(text))
                        .min(width_bound),
                    height: style
                        .height
                        .map(|d| d.resolve(metrics.line_height()))
                        .unwrap_or(metrics.line_height()),
                }
            }
        }
        Widget::Button { label, style } => {
            let (px, py) = style.effective_padding();
            let w = metrics.text_width(label).saturating_add(px * 2);
            let h = metrics.line_height().saturating_add(py * 2);
            Size {
                width: style
                    .width
                    .map(|d| d.resolve(width_bound))
                    .unwrap_or(w)
                    .min(width_bound),
                height: style.height.map(|d| d.resolve(h)).unwrap_or(h),
            }
        }
        Widget::Input {
            value,
            width: input_width,
            placeholder,
            style,
        } => {
            let display = if value.is_empty() {
                placeholder.as_str()
            } else {
                value.as_str()
            };
            let (px, py) = style.effective_padding();
            let w = input_width
                .or(style.width.map(|d| d.resolve(width_bound)))
                .unwrap_or_else(|| metrics.text_width(display).saturating_add(px * 2));
            let h = metrics.line_height().saturating_add(py * 2);
            Size {
                width: w.min(width_bound),
                height: style.height.map(|d| d.resolve(h)).unwrap_or(h),
            }
        }
        Widget::Unknown { kind, style } => {
            let (px, py) = style.effective_padding();
            let w = metrics.text_width(kind).saturating_add(px * 2);
            let h = metrics.line_height().saturating_add(py * 2);
            Size {
                width: style
                    .width
                    .map(|d| d.resolve(width_bound))
                    .unwrap_or(w)
                    .min(width_bound),
                height: style.height.map(|d| d.resolve(h)).unwrap_or(h),
            }
        }
        Widget::VStack {
            spacing,
            children,
            style,
        } => {
            let mut size = measure_stack(children, *spacing, width_bound, metrics);
            let pad = style.padding.unwrap_or(0);
            size.width = size.width.saturating_add(pad * 2);
            size.height = size.height.saturating_add(pad * 2);
            size.width = style
                .width
                .map(|d| d.resolve(width_bound))
                .unwrap_or(size.width)
                .min(width_bound);
            size.height = style
                .height
                .map(|d| d.resolve(size.height))
                .unwrap_or(size.height);
            size
        }
        Widget::Screen {
            children, style, ..
        } => {
            let mut size = measure_stack(children, 0, width_bound, metrics);
            let pad = style.padding.unwrap_or(0);
            size.width = size.width.saturating_add(pad * 2);
            size.height = size.height.saturating_add(pad * 2);
            size.width = style
                .width
                .map(|d| d.resolve(width_bound))
                .unwrap_or(size.width)
                .min(width_bound);
            size.height = style
                .height
                .map(|d| d.resolve(size.height))
                .unwrap_or(size.height);
            size
        }
        Widget::HStack {
            spacing,
            children,
            style,
        } => {
            let mut width = 0u32;
            let mut height = 0u32;
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    width = width.saturating_add(*spacing);
                }
                let remaining = width_bound.saturating_sub(width);
                let child_size = measure(child, remaining, metrics);
                width = width.saturating_add(child_size.width);
                height = height.max(child_size.height);
            }
            let pad = style.padding.unwrap_or(0);
            width = width.saturating_add(pad * 2);
            height = height.saturating_add(pad * 2);
            Size {
                width: style
                    .width
                    .map(|d| d.resolve(width_bound))
                    .unwrap_or(width)
                    .min(width_bound),
                height: style.height.map(|d| d.resolve(height)).unwrap_or(height),
            }
        }
        Widget::Fill { children, style } => {
            // Fill has zero intrinsic size; it expands to fill remaining space.
            // Its children are measured normally for their own intrinsic sizes,
            // but Fill itself reports 0×0 to the parent.
            let _ = measure_stack(children, 0, width_bound, metrics);
            Size {
                width: style.width.map(|d| d.resolve(width_bound)).unwrap_or(0),
                height: style.height.map(|d| d.resolve(0)).unwrap_or(0),
            }
        }
        Widget::ScrollView {
            children, style, ..
        } => {
            // ScrollView is like VStack: children measured at full intrinsic
            // height, but the container clips to its viewport (height from
            // style or parent constraint).
            let mut size = measure_stack(children, 0, width_bound, metrics);
            let pad = style.padding.unwrap_or(0);
            size.width = size.width.saturating_add(pad * 2);
            size.height = size.height.saturating_add(pad * 2);
            size.width = style
                .width
                .map(|d| d.resolve(width_bound))
                .unwrap_or(size.width)
                .min(width_bound);
            size.height = style
                .height
                .map(|d| d.resolve(size.height))
                .unwrap_or(size.height);
            size
        }
    }
}

fn measure_stack(
    children: &[Widget],
    spacing: u32,
    width_bound: u32,
    metrics: &dyn FontMetrics,
) -> Size {
    let mut width = 0u32;
    let mut height = 0u32;
    for (i, child) in children.iter().enumerate() {
        if i > 0 {
            height = height.saturating_add(spacing);
        }
        let child_size = measure(child, width_bound, metrics);
        width = width.max(child_size.width);
        height = height.saturating_add(child_size.height);
    }
    Size {
        width: width.min(width_bound),
        height,
    }
}

/// Assign each widget a rectangle. `width` is the width the widget occupies:
/// `VStack`/`Screen` fill it, `HStack` shrinks to its children.
/// `constraints` propagate available space from parent to child (replaces
/// the old `override_h` / `width_bound` parameters).
fn place<'a>(
    widget: &'a Widget,
    origin: (u32, u32),
    width: u32,
    metrics: &dyn FontMetrics,
    constraints: Constraints,
) -> Placed<'a> {
    let (x, y) = origin;
    match widget {
        Widget::Text { text, style, .. } => {
            let margin = style.margin.unwrap_or(0);
            if style.is_text_decorated() {
                let pad = style.padding.unwrap_or(0);
                let text_w = metrics.text_width(text).saturating_add(pad * 2);
                let text_h = metrics.line_height().saturating_add(pad * 2);
                let w = resolve_dim(style.width, width, text_w);
                let h = resolve_dim(style.height, text_h, text_h);
                let rect = Rect::new(
                    x + margin,
                    y + margin,
                    w.min(width.saturating_sub(margin * 2)),
                    h,
                );
                Placed {
                    widget,
                    rect: apply_size_constraints(rect, style),
                    children: Vec::new(),
                    scroll_index: None,
                }
            } else {
                let text_w = metrics.text_width(text);
                let text_h = metrics.line_height();
                let w = resolve_dim(style.width, width, text_w);
                let h = resolve_dim(style.height, text_h, text_h);
                let rect = Rect::new(
                    x + margin,
                    y + margin,
                    w.min(width.saturating_sub(margin * 2)),
                    h,
                );
                Placed {
                    widget,
                    rect: apply_size_constraints(rect, style),
                    children: Vec::new(),
                    scroll_index: None,
                }
            }
        }
        Widget::Button { label, style } => {
            let margin = style.margin.unwrap_or(0);
            let (px, py) = style.effective_padding();
            let w = metrics.text_width(label).saturating_add(px * 2);
            let h = metrics.line_height().saturating_add(py * 2);
            let w = resolve_dim(style.width, width, w);
            let h = resolve_dim(style.height, h, h);
            let rect = Rect::new(
                x + margin,
                y + margin,
                w.min(width.saturating_sub(margin * 2)),
                h,
            );
            Placed {
                widget,
                rect: apply_size_constraints(rect, style),
                children: Vec::new(),
                scroll_index: None,
            }
        }
        Widget::Input {
            value,
            width: input_width,
            placeholder,
            style,
        } => {
            let display = if value.is_empty() {
                placeholder.as_str()
            } else {
                value.as_str()
            };
            let margin = style.margin.unwrap_or(0);
            let (px, py) = style.effective_padding();
            let w = input_width
                .or(style.width.map(|d| d.resolve(width)))
                .unwrap_or_else(|| metrics.text_width(display).saturating_add(px * 2));
            let h = metrics.line_height().saturating_add(py * 2);
            let rect = Rect::new(
                x + margin,
                y + margin,
                w.min(width.saturating_sub(margin * 2)),
                resolve_dim(style.height, h, h),
            );
            Placed {
                widget,
                rect: apply_size_constraints(rect, style),
                children: Vec::new(),
                scroll_index: None,
            }
        }
        Widget::Unknown { kind, style } => {
            let margin = style.margin.unwrap_or(0);
            let (px, py) = style.effective_padding();
            let w = metrics.text_width(kind).saturating_add(px * 2);
            let h = metrics.line_height().saturating_add(py * 2);
            let w = resolve_dim(style.width, width, w);
            let h = resolve_dim(style.height, h, h);
            let rect = Rect::new(
                x + margin,
                y + margin,
                w.min(width.saturating_sub(margin * 2)),
                h,
            );
            Placed {
                widget,
                rect: apply_size_constraints(rect, style),
                children: Vec::new(),
                scroll_index: None,
            }
        }
        Widget::VStack {
            spacing,
            children,
            style,
        } => {
            let margin = style.margin.unwrap_or(0);
            let pad = style.padding.unwrap_or(0);
            let inner_x = x + margin;
            let inner_y = y + margin;
            let inner_width = width.saturating_sub(margin * 2).saturating_sub(pad * 2);

            // Pass 1: measure intrinsic sizes. Flex children (flex_grow > 0)
            // have 0 intrinsic size; they expand to fill remaining space.
            let mut total_content_h = 0u32;
            let mut flex_count = 0u32;
            let mut total_flex_grow = 0.0f32;
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    total_content_h = total_content_h.saturating_add(*spacing);
                }
                let fg = widget_flex_grow(child);
                if fg > 0.0 {
                    flex_count += 1;
                    total_flex_grow += fg;
                } else {
                    let s = measure(child, inner_width, metrics);
                    total_content_h = total_content_h.saturating_add(s.height);
                }
            }

            // Container height: explicit style wins; otherwise use Screen's
            // constraint, falling back to intrinsic content height.
            let container_h =
                resolve_container_dim(style.height, constraints.max_height, total_content_h);
            let available_h = container_h.max(total_content_h);
            let free_space = available_h.saturating_sub(total_content_h);

            // Distribute remaining space among flex children.
            let (offset, gap, flex_heights) = if flex_count > 0 {
                // When flex children exist, don't apply justify to non-flex items
                // — all free space goes to flex children.
                let (o, g) = (0u32, 0u32);
                let remaining = free_space;
                // Distribute proportionally by flex_grow.
                let heights: Vec<u32> = children
                    .iter()
                    .map(|child| {
                        let fg = widget_flex_grow(child);
                        if fg > 0.0 && total_flex_grow > 0.0 && remaining > 0 {
                            (remaining as f32 * fg / total_flex_grow).floor().max(1.0) as u32
                        } else {
                            0
                        }
                    })
                    .collect();
                (o, g, heights)
            } else {
                let (o, g) = justify_offsets(style.justify, free_space, children.len() as u32);
                (o, g, vec![0u32; children.len()])
            };

            // Pass 2: place children sequentially
            let mut cursor_y = inner_y + pad + offset;
            let mut max_bottom = inner_y + pad;
            let mut placed_children = Vec::with_capacity(children.len());
            let mut prev_flex = false;
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    let is_flex = widget_flex_grow(child) > 0.0;
                    // Gap only between consecutive non-flex children.
                    if !is_flex && !prev_flex {
                        cursor_y = cursor_y.saturating_add(gap).saturating_add(*spacing);
                    } else {
                        cursor_y = cursor_y.saturating_add(*spacing);
                    }
                }
                let is_flex = widget_flex_grow(child) > 0.0;
                let child_constraints = if is_flex {
                    Constraints::width(inner_width).with_height(flex_heights[i])
                } else {
                    Constraints::width(inner_width)
                };
                let mut child_placed = place(
                    child,
                    (inner_x + pad, cursor_y),
                    inner_width,
                    metrics,
                    child_constraints,
                );
                // Flex children get the allocated height.
                if is_flex {
                    child_placed.rect.height = flex_heights[i];
                }
                // Decorated Text fills the VStack container width
                if is_flex_text(child) {
                    child_placed.rect.width = inner_width;
                }
                let (child_x, child_w) = match style.alignment {
                    Some(Alignment::Stretch) => (inner_x + pad, inner_width),
                    Some(Alignment::Center) => {
                        let cx = inner_x
                            + pad
                            + (inner_width.saturating_sub(child_placed.rect.width)) / 2;
                        (cx, child_placed.rect.width)
                    }
                    Some(Alignment::End) => {
                        let ex =
                            inner_x + pad + inner_width.saturating_sub(child_placed.rect.width);
                        (ex, child_placed.rect.width)
                    }
                    _ => (inner_x + pad, child_placed.rect.width),
                };
                child_placed.rect.x = child_x;
                child_placed.rect.width = child_w;
                cursor_y = cursor_y.saturating_add(child_placed.rect.height);
                max_bottom = max_bottom.max(cursor_y);
                prev_flex = is_flex;
                placed_children.push(child_placed);
            }
            let total_h = if style.height.is_some() {
                resolve_dim(style.height, container_h, 0)
            } else {
                max_bottom + pad - y - margin
            };
            let total_w = resolve_dim(style.width, width, width);
            let rect = Rect::new(
                x + margin,
                y + margin,
                total_w.saturating_sub(margin * 2),
                total_h,
            );
            Placed {
                widget,
                rect: apply_size_constraints(rect, style),
                children: placed_children,
                scroll_index: None,
            }
        }
        Widget::Screen {
            children, style, ..
        } => {
            let margin = style.margin.unwrap_or(0);
            let pad = style.padding.unwrap_or(0);
            let inner_x = x + margin + pad;
            let inner_y = y + margin + pad;
            let inner_width = width.saturating_sub(margin * 2).saturating_sub(pad * 2);
            let mut cursor_y = inner_y;
            let mut max_bottom = inner_y;
            let mut placed_children = Vec::with_capacity(children.len());
            for child in children.iter() {
                let child_placed = place(
                    child,
                    (inner_x, cursor_y),
                    inner_width,
                    metrics,
                    constraints,
                );
                cursor_y = cursor_y.saturating_add(child_placed.rect.height);
                max_bottom = max_bottom.max(cursor_y);
                placed_children.push(child_placed);
            }
            let total_h = max_bottom + pad - y - margin;
            let total_w = resolve_dim(style.width, width, width);
            let rect = Rect::new(
                x + margin,
                y + margin,
                total_w.saturating_sub(margin * 2),
                total_h,
            );
            Placed {
                widget,
                rect: apply_size_constraints(rect, style),
                children: placed_children,
                scroll_index: None,
            }
        }
        Widget::HStack {
            spacing,
            children,
            style,
        } => {
            let margin = style.margin.unwrap_or(0);
            let pad = style.padding.unwrap_or(0);
            let inner_x = x + margin + pad;
            let inner_y = y + margin + pad;
            let inner_width = width.saturating_sub(margin * 2).saturating_sub(pad * 2);

            // Pass 1: measure intrinsic sizes. Flex children (flex_grow > 0
            // or decorated Text) have 0 intrinsic width; they expand to fill
            // remaining space.
            let mut total_content_w = 0u32;
            let mut max_child_h = 0u32;
            let mut flex_count = 0u32;
            let mut total_flex_grow = 0.0f32;
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    total_content_w = total_content_w.saturating_add(*spacing);
                }
                let fg = widget_flex_grow(child);
                let is_flex_child = fg > 0.0 || is_flex_text(child);
                let child_size = measure(child, inner_width, metrics);
                if is_flex_child {
                    flex_count += 1;
                    total_flex_grow += if fg > 0.0 { fg } else { 1.0 };
                } else {
                    total_content_w = total_content_w.saturating_add(child_size.width);
                }
                max_child_h = max_child_h.max(child_size.height);
            }

            let container_w = resolve_dim(style.width, width, inner_width);
            let available_w = container_w.max(total_content_w);
            let free_space = available_w.saturating_sub(total_content_w);

            // Distribute remaining space among flex children.
            let (offset, gap, flex_widths) = if flex_count > 0 {
                // When flex children exist, don't apply justify to non-flex items
                // — all free space goes to flex children.
                let (o, g) = (0u32, 0u32);
                let remaining = free_space;
                let widths: Vec<u32> = children
                    .iter()
                    .map(|child| {
                        let fg = widget_flex_grow(child);
                        let is_flex_child = fg > 0.0 || is_flex_text(child);
                        let effective_fg = if fg > 0.0 { fg } else { 1.0 };
                        if is_flex_child && total_flex_grow > 0.0 && remaining > 0 {
                            (remaining as f32 * effective_fg / total_flex_grow)
                                .floor()
                                .max(1.0) as u32
                        } else {
                            0
                        }
                    })
                    .collect();
                (o, g, widths)
            } else {
                let (o, g) = justify_offsets(style.justify, free_space, children.len() as u32);
                (o, g, vec![0u32; children.len()])
            };

            // Pass 2: place children sequentially
            let container_h =
                resolve_container_dim(style.height, constraints.max_height, max_child_h);
            let mut cursor_x = inner_x + offset;
            let mut max_right = inner_x + offset;
            let mut placed_children = Vec::with_capacity(children.len());
            let mut prev_flex = false;
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    let is_flex = widget_flex_grow(child) > 0.0 || is_flex_text(child);
                    if !is_flex && !prev_flex {
                        cursor_x = cursor_x.saturating_add(gap).saturating_add(*spacing);
                    } else {
                        cursor_x = cursor_x.saturating_add(*spacing);
                    }
                }
                let is_flex = widget_flex_grow(child) > 0.0 || is_flex_text(child);
                let remaining = inner_width.saturating_sub(cursor_x - inner_x);
                let mut child_width = match child {
                    Widget::VStack { .. } | Widget::Screen { .. } => {
                        measure(child, remaining, metrics).width
                    }
                    _ => remaining,
                };
                // Flex children get the allocated width
                if is_flex {
                    child_width = flex_widths[i];
                }
                let align_height = container_h.max(max_child_h);
                let child_constraints = if is_flex {
                    Constraints::width(child_width).with_height(align_height)
                } else {
                    Constraints::width(child_width)
                };
                let (mut child_placed, child_size) = place_sized(
                    child,
                    (cursor_x, inner_y),
                    child_width,
                    metrics,
                    child_constraints,
                );
                // Override flex children width and height
                if is_flex {
                    child_placed.rect.width = flex_widths[i];
                    child_placed.rect.height = align_height;
                }
                let child_y = match style.alignment {
                    Some(Alignment::Center) => {
                        inner_y + (align_height.saturating_sub(child_size.height)) / 2
                    }
                    Some(Alignment::End) => {
                        inner_y + align_height.saturating_sub(child_size.height)
                    }
                    _ => inner_y,
                };
                child_placed.rect.y = child_y;
                cursor_x = cursor_x.saturating_add(child_placed.rect.width);
                max_right = max_right.max(cursor_x);
                prev_flex = is_flex;
                placed_children.push(child_placed);
            }
            let total_w = resolve_dim(style.width, width, max_right - x + pad + margin);
            let total_h = resolve_dim(
                style.height,
                container_h,
                max_child_h.saturating_add(pad * 2),
            );
            let rect = Rect::new(
                x + margin,
                y + margin,
                total_w.saturating_sub(margin * 2),
                total_h,
            );
            Placed {
                widget,
                rect: apply_size_constraints(rect, style),
                children: placed_children,
                scroll_index: None,
            }
        }
        Widget::Fill { children, style } => {
            // Fill is placed by its parent (VStack/HStack) which sets the rect.
            // Here we just layout the children inside the Fill's rect,
            // propagating constraints so children know the assigned height.
            let margin = style.margin.unwrap_or(0);
            let pad = style.padding.unwrap_or(0);
            let inner_x = x + margin + pad;
            let inner_y = y + margin + pad;
            let inner_width = width.saturating_sub(margin * 2).saturating_sub(pad * 2);
            let mut cursor_y = inner_y;
            let mut max_bottom = inner_y;
            let mut placed_children = Vec::with_capacity(children.len());
            for child in children.iter() {
                let child_placed = place(
                    child,
                    (inner_x, cursor_y),
                    inner_width,
                    metrics,
                    constraints,
                );
                cursor_y = cursor_y.saturating_add(child_placed.rect.height);
                max_bottom = max_bottom.max(cursor_y);
                placed_children.push(child_placed);
            }
            let total_h = resolve_dim(
                style.height,
                constraints.max_height.unwrap_or(0),
                max_bottom + pad - y - margin,
            );
            let total_w = resolve_dim(style.width, width, width);
            let rect = Rect::new(
                x + margin,
                y + margin,
                total_w.saturating_sub(margin * 2),
                total_h,
            );
            Placed {
                widget,
                rect: apply_size_constraints(rect, style),
                children: placed_children,
                scroll_index: None,
            }
        }
        Widget::ScrollView {
            children, style, ..
        } => {
            let margin = style.margin.unwrap_or(0);
            let pad = style.padding.unwrap_or(0);
            let inner_x = x + margin;
            let inner_y = y + margin;
            let inner_width = width.saturating_sub(margin * 2).saturating_sub(pad * 2);

            // Measure intrinsic content height (children at full size).
            let mut total_content_h = 0u32;
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    // no spacing for ScrollView by default
                }
                let s = measure(child, inner_width, metrics);
                total_content_h = total_content_h.saturating_add(s.height);
            }

            // Container height: explicit style wins; otherwise content size.
            let container_h =
                resolve_container_dim(style.height, constraints.max_height, total_content_h);

            // Place children sequentially at full intrinsic positions (not
            // clipped — clipping happens at paint time via ScrollStart).
            let mut cursor_y = inner_y + pad;
            let mut placed_children = Vec::with_capacity(children.len());
            for child in children.iter() {
                let child_placed = place(
                    child,
                    (inner_x + pad, cursor_y),
                    inner_width,
                    metrics,
                    Constraints::width(inner_width),
                );
                cursor_y = cursor_y.saturating_add(child_placed.rect.height);
                placed_children.push(child_placed);
            }

            let total_h = resolve_dim(
                style.height,
                container_h,
                total_content_h.saturating_add(pad * 2),
            );
            let total_w = resolve_dim(style.width, width, width);
            let rect = Rect::new(
                x + margin,
                y + margin,
                total_w.saturating_sub(margin * 2),
                total_h,
            );

            let scroll_idx = next_scroll_index();
            Placed {
                widget,
                rect: apply_size_constraints(rect, style),
                children: placed_children,
                scroll_index: Some(scroll_idx),
            }
        }
    }
}

/// Like `place`, but also returns the widget's measured size, avoiding a
/// separate `measure` call for HStack children that need intrinsic width.
fn place_sized<'a>(
    widget: &'a Widget,
    origin: (u32, u32),
    width: u32,
    metrics: &dyn FontMetrics,
    constraints: Constraints,
) -> (Placed<'a>, Size) {
    let placed = place(widget, origin, width, metrics, constraints);
    let size = Size {
        width: placed.rect.width,
        height: placed.rect.height,
    };
    (placed, size)
}
