//! Taffy-backed layout engine.
//!
//! Converts the [`Widget`] tree into a [`taffy::TaffyTree`], runs Taffy's
//! Flexbox algorithm, and maps the results back to [`Placed`] rectangles.

use crate::layout::{FontMetrics, LayoutEngine, Placed};
use crate::widgets::{Alignment, Dimension, Justify, Rect, Size, StyleProps, Widget};
use std::sync::Arc;
use taffy::TaffyTree;
use taffy::prelude::TaffyAuto;

/// Metadata stored alongside each Taffy node so the measure function can
/// compute intrinsic sizes for leaf widgets.
#[derive(Debug, Clone)]
enum NodeContext {
    Leaf { widget: Arc<Widget> },
}

/// Convert an `Option<u32>` to a taffy `LengthPercentageAuto`.
fn opt_to_lpa(d: Option<u32>) -> taffy::LengthPercentageAuto {
    match d {
        Some(n) => taffy::LengthPercentageAuto::length(n as f32),
        None => taffy::LengthPercentageAuto::auto(),
    }
}

/// Convert xulo `StyleProps` + container spacing to taffy `Style`.
fn style_to_taffy(
    style: &StyleProps,
    flex_direction: taffy::FlexDirection,
    spacing: u32,
    is_container: bool,
) -> taffy::Style {
    let gap = spacing as f32;
    let padding = style.padding.unwrap_or(0) as f32;
    let margin = style.margin.unwrap_or(0) as f32;

    let justify_content = style
        .justify
        .map(|j| match j {
            Justify::Start => taffy::JustifyContent::FLEX_START,
            Justify::Center => taffy::JustifyContent::CENTER,
            Justify::End => taffy::JustifyContent::FLEX_END,
            Justify::SpaceBetween => taffy::JustifyContent::SPACE_BETWEEN,
            Justify::SpaceAround => taffy::JustifyContent::SPACE_AROUND,
            Justify::SpaceEvenly => taffy::JustifyContent::SPACE_EVENLY,
        })
        .unwrap_or(taffy::JustifyContent::FLEX_START);

    let align_items = style
        .alignment
        .map(|a| match a {
            Alignment::Start => taffy::AlignItems::FLEX_START,
            Alignment::Center => taffy::AlignItems::CENTER,
            Alignment::End => taffy::AlignItems::FLEX_END,
            Alignment::Stretch => taffy::AlignItems::STRETCH,
        })
        .unwrap_or(taffy::AlignItems::FLEX_START);

    let flex_grow = style.flex_grow.unwrap_or(0.0);
    let flex_shrink = style.flex_shrink.unwrap_or(1.0);

    // Containers (VStack/HStack/Screen) default to 100% width when unset
    let size_width = style
        .width
        .map(|d| match d {
            Dimension::Pixels(n) => taffy::Dimension::length(n as f32),
            Dimension::Full => taffy::Dimension::percent(1.0),
        })
        .unwrap_or(if is_container {
            taffy::Dimension::percent(1.0)
        } else {
            taffy::Dimension::AUTO
        });

    let size_height = style
        .height
        .map(|d| match d {
            Dimension::Pixels(n) => taffy::Dimension::length(n as f32),
            Dimension::Full => taffy::Dimension::percent(1.0),
        })
        .unwrap_or(taffy::Dimension::AUTO);

    // flex_basis: explicit basis wins, otherwise fall back to size
    let flex_basis = style
        .flex_basis
        .map(|b| taffy::Dimension::length(b as f32))
        .unwrap_or(taffy::Dimension::AUTO);

    taffy::Style {
        display: taffy::Display::Flex,
        flex_direction,
        justify_content: Some(justify_content),
        align_items: Some(align_items),
        flex_grow,
        flex_shrink,
        flex_basis,
        size: taffy::Size {
            width: size_width,
            height: size_height,
        },
        gap: taffy::Size {
            width: taffy::LengthPercentage::length(gap),
            height: taffy::LengthPercentage::length(gap),
        },
        padding: taffy::Rect {
            left: taffy::LengthPercentage::length(padding),
            right: taffy::LengthPercentage::length(padding),
            top: taffy::LengthPercentage::length(padding),
            bottom: taffy::LengthPercentage::length(padding),
        },
        margin: taffy::Rect {
            left: taffy::LengthPercentageAuto::length(margin),
            right: taffy::LengthPercentageAuto::length(margin),
            top: taffy::LengthPercentageAuto::length(margin),
            bottom: taffy::LengthPercentageAuto::length(margin),
        },
        min_size: taffy::Size {
            width: opt_to_lpa(style.min_width),
            height: opt_to_lpa(style.min_height),
        },
        max_size: taffy::Size {
            width: opt_to_lpa(style.max_width),
            height: opt_to_lpa(style.max_height),
        },
        ..Default::default()
    }
}

