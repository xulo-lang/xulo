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
use crate::widgets::{Color, Rect, Size, StyleProps, Widget};

/// Horizontal padding inside buttons and inputs, in layout units.
pub const PAD_X: u32 = 1;

/// Vertical padding inside buttons and inputs (border rows), in layout units.
pub const PAD_Y: u32 = 1;

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

/// Lay out `root` against `surface` with `metrics`, returning the placed tree.
pub fn layout<'a>(root: &'a Widget, surface: Size, metrics: &dyn FontMetrics) -> Placed<'a> {
    let mut placed = place(root, (0, 0), surface.width, metrics);
    if matches!(root, Widget::Screen { .. }) {
        // A Screen always fills the whole surface, even when its content is
        // shorter (the background clear covers the remaining area).
        placed.rect.height = surface.height;
    }
    placed
}

/// Flatten a placed tree into paint commands.
pub fn paint<'a>(placed: &Placed<'a>, theme: &Theme, ops: &mut Vec<PaintOp<'a>>) {
    match placed.widget {
        Widget::Screen { background, style, .. } => {
            let bg = style.background_color.or(*background).unwrap_or(theme.background);
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
                });
            }
            for child in &placed.children {
                paint(child, theme, ops);
            }
        }
        Widget::Text { text, color, style } => {
            let text_color = style.color.or(*color).unwrap_or(theme.text);
            ops.push(PaintOp::DrawText {
                rect: placed.rect,
                text,
                color: text_color,
            });
        }
        Widget::Button { label, style } => {
            let border_color = style.border_color.unwrap_or(theme.border);
            let label_color = style.color.unwrap_or(theme.accent);
            let br = style.border_radius.unwrap_or(0);
            if let Some(bg) = style.background_color {
                ops.push(PaintOp::FillRect {
                    rect: placed.rect,
                    color: bg,
                    border_radius: br,
                });
            }
            ops.push(PaintOp::DrawBorder {
                rect: placed.rect,
                color: border_color,
                border_radius: br,
            });
            ops.push(PaintOp::DrawText {
                rect: inset(placed.rect, style),
                text: label,
                color: label_color,
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
            if let Some(bg) = style.background_color {
                ops.push(PaintOp::FillRect {
                    rect: placed.rect,
                    color: bg,
                    border_radius: br,
                });
            }
            if style.border_color.is_some() {
                ops.push(PaintOp::DrawBorder {
                    rect: placed.rect,
                    color: style.border_color.unwrap_or(theme.border),
                    border_radius: br,
                });
            }
            ops.push(PaintOp::Input {
                rect: placed.rect,
                text: value,
                placeholder,
                color: text_color,
                focused: false,
                border_radius: br,
            });
        }
        Widget::Unknown { kind, style } => {
            let border_color = style.border_color.unwrap_or(theme.border);
            let text_color = style.color.unwrap_or(theme.text);
            let br = style.border_radius.unwrap_or(0);
            if let Some(bg) = style.background_color {
                ops.push(PaintOp::FillRect {
                    rect: placed.rect,
                    color: bg,
                    border_radius: br,
                });
            }
            ops.push(PaintOp::DrawBorder {
                rect: placed.rect,
                color: border_color,
                border_radius: br,
            });
            ops.push(PaintOp::DrawText {
                rect: placed.rect,
                text: kind,
                color: text_color,
            });
        }
    }
}

fn inset(rect: Rect, style: &StyleProps) -> Rect {
    let (px, py) = style.effective_padding();
    let width = rect.width.saturating_sub(px * 2);
    let height = rect.height.saturating_sub(py * 2);
    Rect::new(rect.x + px, rect.y + py, width, height)
}

