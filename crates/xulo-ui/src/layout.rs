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
use crate::widgets::{Color, Rect, Size, Widget};

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
pub fn paint(placed: &Placed<'_>, theme: &Theme, ops: &mut Vec<PaintOp>) {
    match placed.widget {
        Widget::Screen { background, .. } => {
            let color = background.unwrap_or(theme.background);
            ops.push(PaintOp::Clear { color });
            for child in &placed.children {
                paint(child, theme, ops);
            }
        }
        Widget::VStack { .. } | Widget::HStack { .. } => {
            for child in &placed.children {
                paint(child, theme, ops);
            }
        }
        Widget::Text { text, color } => {
            ops.push(PaintOp::DrawText {
                rect: placed.rect,
                text: text.clone(),
                color: color.unwrap_or(theme.text),
            });
        }
        Widget::Button { label } => {
            ops.push(PaintOp::DrawBorder {
                rect: placed.rect,
                color: theme.border,
            });
            ops.push(PaintOp::DrawText {
                rect: inset(placed.rect),
                text: label.clone(),
                color: theme.accent,
            });
        }
        Widget::Input { value } => {
            ops.push(PaintOp::DrawBorder {
                rect: placed.rect,
                color: theme.border,
            });
            ops.push(PaintOp::DrawText {
                rect: inset(placed.rect),
                text: value.clone(),
                color: theme.text,
            });
        }
        Widget::Unknown { kind } => {
            ops.push(PaintOp::DrawBorder {
                rect: placed.rect,
                color: theme.border,
            });
            ops.push(PaintOp::DrawText {
                rect: placed.rect,
                text: kind.clone(),
                color: theme.text,
            });
        }
    }
}

fn inset(rect: Rect) -> Rect {
    let width = rect.width.saturating_sub(PAD_X * 2);
    let height = rect.height.saturating_sub(PAD_Y * 2);
    Rect::new(rect.x + PAD_X, rect.y + PAD_Y, width, height)
}

/// Intrinsic size of `widget` given a width bound (`width_bound`).
fn measure(widget: &Widget, width_bound: u32, metrics: &dyn FontMetrics) -> Size {
    match widget {
        Widget::Text { text, .. } => Size {
            width: metrics.text_width(text).min(width_bound),
            height: metrics.line_height(),
        },
        Widget::Button { label } => Size {
            width: metrics
                .text_width(label)
                .saturating_add(PAD_X * 2)
                .min(width_bound),
            height: metrics.line_height().saturating_add(PAD_Y * 2),
        },
        Widget::Input { value } => Size {
            width: metrics
                .text_width(value)
                .saturating_add(PAD_X * 2)
                .min(width_bound),
            height: metrics.line_height().saturating_add(PAD_Y * 2),
        },
        Widget::Unknown { kind } => Size {
            width: metrics
                .text_width(kind)
                .saturating_add(PAD_X * 2)
                .min(width_bound),
            height: metrics.line_height().saturating_add(PAD_Y * 2),
        },
        Widget::VStack { spacing, children } => {
            measure_stack(children, *spacing, width_bound, metrics)
        }
        Widget::Screen { children, .. } => measure_stack(children, 0, width_bound, metrics),
        Widget::HStack { spacing, children } => {
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
            Size {
                width: width.min(width_bound),
                height,
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
        Widget::Text { text, .. } => Placed {
            widget,
            rect: Rect::new(
                x,
                y,
                metrics.text_width(text).min(width),
                metrics.line_height(),
            ),
            children: Vec::new(),
        },
        Widget::Button { label } => Placed {
            widget,
            rect: Rect::new(
                x,
                y,
                metrics
                    .text_width(label)
                    .saturating_add(PAD_X * 2)
                    .min(width),
                metrics.line_height().saturating_add(PAD_Y * 2),
            ),
            children: Vec::new(),
        },
        Widget::Input { value } => Placed {
            widget,
            rect: Rect::new(
                x,
                y,
                metrics
                    .text_width(value)
                    .saturating_add(PAD_X * 2)
                    .min(width),
                metrics.line_height().saturating_add(PAD_Y * 2),
            ),
            children: Vec::new(),
        },
        Widget::Unknown { kind } => Placed {
            widget,
            rect: Rect::new(
                x,
                y,
                metrics
                    .text_width(kind)
                    .saturating_add(PAD_X * 2)
                    .min(width),
                metrics.line_height().saturating_add(PAD_Y * 2),
            ),
            children: Vec::new(),
        },
        Widget::VStack { spacing, children } => {
            place_stack(widget, children, *spacing, origin, width, metrics)
        }
        Widget::Screen { children, .. } => place_stack(widget, children, 0, origin, width, metrics),
        Widget::HStack { spacing, children } => {
            let mut cursor_x = x;
            let mut max_right = x;
            let mut max_height = 0u32;
            let mut placed_children = Vec::with_capacity(children.len());
            for (i, child) in children.iter().enumerate() {
                if i > 0 {
                    cursor_x = cursor_x.saturating_add(*spacing);
                }
                let remaining = width.saturating_sub(cursor_x - x);
                let child_width = measure(child, remaining, metrics).width;
                let child_placed = place(child, (cursor_x, y), child_width, metrics);
                cursor_x = cursor_x.saturating_add(child_placed.rect.width);
                max_right = max_right.max(cursor_x);
                max_height = max_height.max(child_placed.rect.height);
                placed_children.push(child_placed);
            }
            Placed {
                widget,
                rect: Rect::new(x, y, max_right - x, max_height),
                children: placed_children,
            }
        }
    }
}

fn place_stack<'a>(
    widget: &'a Widget,
    children: &'a [Widget],
    spacing: u32,
    origin: (u32, u32),
    width: u32,
    metrics: &dyn FontMetrics,
) -> Placed<'a> {
    let (x, y) = origin;
    let mut cursor_y = y;
    let mut max_bottom = y;
    let mut placed_children = Vec::with_capacity(children.len());
    for (i, child) in children.iter().enumerate() {
        if i > 0 {
            cursor_y = cursor_y.saturating_add(spacing);
        }
        let child_placed = place(child, (x, cursor_y), width, metrics);
        cursor_y = cursor_y.saturating_add(child_placed.rect.height);
        max_bottom = max_bottom.max(cursor_y);
        placed_children.push(child_placed);
    }
    Placed {
        widget,
        rect: Rect::new(x, y, width, max_bottom - y),
        children: placed_children,
    }
}