/// Build a `TaffyTree` from the `Widget` tree, returning the root `NodeId`.
fn build_taffy_tree(taffy: &mut TaffyTree<NodeContext>, widget: &Widget) -> taffy::NodeId {
    match widget {
        Widget::Screen {
            children, style, ..
        } => {
            let s = style_to_taffy(style, taffy::FlexDirection::Column, 0, true);
            // Screen children fill the available height via flex_grow,
            // unless they have an explicit height set.
            let child_ids: Vec<taffy::NodeId> = children
                .iter()
                .map(|c| {
                    let child_id = build_taffy_tree(taffy, c);
                    // Only set flex_grow if child doesn't have explicit height
                    if c.style().height.is_none()
                        && let Ok(child_style) = taffy.style(child_id)
                    {
                        let mut child_style = child_style.clone();
                        child_style.flex_grow = 1.0;
                        let _ = taffy.set_style(child_id, child_style);
                    }
                    child_id
                })
                .collect();
            taffy.new_with_children(s, &child_ids).unwrap()
        }
        Widget::VStack {
            children,
            style,
            spacing,
        } => {
            let s = style_to_taffy(style, taffy::FlexDirection::Column, *spacing, true);
            let child_ids: Vec<taffy::NodeId> = children
                .iter()
                .map(|c| build_taffy_tree(taffy, c))
                .collect();
            taffy.new_with_children(s, &child_ids).unwrap()
        }
        Widget::HStack {
            children,
            style,
            spacing,
        } => {
            let s = style_to_taffy(style, taffy::FlexDirection::Row, *spacing, true);
            let child_ids: Vec<taffy::NodeId> = children
                .iter()
                .map(|c| build_taffy_tree(taffy, c))
                .collect();
            taffy.new_with_children(s, &child_ids).unwrap()
        }
        Widget::Fill { children, style } => {
            let mut s = style_to_taffy(style, taffy::FlexDirection::Column, 0, true);
            s.flex_grow = style.flex_grow.unwrap_or(1.0);
            let child_ids: Vec<taffy::NodeId> = children
                .iter()
                .map(|c| build_taffy_tree(taffy, c))
                .collect();
            taffy.new_with_children(s, &child_ids).unwrap()
        }
        Widget::ScrollView {
            children, style, ..
        } => {
            let mut s = style_to_taffy(style, taffy::FlexDirection::Column, 0, true);
            s.overflow = taffy::Point {
                x: taffy::Overflow::Hidden,
                y: taffy::Overflow::Hidden,
            };
            let child_ids: Vec<taffy::NodeId> = children
                .iter()
                .map(|c| build_taffy_tree(taffy, c))
                .collect();
            taffy.new_with_children(s, &child_ids).unwrap()
        }
        // Leaf widgets — compute size from both style and measure function.
        // Explicit width/height from the widget style are set in the Taffy style;
        // only unset (AUTO) dimensions are measured by the measure callback.
        _ => {
            let mut s = style_to_taffy(widget.style(), taffy::FlexDirection::Column, 0, false);
            // For dimensions WITHOUT an explicit style value, set AUTO so the measure function determines them.
            // For dimensions WITH an explicit value, keep the value from style_to_taffy.
            if widget.style().width.is_none() {
                s.size.width = taffy::Dimension::AUTO;
            }
            if widget.style().height.is_none() {
                s.size.height = taffy::Dimension::AUTO;
            }
            // Pin explicit sizes via min/max so Taffy's compute_leaf_layout
            // cannot override them with known_dimensions from the parent.
            if let Some(crate::widgets::Dimension::Pixels(n)) = widget.style().width {
                let v = taffy::LengthPercentageAuto::length(n as f32);
                s.min_size.width = v;
                s.max_size.width = v;
            }
            if let Some(crate::widgets::Dimension::Pixels(n)) = widget.style().height {
                let v = taffy::LengthPercentageAuto::length(n as f32);
                s.min_size.height = v;
                s.max_size.height = v;
            }
            // Decorated text should expand in flex containers
            if matches!(widget, Widget::Text { style, .. } if style.is_text_decorated())
                && s.flex_grow == 0.0
            {
                s.flex_grow = 1.0;
            }
            taffy
                .new_leaf_with_context(
                    s,
                    NodeContext::Leaf {
                        widget: Arc::new(widget.clone()),
                    },
                )
                .unwrap()
        }
    }
}