/// Intrinsic size of `widget` given a width bound (`width_bound`).
fn measure(widget: &Widget, width_bound: u32, metrics: &dyn FontMetrics) -> Size {
    match widget {
        Widget::Text { text, style, .. } => {
            let pad = style.padding.unwrap_or(0);
            let w = metrics.text_width(text).saturating_add(pad * 2);
            let h = metrics.line_height().saturating_add(pad * 2);
            Size {
                width: style.width.unwrap_or(w).min(width_bound),
                height: style.height.unwrap_or(h),
            }
        }
        Widget::Button { label, style } => {
            let (px, py) = style.effective_padding();
            let w = metrics
                .text_width(label)
                .saturating_add(px * 2);
            let h = metrics.line_height().saturating_add(py * 2);
            Size {
                width: style.width.unwrap_or(w).min(width_bound),
                height: style.height.unwrap_or(h),
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
                .or(style.width)
                .unwrap_or_else(|| {
                    metrics
                        .text_width(display)
                        .saturating_add(px * 2)
                });
            let h = metrics.line_height().saturating_add(py * 2);
            Size {
                width: w.min(width_bound),
                height: style.height.unwrap_or(h),
            }
        }
        Widget::Unknown { kind, style } => {
            let (px, py) = style.effective_padding();
            let w = metrics
                .text_width(kind)
                .saturating_add(px * 2);
            let h = metrics.line_height().saturating_add(py * 2);
            Size {
                width: style.width.unwrap_or(w).min(width_bound),
                height: style.height.unwrap_or(h),
            }
        }
        Widget::VStack { spacing, children, style } => {
            let mut size = measure_stack(children, *spacing, width_bound, metrics);
            let pad = style.padding.unwrap_or(0);
            size.width = size.width.saturating_add(pad * 2);
            size.height = size.height.saturating_add(pad * 2);
            size.width = style.width.unwrap_or(size.width).min(width_bound);
            size.height = style.height.unwrap_or(size.height);
            size
        }
        Widget::Screen { children, style, .. } => {
            let mut size = measure_stack(children, 0, width_bound, metrics);
            let pad = style.padding.unwrap_or(0);
            size.width = size.width.saturating_add(pad * 2);
            size.height = size.height.saturating_add(pad * 2);
            size.width = style.width.unwrap_or(size.width).min(width_bound);
            size.height = style.height.unwrap_or(size.height);
            size
        }
        Widget::HStack { spacing, children, style } => {
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
                width: style.width.unwrap_or(width).min(width_bound),
                height: style.height.unwrap_or(height),
            }
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
fn place<'a>(
    widget: &'a Widget,
    origin: (u32, u32),
    width: u32,
    metrics: &dyn FontMetrics,
) -> Placed<'a> {
    let (x, y) = origin;
    match widget {
        Widget::Text { text, style, .. } => {
            let margin = style.margin.unwrap_or(0);
            let pad = style.padding.unwrap_or(0);
            let text_w = metrics.text_width(text).saturating_add(pad * 2);
            let text_h = metrics.line_height().saturating_add(pad * 2);
            let w = style.width.unwrap_or(text_w);
            let h = style.height.unwrap_or(text_h);
            Placed {
                widget,
                rect: Rect::new(x + margin, y + margin, w.min(width.saturating_sub(margin * 2)), h),
                children: Vec::new(),
            }
        }
        Widget::Button { label, style } => {
            let margin = style.margin.unwrap_or(0);
            let (px, py) = style.effective_padding();
            let w = metrics
                .text_width(label)
                .saturating_add(px * 2);
            let h = metrics.line_height().saturating_add(py * 2);
            let w = style.width.unwrap_or(w);
            let h = style.height.unwrap_or(h);
            Placed {
                widget,
                rect: Rect::new(x + margin, y + margin, w.min(width.saturating_sub(margin * 2)), h),
                children: Vec::new(),
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
                .or(style.width)
                .unwrap_or_else(|| {
                    metrics
                        .text_width(display)
                        .saturating_add(px * 2)
                });
            let h = metrics.line_height().saturating_add(py * 2);
            Placed {
                widget,
                rect: Rect::new(x + margin, y + margin, w.min(width.saturating_sub(margin * 2)), style.height.unwrap_or(h)),
                children: Vec::new(),
            }
        }
        Widget::Unknown { kind, style } => {
            let margin = style.margin.unwrap_or(0);
            let (px, py) = style.effective_padding();
            let w = metrics
                .text_width(kind)
                .saturating_add(px * 2);
            let h = metrics.line_height().saturating_add(py * 2);
            let w = style.width.unwrap_or(w);
            let h = style.height.unwrap_or(h);
            Placed {
                widget,
                rect: Rect::new(x + margin, y + margin, w.min(width.saturating_sub(margin * 2)), h),
                children: Vec::new(),
            }
        }
        Widget::VStack { spacing, children, style } => {
            let margin = style.margin.unwrap_or(0);
            let pad = style.padding.unwrap_or(0);
            let inner_x = x + margin;
            let inner_y = y + margin;
            let inner_width = width.saturating_sub(margin * 2).saturating_sub(pad * 2);
            let mut cursor_y = inner_y + pad;
            let mut max_bottom = inner_y + pad;
            let mut placed_children = Vec::with_capacity(children.len());
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    cursor_y = cursor_y.saturating_add(*spacing);
                }
                let child_placed = place(child, (inner_x + pad, cursor_y), inner_width, metrics);
                cursor_y = cursor_y.saturating_add(child_placed.rect.height);
                max_bottom = max_bottom.max(cursor_y);
                placed_children.push(child_placed);
            }
            let total_h = max_bottom + pad - y - margin;
            let total_w = style.width.unwrap_or(width);
            Placed {
                widget,
                rect: Rect::new(x + margin, y + margin, total_w.saturating_sub(margin * 2), total_h),
                children: placed_children,
            }
        }
        Widget::Screen { children, style, .. } => {
            let margin = style.margin.unwrap_or(0);
            let pad = style.padding.unwrap_or(0);
            let inner_x = x + margin + pad;
            let inner_y = y + margin + pad;
            let inner_width = width.saturating_sub(margin * 2).saturating_sub(pad * 2);
            let mut cursor_y = inner_y;
            let mut max_bottom = inner_y;
            let mut placed_children = Vec::with_capacity(children.len());
            for child in children.iter() {
                let child_placed = place(child, (inner_x, cursor_y), inner_width, metrics);
                cursor_y = cursor_y.saturating_add(child_placed.rect.height);
                max_bottom = max_bottom.max(cursor_y);
                placed_children.push(child_placed);
            }
            let total_h = max_bottom + pad - y - margin;
            let total_w = style.width.unwrap_or(width);
            Placed {
                widget,
                rect: Rect::new(x + margin, y + margin, total_w.saturating_sub(margin * 2), total_h),
                children: placed_children,
            }
        }
        Widget::HStack { spacing, children, style } => {
            let margin = style.margin.unwrap_or(0);
            let pad = style.padding.unwrap_or(0);
            let inner_x = x + margin + pad;
            let inner_y = y + margin + pad;
            let inner_width = width.saturating_sub(margin * 2).saturating_sub(pad * 2);
            let mut cursor_x = inner_x;
            let mut max_right = inner_x;
            let mut max_height = 0u32;
            let mut placed_children = Vec::with_capacity(children.len());
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    cursor_x = cursor_x.saturating_add(*spacing);
                }
                let remaining = inner_width.saturating_sub(cursor_x - inner_x);
                let child_width = match child {
                    Widget::VStack { .. } | Widget::Screen { .. } => {
                        measure(child, remaining, metrics).width
                    }
                    _ => remaining,
                };
                let (child_placed, child_size) =
                    place_sized(child, (cursor_x, inner_y), child_width, metrics);
                cursor_x = cursor_x.saturating_add(child_size.width);
                max_right = max_right.max(cursor_x);
                max_height = max_height.max(child_size.height);
                placed_children.push(child_placed);
            }
            let total_w = style.width.unwrap_or(max_right - x + pad + margin);
            let total_h = style.height.unwrap_or(max_height.saturating_add(pad * 2));
            Placed {
                widget,
                rect: Rect::new(x + margin, y + margin, total_w.saturating_sub(margin * 2), total_h),
                children: placed_children,
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
) -> (Placed<'a>, Size) {
    let placed = place(widget, origin, width, metrics);
    let size = Size {
        width: placed.rect.width,
        height: placed.rect.height,
    };
    (placed, size)
}