/// Measure function for leaf nodes. Computes intrinsic size from text content.
fn measure_leaf(
    widget: &Widget,
    known_width: Option<f32>,
    metrics: &dyn FontMetrics,
) -> taffy::Size<f32> {
    match widget {
        Widget::Text { text, style, .. } => {
            if style.is_text_decorated() {
                let pad = style.padding.unwrap_or(0) as f32;
                let text_w = metrics.text_width(text) as f32 + pad * 2.0;
                let text_h = metrics.line_height() as f32 + pad * 2.0;
                let w = style
                    .width
                    .map(|d| match d {
                        Dimension::Pixels(n) => n as f32,
                        Dimension::Full => known_width.unwrap_or(text_w),
                    })
                    .unwrap_or(text_w);
                let h = style
                    .height
                    .map(|d| match d {
                        Dimension::Pixels(n) => n as f32,
                        Dimension::Full => text_h,
                    })
                    .unwrap_or(text_h);
                taffy::Size {
                    width: w,
                    height: h,
                }
            } else {
                let text_w = metrics.text_width(text) as f32;
                let text_h = metrics.line_height() as f32;
                let w = style
                    .width
                    .map(|d| match d {
                        Dimension::Pixels(n) => n as f32,
                        Dimension::Full => known_width.unwrap_or(text_w),
                    })
                    .unwrap_or(text_w);
                let h = style
                    .height
                    .map(|d| match d {
                        Dimension::Pixels(n) => n as f32,
                        Dimension::Full => text_h,
                    })
                    .unwrap_or(text_h);
                taffy::Size {
                    width: w,
                    height: h,
                }
            }
        }
        Widget::Button { label, style } => {
            let (px, py) = style.effective_padding();
            let w = metrics.text_width(label) as f32 + px as f32 * 2.0;
            let h = metrics.line_height() as f32 + py as f32 * 2.0;
            let w = style
                .width
                .map(|d| match d {
                    Dimension::Pixels(n) => n as f32,
                    Dimension::Full => known_width.unwrap_or(w),
                })
                .unwrap_or(w);
            let h = style
                .height
                .map(|d| match d {
                    Dimension::Pixels(n) => n as f32,
                    Dimension::Full => h,
                })
                .unwrap_or(h);
            taffy::Size {
                width: w,
                height: h,
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
                .or(style.width.and_then(|d| match d {
                    Dimension::Pixels(n) => Some(n),
                    _ => None,
                }))
                .map(|n| n as f32)
                .unwrap_or_else(|| metrics.text_width(display) as f32 + px as f32 * 2.0);
            let h = metrics.line_height() as f32 + py as f32 * 2.0;
            let h = style
                .height
                .map(|d| match d {
                    Dimension::Pixels(n) => n as f32,
                    Dimension::Full => h,
                })
                .unwrap_or(h);
            taffy::Size {
                width: w.min(known_width.unwrap_or(f32::MAX)),
                height: h,
            }
        }
        Widget::Unknown { kind, style } => {
            let (px, py) = style.effective_padding();
            let w = metrics.text_width(kind) as f32 + px as f32 * 2.0;
            let h = metrics.line_height() as f32 + py as f32 * 2.0;
            let w = style
                .width
                .map(|d| match d {
                    Dimension::Pixels(n) => n as f32,
                    Dimension::Full => known_width.unwrap_or(w),
                })
                .unwrap_or(w);
            let h = style
                .height
                .map(|d| match d {
                    Dimension::Pixels(n) => n as f32,
                    Dimension::Full => h,
                })
                .unwrap_or(h);
            taffy::Size {
                width: w,
                height: h,
            }
        }
        _ => taffy::Size {
            width: 0.0,
            height: 0.0,
        },
    }
}

/// Read the taffy layout results and build a `Placed` tree.
fn read_layout<'a>(
    taffy: &TaffyTree<NodeContext>,
    node_id: taffy::NodeId,
    widget: &'a Widget,
    parent_offset: (u32, u32),
    _metrics: &dyn FontMetrics,
) -> crate::layout::Placed<'a> {
    let layout = taffy.layout(node_id).unwrap();
    let x = parent_offset.0 + (layout.location.x.round() as u32);
    let y = parent_offset.1 + (layout.location.y.round() as u32);
    let w = layout.size.width.round() as u32;
    let h = layout.size.height.round() as u32;

    let children_widgets = match widget {
        Widget::Screen { children, .. }
        | Widget::VStack { children, .. }
        | Widget::HStack { children, .. }
        | Widget::Fill { children, .. }
        | Widget::ScrollView { children, .. } => Some(children.as_slice()),
        _ => None,
    };

    let placed_children = if let Some(child_widgets) = children_widgets {
        let taffy_children = taffy.children(node_id).unwrap();
        child_widgets
            .iter()
            .zip(taffy_children.iter())
            .map(|(cw, &cn)| read_layout(taffy, cn, cw, (x, y), _metrics))
            .collect()
    } else {
        Vec::new()
    };

    let scroll_index = if matches!(widget, Widget::ScrollView { .. }) {
        Some(crate::layout::next_scroll_index())
    } else {
        None
    };

    crate::layout::Placed {
        widget,
        rect: Rect::new(x, y, w, h),
        children: placed_children,
        scroll_index,
    }
}

/// Layout `root` against `surface` with `metrics` using Taffy's Flexbox engine.
pub fn layout_taffy<'a>(
    root: &'a Widget,
    surface: Size,
    metrics: &dyn FontMetrics,
) -> crate::layout::Placed<'a> {
    crate::layout::reset_scroll_state();
    let mut taffy: TaffyTree<NodeContext> = TaffyTree::new();
    let root_id = build_taffy_tree(&mut taffy, root);

    // Constrain root to surface size
    let mut root_style = taffy.style(root_id).cloned().unwrap_or_default();
    root_style.size = taffy::Size {
        width: taffy::Dimension::length(surface.width as f32),
        height: taffy::Dimension::length(surface.height as f32),
    };
    root_style.min_size = taffy::Size {
        width: taffy::LengthPercentageAuto::length(surface.width as f32),
        height: taffy::LengthPercentageAuto::length(surface.height as f32),
    };
    root_style.max_size = taffy::Size {
        width: taffy::LengthPercentageAuto::length(surface.width as f32),
        height: taffy::LengthPercentageAuto::length(surface.height as f32),
    };
    let _ = taffy.set_style(root_id, root_style);

    let available = taffy::Size {
        width: taffy::AvailableSpace::Definite(surface.width as f32),
        height: taffy::AvailableSpace::Definite(surface.height as f32),
    };

    taffy
        .compute_layout_with_measure(
            root_id,
            available,
            |inputs, _node_id, node_context, _style| {
                taffy::compute_leaf_layout(
                    inputs,
                    _style,
                    |_, _| 0.0,
                    |known_dimensions, _available| {
                        if let Some(NodeContext::Leaf { widget }) = node_context {
                            let known_w = known_dimensions.width;
                            measure_leaf(widget, known_w, metrics)
                        } else {
                            taffy::Size {
                                width: 0.0,
                                height: 0.0,
                            }
                        }
                    },
                )
            },
        )
        .unwrap();

    let mut placed = read_layout(&taffy, root_id, root, (0, 0), metrics);
    // Force root to surface size (Screen always fills)
    placed.rect.width = surface.width;
    placed.rect.height = surface.height;
    placed
}

/// Taffy-backed layout engine implementing [`LayoutEngine`].
pub struct TaffyLayout;

impl LayoutEngine for TaffyLayout {
    fn layout<'a>(&self, root: &'a Widget, surface: Size, metrics: &dyn FontMetrics) -> Placed<'a> {
        layout_taffy(root, surface, metrics)
    }
}
