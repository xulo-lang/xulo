//! Layout integration tests using a character-cell font metric.

use xulo_ui::layout::layout;
use xulo_ui::{StyleProps, Color, Dimension, FontMetrics, Justify, Rect, Size, Widget};

/// One layout unit per character: the terminal backend's metric.
struct CharMetrics;

impl FontMetrics for CharMetrics {
    fn text_width(&self, text: &str) -> u32 {
        text.chars().count() as u32
    }
    fn line_height(&self) -> u32 {
        1
    }
}

const SURFACE: Size = Size {
    width: 80,
    height: 24,
};

fn metrics() -> CharMetrics {
    CharMetrics
}

#[test]
fn vstack_stacks_children_top_to_bottom_with_spacing() {
    let root = Widget::VStack {
        spacing: 2,
        style: StyleProps::default(),
        children: vec![
            Widget::Text {
                text: "aaa".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "bb".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // VStack fills the surface width (80); height = 1 + 2 + 1 = 4.
    assert_eq!(placed.rect, Rect::new(0, 0, 80, 4));
    assert_eq!(placed.children.len(), 2);
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 3, 1));
    assert_eq!(placed.children[1].rect, Rect::new(0, 3, 2, 1));
}

#[test]
fn vstack_children_fill_container_width() {
    // A text shorter than the container still occupies the full row width so
    // later siblings align on the left edge of the same column.
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![Widget::Text {
            text: "x".into(),
            color: None,
            style: StyleProps::default(),
        }],
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 1, 1));
    // The text is clipped to its own width even though it fills the row.
    assert_eq!(placed.rect.width, 80);
}

#[test]
fn hstack_places_children_side_by_side() {
    let root = Widget::HStack {
        spacing: 1,
        style: StyleProps::default(),
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.rect, Rect::new(0, 0, 3, 1));
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 1, 1));
    assert_eq!(placed.children[1].rect, Rect::new(2, 0, 1, 1));
}

#[test]
fn button_gets_horizontal_padding() {
    let root = Widget::HStack {
        spacing: 2,
        style: StyleProps::default(),
        children: vec![
            Widget::Button { label: "+".into(), style: StyleProps::default() },
            Widget::Button { label: "-".into(), style: StyleProps::default() },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Each button is label(1) + 2*padding(1) = 3 wide and 3 tall.
    assert_eq!(placed.rect, Rect::new(0, 0, 8, 3));
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 3, 3));
    assert_eq!(placed.children[1].rect, Rect::new(5, 0, 3, 3));
}

#[test]
fn nested_vstack_in_hstack_shrinks_to_widest_child() {
    // The inner VStack must not swallow the remaining row width: it sizes to
    // its widest child ("hello" = 5), letting the trailing Text sit beside it.
    let root = Widget::HStack {
        spacing: 1,
        style: StyleProps::default(),
        children: vec![
            Widget::VStack {
                spacing: 0,
                style: StyleProps::default(),
                children: vec![
                    Widget::Text {
                        text: "hello".into(),
                        color: None,
                        style: StyleProps::default(),
                    },
                    Widget::Text {
                        text: "x".into(),
                        color: None,
                        style: StyleProps::default(),
                    },
                ],
            },
            Widget::Text {
                text: "!".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.rect, Rect::new(0, 0, 7, 2));
    let inner = &placed.children[0];
    assert_eq!(inner.rect, Rect::new(0, 0, 5, 2));
    assert_eq!(inner.children[0].rect, Rect::new(0, 0, 5, 1));
    assert_eq!(inner.children[1].rect, Rect::new(0, 1, 1, 1));
    assert_eq!(placed.children[1].rect, Rect::new(6, 0, 1, 1));
}

#[test]
fn text_truncates_to_container_width() {
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![Widget::Text {
            text: "a_very_long_line".into(),
            color: None,
            style: StyleProps::default(),
        }],
    };
    // Give the layout a narrow bound via a Screen, whose width is the surface.
    let screen = Widget::Screen {
        background: None,
        style: StyleProps::default(),
        children: vec![root],
    };
    let narrow = Size {
        width: 4,
        height: 1,
    };
    let placed = layout(&screen, narrow, &metrics());
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 4, 1));
    assert_eq!(placed.children[0].children[0].rect, Rect::new(0, 0, 4, 1));
}

#[test]
fn screen_fills_surface() {
    let root = Widget::Screen {
        background: Some(Color::new(1, 2, 3)),
        style: StyleProps::default(),
        children: vec![Widget::Text {
            text: "hi".into(),
            color: None,
            style: StyleProps::default(),
        }],
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.rect, SURFACE.into_dimensions());
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 2, 1));
}

#[test]
fn unknown_renders_as_box() {
    let root = Widget::Unknown {
        kind: "SomeWidget".into(),
        style: StyleProps::default(),
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.rect, Rect::new(0, 0, 12, 3));
}

trait IntoRect {
    fn into_dimensions(self) -> Rect;
}

impl IntoRect for Size {
    fn into_dimensions(self) -> Rect {
        Rect::new(0, 0, self.width, self.height)
    }
}

// ── Alignment tests ─────────────────────────────────────────────────────────

#[test]
fn vstack_center_alignment() {
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            alignment: Some(xulo_ui::Alignment::Center),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "bbb".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // "a" (1) centered in 80-width: x = (80-1)/2 = 39
    assert_eq!(placed.children[0].rect.x, 39);
    // "bbb" (3) centered in 80-width: x = (80-3)/2 = 38
    assert_eq!(placed.children[1].rect.x, 38);
}

#[test]
fn vstack_end_alignment() {
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            alignment: Some(xulo_ui::Alignment::End),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "short".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "longer".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // "short" (5) right-aligned: x = 80-5 = 75
    assert_eq!(placed.children[0].rect.x, 75);
    // "longer" (6) right-aligned: x = 80-6 = 74
    assert_eq!(placed.children[1].rect.x, 74);
}

#[test]
fn vstack_start_alignment_is_default() {
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![Widget::Text {
            text: "hi".into(),
            color: None,
            style: StyleProps::default(),
        }],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Default alignment = Start, x = 0
    assert_eq!(placed.children[0].rect.x, 0);
}

#[test]
fn hstack_center_alignment() {
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps {
            alignment: Some(xulo_ui::Alignment::Center),
            height: Some(Dimension::Pixels(5)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "hi".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "x".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // HStack height=5, children centered vertically
    // "hi" (height 1): y = (5-1)/2 = 2
    assert_eq!(placed.children[0].rect.y, 2);
    // "x" (height 1): y = (5-1)/2 = 2
    assert_eq!(placed.children[1].rect.y, 2);
}

#[test]
fn hstack_end_alignment() {
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps {
            alignment: Some(xulo_ui::Alignment::End),
            height: Some(Dimension::Pixels(5)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // HStack height=5, children bottom-aligned
    // "a" (height 1): y = 5-1 = 4
    assert_eq!(placed.children[0].rect.y, 4);
    // "b" (height 1): y = 5-1 = 4
    assert_eq!(placed.children[1].rect.y, 4);
}

#[test]
fn hstack_start_alignment_is_default() {
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(5)),
            ..StyleProps::default()
        },
        children: vec![Widget::Text {
            text: "hi".into(),
            color: None,
            style: StyleProps::default(),
        }],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Default alignment = Start, y = 0
    assert_eq!(placed.children[0].rect.y, 0);
}

// ── effective_padding tests ─────────────────────────────────────────────────

#[test]
fn effective_padding_returns_padding_when_set() {
    let style = StyleProps {
        padding: Some(3),
        ..StyleProps::default()
    };
    let (px, py) = style.effective_padding();
    assert_eq!(px, 3);
    assert_eq!(py, 3);
}

#[test]
fn effective_padding_returns_default_when_unset() {
    let style = StyleProps::default();
    let (px, py) = style.effective_padding();
    assert_eq!(px, 1); // PAD_X
    assert_eq!(py, 1); // PAD_Y
}

// ── collect_interactive_rects tests ─────────────────────────────────────────

#[test]
fn collect_interactive_rects_finds_buttons_and_inputs() {
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![
            Widget::Button {
                label: "+".into(),
                style: StyleProps::default(),
            },
            Widget::Input {
                value: "".into(),
                width: None,
                placeholder: "name".into(),
                style: StyleProps::default(),
            },
            Widget::Button {
                label: "-".into(),
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    let (buttons, inputs) = xulo_ui::layout::collect_interactive_rects(&placed);
    assert_eq!(buttons.len(), 2);
    assert_eq!(inputs.len(), 1);
}

#[test]
fn collect_interactive_rects_nested_structure() {
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![
            Widget::HStack {
                spacing: 0,
                style: StyleProps::default(),
                children: vec![
                    Widget::Button {
                        label: "A".into(),
                        style: StyleProps::default(),
                    },
                    Widget::Button {
                        label: "B".into(),
                        style: StyleProps::default(),
                    },
                ],
            },
            Widget::Input {
                value: "".into(),
                width: None,
                placeholder: "".into(),
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    let (buttons, inputs) = xulo_ui::layout::collect_interactive_rects(&placed);
    assert_eq!(buttons.len(), 2);
    assert_eq!(inputs.len(), 1);
}

#[test]
fn collect_interactive_rects_no_interactive() {
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![
            Widget::Text {
                text: "hello".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "world".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    let (buttons, inputs) = xulo_ui::layout::collect_interactive_rects(&placed);
    assert_eq!(buttons.len(), 0);
    assert_eq!(inputs.len(), 0);
}

// ── VStack justify-content tests ────────────────────────────────────────────

#[test]
fn vstack_justify_start_is_default() {
    // Two 1-height children in a 10-height VStack: both should start at top
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // No justify = Start: children at y=0, y=1
    assert_eq!(placed.children[0].rect.y, 0);
    assert_eq!(placed.children[1].rect.y, 1);
}

#[test]
fn vstack_justify_center() {
    // Two 1-height children in a 10-height VStack: centered vertically
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            justify: Some(Justify::Center),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Free space = 10 - 2 = 8, offset = 8/2 = 4
    assert_eq!(placed.children[0].rect.y, 4);
    assert_eq!(placed.children[1].rect.y, 5);
}

#[test]
fn vstack_justify_end() {
    // Two 1-height children in a 10-height VStack: bottom-aligned
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            justify: Some(Justify::End),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Free space = 10 - 2 = 8, offset = 8
    assert_eq!(placed.children[0].rect.y, 8);
    assert_eq!(placed.children[1].rect.y, 9);
}

#[test]
fn vstack_justify_space_between() {
    // Three 1-height children in a 10-height VStack
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            justify: Some(Justify::SpaceBetween),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "c".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Free space = 10 - 3 = 7, gap = 7/(3-1) = 3 (integer division)
    // y: 0, 0+1+3=4, 4+1+3=8
    assert_eq!(placed.children[0].rect.y, 0);
    assert_eq!(placed.children[1].rect.y, 4);
    assert_eq!(placed.children[2].rect.y, 8);
}

#[test]
fn vstack_justify_space_around() {
    // Two 1-height children in a 10-height VStack
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            justify: Some(Justify::SpaceAround),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Free space = 10 - 2 = 8, gap = 8/2 = 4, offset = 4/2 = 2
    // y: 2, 2+1+4=7
    assert_eq!(placed.children[0].rect.y, 2);
    assert_eq!(placed.children[1].rect.y, 7);
}

#[test]
fn vstack_justify_space_evenly() {
    // Two 1-height children in a 10-height VStack
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            justify: Some(Justify::SpaceEvenly),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Free space = 10 - 2 = 8, gap = 8/(2+1) = 2 (integer division)
    // y: 2, 2+1+2=5
    assert_eq!(placed.children[0].rect.y, 2);
    assert_eq!(placed.children[1].rect.y, 5);
}

#[test]
fn vstack_justify_with_spacing() {
    // justify and spacing both apply
    let root = Widget::VStack {
        spacing: 1,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            justify: Some(Justify::SpaceBetween),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "c".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Total content = 1*3 + spacing*2 = 5, free = 10-5 = 5
    // gap = 5/2 = 2, actual spacing between = justify_gap + widget_spacing = 2+1=3
    // y: 0, 0+1+3=4, 4+1+3=8
    assert_eq!(placed.children[0].rect.y, 0);
    assert_eq!(placed.children[1].rect.y, 4);
    assert_eq!(placed.children[2].rect.y, 8);
}

// ── HStack justify-content tests ────────────────────────────────────────────

#[test]
fn hstack_justify_center() {
    // Two 1-width children in a 20-width HStack: centered horizontally
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps {
            width: Some(Dimension::Pixels(20)),
            justify: Some(Justify::Center),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, Size { width: 80, height: 24 }, &metrics());
    // Free space = 20 - 2 = 18, offset = 18/2 = 9
    assert_eq!(placed.children[0].rect.x, 9);
    assert_eq!(placed.children[1].rect.x, 10);
}

#[test]
fn hstack_justify_end() {
    // Two 1-width children in a 20-width HStack: right-aligned
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps {
            width: Some(Dimension::Pixels(20)),
            justify: Some(Justify::End),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, Size { width: 80, height: 24 }, &metrics());
    // Free space = 20 - 2 = 18, offset = 18
    assert_eq!(placed.children[0].rect.x, 18);
    assert_eq!(placed.children[1].rect.x, 19);
}

#[test]
fn hstack_justify_space_between() {
    // Three 1-width children in a 10-width HStack
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps {
            width: Some(Dimension::Pixels(10)),
            justify: Some(Justify::SpaceBetween),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "c".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, Size { width: 80, height: 24 }, &metrics());
    // Free space = 10 - 3 = 7, gap = 7/(3-1) = 3
    // x: 0, 0+1+3=4, 4+1+3=8
    assert_eq!(placed.children[0].rect.x, 0);
    assert_eq!(placed.children[1].rect.x, 4);
    assert_eq!(placed.children[2].rect.x, 8);
}

#[test]
fn hstack_justify_space_around() {
    // Two 1-width children in a 10-width HStack
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps {
            width: Some(Dimension::Pixels(10)),
            justify: Some(Justify::SpaceAround),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, Size { width: 80, height: 24 }, &metrics());
    // Free space = 10 - 2 = 8, gap = 8/2 = 4, offset = 4/2 = 2
    // x: 2, 2+1+4=7
    assert_eq!(placed.children[0].rect.x, 2);
    assert_eq!(placed.children[1].rect.x, 7);
}

#[test]
fn hstack_justify_space_evenly() {
    // Two 1-width children in a 10-width HStack
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps {
            width: Some(Dimension::Pixels(10)),
            justify: Some(Justify::SpaceEvenly),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, Size { width: 80, height: 24 }, &metrics());
    // Free space = 10 - 2 = 8, gap = 8/(2+1) = 2
    // x: 2, 2+1+2=5
    assert_eq!(placed.children[0].rect.x, 2);
    assert_eq!(placed.children[1].rect.x, 5);
}

#[test]
fn hstack_justify_with_spacing() {
    // justify and spacing both apply
    let root = Widget::HStack {
        spacing: 1,
        style: StyleProps {
            width: Some(Dimension::Pixels(20)),
            justify: Some(Justify::SpaceBetween),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "c".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, Size { width: 80, height: 24 }, &metrics());
    // Total content = 1*3 + spacing*2 = 5, free = 20-5 = 15
    // gap = 15/2 = 7, actual spacing between = justify_gap + widget_spacing = 7+1=8
    // x: 0, 0+1+8=9, 9+1+8=18
    assert_eq!(placed.children[0].rect.x, 0);
    assert_eq!(placed.children[1].rect.x, 9);
    assert_eq!(placed.children[2].rect.x, 18);
}

// ── Fill widget tests ───────────────────────────────────────────────────────

#[test]
fn fill_in_vstack_fills_remaining_height() {
    // VStack(height: 10) { Text("a"), Fill{}, Text("b") }
    // Text("a")=1, Fill gets remaining, Text("b")=1
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Fill {
                children: vec![],
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Text("a") at y=0, height=1, width=1 (intrinsic)
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 1, 1));
    // Fill at y=1, height=8 (remaining = 10 - 2 = 8), width=80 (fills container)
    assert_eq!(placed.children[1].rect, Rect::new(0, 1, 80, 8));
    // Text("b") at y=9, height=1, width=1 (intrinsic)
    assert_eq!(placed.children[2].rect, Rect::new(0, 9, 1, 1));
}

#[test]
fn fill_in_hstack_fills_remaining_width() {
    // HStack(width: 20) { Text("a"), Fill{}, Text("b") }
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps {
            width: Some(Dimension::Pixels(20)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Fill {
                children: vec![],
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, Size { width: 80, height: 24 }, &metrics());
    // Text("a") at x=0, width=1
    assert_eq!(placed.children[0].rect.x, 0);
    assert_eq!(placed.children[0].rect.width, 1);
    // Fill at x=1, width=18 (remaining = 20 - 2 = 18)
    assert_eq!(placed.children[1].rect.x, 1);
    assert_eq!(placed.children[1].rect.width, 18);
    // Text("b") at x=19, width=1
    assert_eq!(placed.children[2].rect.x, 19);
    assert_eq!(placed.children[2].rect.width, 1);
}

#[test]
fn multiple_fills_share_equally() {
    // VStack(height: 10) { Fill{}, Fill{} }
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Fill {
                children: vec![],
                style: StyleProps::default(),
            },
            Widget::Fill {
                children: vec![],
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Each Fill gets 10/2 = 5
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 80, 5));
    assert_eq!(placed.children[1].rect, Rect::new(0, 5, 80, 5));
}

#[test]
fn fill_with_children_inside() {
    // VStack(height: 10) { Text("a"), Fill { Text("inner") } }
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Fill {
                children: vec![Widget::Text {
                    text: "inner".into(),
                    color: None,
                    style: StyleProps::default(),
                }],
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Text("a") at y=0, height=1, width=1 (intrinsic)
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 1, 1));
    // Fill at y=1, height=9 (remaining = 10 - 1 = 9), width=80 (fills container)
    assert_eq!(placed.children[1].rect, Rect::new(0, 1, 80, 9));
    // Inner text inside Fill at y=1 (relative to Fill), height=1, width=5 (intrinsic)
    assert_eq!(placed.children[1].children[0].rect, Rect::new(0, 1, 5, 1));
}

#[test]
fn fill_with_spacing() {
    // VStack(height: 10, spacing: 1) { Text("a"), Fill{}, Text("b") }
    let root = Widget::VStack {
        spacing: 1,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Fill {
                children: vec![],
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Total content = 1 + 1 (spacing) + 0 (fill) + 1 (spacing) + 1 = 4
    // remaining = 10 - 4 = 6, Fill gets 6
    // Text("a") at y=0, spacing at y=1, Fill at y=2, spacing at y=8, Text("b") at y=9
    assert_eq!(placed.children[0].rect.y, 0);
    assert_eq!(placed.children[1].rect.y, 2);
    assert_eq!(placed.children[1].rect.height, 6);
    assert_eq!(placed.children[2].rect.y, 9);
}

#[test]
fn fill_no_height_uses_content() {
    // VStack without explicit height: Fill should take 0 height
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Fill {
                children: vec![],
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Fill gets 0 height (no remaining space without explicit container height)
    assert_eq!(placed.children[1].rect.height, 0);
    // VStack height = 1 (just the text)
    assert_eq!(placed.rect.height, 1);
}

#[test]
fn fill_in_hstack_multiple() {
    // HStack(width: 10) { Fill{}, Text("x"), Fill{} }
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps {
            width: Some(Dimension::Pixels(10)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Fill {
                children: vec![],
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "x".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Fill {
                children: vec![],
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, Size { width: 80, height: 24 }, &metrics());
    // remaining = 10 - 1 = 9, two fills share: 9/2 = 4 each
    assert_eq!(placed.children[0].rect.x, 0);
    assert_eq!(placed.children[0].rect.width, 4);
    assert_eq!(placed.children[1].rect.x, 4);
    assert_eq!(placed.children[1].rect.width, 1);
    assert_eq!(placed.children[2].rect.x, 5);
    assert_eq!(placed.children[2].rect.width, 4);
}

#[test]
fn fill_painted_ops_empty() {
    use xulo_ui::layout::paint;
    use xulo_ui::Theme;

    let root = Widget::Fill {
        children: vec![],
        style: StyleProps::default(),
    };
    let placed = layout(&root, SURFACE, &metrics());
    let theme = Theme::default();
    let mut ops = Vec::new();
    paint(&placed, &theme, &mut ops);
    // Fill with no children and no background produces no ops
    assert!(ops.is_empty());
}

#[test]
fn fill_painted_with_background() {
    use xulo_ui::layout::paint;
    use xulo_ui::{PaintOp, Theme};

    let root = Widget::Fill {
        children: vec![],
        style: StyleProps {
            background_color: Some(Color::new(255, 0, 0)),
            ..StyleProps::default()
        },
    };
    let placed = layout(&root, SURFACE, &metrics());
    let theme = Theme::default();
    let mut ops = Vec::new();
    paint(&placed, &theme, &mut ops);
    // Fill with background produces a FillRect
    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], PaintOp::FillRect { color: Color { r: 255, g: 0, b: 0 }, .. }));
}

#[test]
fn fill_component_from_props() {
    use xulo_ui::from_props;

    let props = xulo_ui::Props::new()
        .children(vec![Widget::Text {
            text: "inside".into(),
            color: None,
            style: StyleProps::default(),
        }]);
    let widget = from_props("Fill", &props, &mut Vec::new());
    assert!(matches!(widget, Widget::Fill { children, .. } if children.len() == 1));
}

// ── Fill with CellMetrics (pixel-based, like webview) ─────────────────────

#[test]
fn fill_in_vstack_with_cell_metrics() {
    use xulo_ui::layout::CellMetrics;
    let metrics = CellMetrics;
    let surface = Size { width: 320, height: 192 };
    // VStack(height: 24) { Text("a"), Fill{}, Text("b") }
    // CellMetrics: text_width("a")=8, line_height()=16
    // Text("a"): w=8, h=16; Text("b"): w=8, h=16
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(24)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Fill {
                children: vec![],
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, surface, &metrics);
    // Text("a") at y=0, h=16
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 8, 16));
    // Fill at y=16, h=24-16-16=... wait, container_h=24, non_fill=32, free=0!
    // Actually: total_content_h = 16+16=32, container_h=24, available=32, free=0
    // So Fill gets 0 height because content exceeds container.
    // Let me use a bigger container:
    let root2 = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(200)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Fill {
                children: vec![],
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed2 = layout(&root2, surface, &metrics);
    // Text("a") at y=0, h=16
    assert_eq!(placed2.children[0].rect, Rect::new(0, 0, 8, 16));
    // Fill at y=16, h=200-32=168
    assert_eq!(placed2.children[1].rect, Rect::new(0, 16, 320, 168));
    // Text("b") at y=184, h=16
    assert_eq!(placed2.children[2].rect, Rect::new(0, 184, 8, 16));
}

#[test]
fn fill_inside_screen_with_cell_metrics() {
    use xulo_ui::layout::CellMetrics;
    let metrics = CellMetrics;
    let surface = Size { width: 320, height: 192 };
    // Screen { VStack { Header, Fill{}, Footer } } — no explicit height, so
    // VStack inherits the Screen's surface height via override_h.
    // CellMetrics: line_height=16, so Header=16px, Footer=16px
    let root = Widget::Screen {
        background: None,
        style: StyleProps::default(),
        children: vec![
            Widget::VStack {
                spacing: 0,
                style: StyleProps::default(),
                children: vec![
                    Widget::Text {
                        text: "Header".into(),
                        color: None,
                        style: StyleProps::default(),
                    },
                    Widget::Fill {
                        children: vec![],
                        style: StyleProps::default(),
                    },
                    Widget::Text {
                        text: "Footer".into(),
                        color: None,
                        style: StyleProps::default(),
                    },
                ],
            },
        ],
    };
    let placed = layout(&root, surface, &metrics);
    // Screen fills surface (192)
    assert_eq!(placed.rect.height, 192);
    // VStack inherits Screen height (192) via override_h
    let vstack = &placed.children[0];
    assert_eq!(vstack.rect.height, 192);
    // Header at y=0, h=16
    assert_eq!(vstack.children[0].rect, Rect::new(0, 0, 48, 16));
    // Fill at y=16, h=192-32=160
    assert_eq!(vstack.children[1].rect.height, 160);
    // Footer at y=176, h=16
    assert_eq!(vstack.children[2].rect, Rect::new(0, 176, 48, 16));
}

#[test]
fn fill_children_are_placed_inside_fill_rect() {
    use xulo_ui::layout::CellMetrics;
    let metrics = CellMetrics;
    let surface = Size { width: 320, height: 192 };
    // Screen { VStack { Text("top"), Fill { Text("inside") }, Text("bottom") } }
    let root = Widget::Screen {
        background: None,
        style: StyleProps::default(),
        children: vec![
            Widget::VStack {
                spacing: 0,
                style: StyleProps::default(),
                children: vec![
                    Widget::Text {
                        text: "top".into(),
                        color: None,
                        style: StyleProps::default(),
                    },
                    Widget::Fill {
                        children: vec![
                            Widget::Text {
                                text: "inside".into(),
                                color: None,
                                style: StyleProps::default(),
                            },
                        ],
                        style: StyleProps::default(),
                    },
                    Widget::Text {
                        text: "bottom".into(),
                        color: None,
                        style: StyleProps::default(),
                    },
                ],
            },
        ],
    };
    let placed = layout(&root, surface, &metrics);
    let vstack = &placed.children[0];
    // top: y=0, h=16
    assert_eq!(vstack.children[0].rect, Rect::new(0, 0, 24, 16));
    // Fill: y=16, h=192-32=160
    let fill = &vstack.children[1];
    assert_eq!(fill.rect.y, 16);
    assert_eq!(fill.rect.height, 160);
    assert_eq!(fill.rect.width, 320);
    // Fill's child "inside": should be inside Fill's rect
    let inside_text = &fill.children[0];
    assert_eq!(inside_text.rect.y, 16);
    assert_eq!(inside_text.rect.height, 16);
    assert!(inside_text.rect.x < 320, "inside text x={} should be < 320", inside_text.rect.x);
    assert!(inside_text.rect.width > 0, "inside text width={} should be > 0", inside_text.rect.width);
    // bottom: y=176, h=16
    assert_eq!(vstack.children[2].rect, Rect::new(0, 176, 48, 16));
}

// ── Explicit height priority over override_h ────────────────────────────────

#[test]
fn vstack_explicit_height_overrides_screen_override() {
    use xulo_ui::layout::CellMetrics;
    let metrics = CellMetrics;
    let surface = Size { width: 320, height: 192 };
    // Screen { VStack(height: 48) { Fill{} } }
    // VStack has explicit height: 48, which should win over Screen's override_h (192)
    let root = Widget::Screen {
        background: None,
        style: StyleProps::default(),
        children: vec![
            Widget::VStack {
                spacing: 0,
                style: StyleProps {
                    height: Some(Dimension::Pixels(48)),
                    ..StyleProps::default()
                },
                children: vec![
                    Widget::Fill {
                        children: vec![],
                        style: StyleProps::default(),
                    },
                ],
            },
        ],
    };
    let placed = layout(&root, surface, &metrics);
    let vstack = &placed.children[0];
    // VStack should be 48px (explicit height), not 192px (screen override)
    assert_eq!(vstack.rect.height, 48);
    // Fill should get the full 48px
    assert_eq!(vstack.children[0].rect.height, 48);
}

#[test]
fn vstack_no_explicit_height_uses_screen_override() {
    use xulo_ui::layout::CellMetrics;
    let metrics = CellMetrics;
    let surface = Size { width: 320, height: 192 };
    // Screen { VStack { Fill{} } }
    // VStack has no explicit height, so it should use Screen's override_h (192)
    let root = Widget::Screen {
        background: None,
        style: StyleProps::default(),
        children: vec![
            Widget::VStack {
                spacing: 0,
                style: StyleProps::default(),
                children: vec![
                    Widget::Fill {
                        children: vec![],
                        style: StyleProps::default(),
                    },
                ],
            },
        ],
    };
    let placed = layout(&root, surface, &metrics);
    let vstack = &placed.children[0];
    // VStack should be 192px (from screen override)
    assert_eq!(vstack.rect.height, 192);
    // Fill should get the full 192px
    assert_eq!(vstack.children[0].rect.height, 192);
}

// ── Text dual-mode tests ────────────────────────────────────────────────────

#[test]
fn text_pure_mode_in_hstack() {
    // Pure Text (no bg, no padding, no width, no height) in HStack
    // CharMetrics: text_width("hi")=2, line_height()=1
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![
            Widget::Text {
                text: "hi".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "hi".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Pure text: width = text_width = 2, height = line_height = 1
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 2, 1));
    assert_eq!(placed.children[1].rect, Rect::new(2, 0, 2, 1));
}

#[test]
fn text_decorated_mode_with_bg_in_hstack() {
    // Text with bg in HStack: fills remaining space as flex child
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![
            Widget::Text {
                text: "hi".into(),
                color: None,
                style: StyleProps {
                    background_color: Some(Color::new(255, 0, 0)),
                    ..StyleProps::default()
                },
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Single decorated text fills the full HStack width
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 80, 1));
}

#[test]
fn text_decorated_mode_with_padding_in_hstack() {
    // Text with bg + padding in HStack: fills as flex child
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![
            Widget::Text {
                text: "hi".into(),
                color: None,
                style: StyleProps {
                    background_color: Some(Color::new(255, 0, 0)),
                    padding: Some(1),
                    ..StyleProps::default()
                },
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Single decorated text fills the full HStack width (height includes padding)
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 80, 3));
}

#[test]
fn text_decorated_mode_with_width_in_hstack() {
    // Text with explicit width in HStack: fills as flex child (explicit width
    // defines intrinsic size, but flex fill overrides)
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![
            Widget::Text {
                text: "hi".into(),
                color: None,
                style: StyleProps {
                    width: Some(Dimension::Pixels(50)),
                    ..StyleProps::default()
                },
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Single decorated text fills the full HStack width
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 80, 1));
}

#[test]
fn multiple_decorated_texts_share_hspace() {
    // 3 decorated texts share HStack width equally (spacing=2, width=20)
    // Available: 20 - 2*2 = 16. Each gets 16/3 = 5 (integer division)
    let root = Widget::HStack {
        spacing: 2,
        style: StyleProps {
            width: Some(Dimension::Pixels(20)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text {
                text: "A".into(),
                color: None,
                style: StyleProps {
                    background_color: Some(Color::new(255, 0, 0)),
                    ..StyleProps::default()
                },
            },
            Widget::Text {
                text: "B".into(),
                color: None,
                style: StyleProps {
                    background_color: Some(Color::new(0, 255, 0)),
                    ..StyleProps::default()
                },
            },
            Widget::Text {
                text: "C".into(),
                color: None,
                style: StyleProps {
                    background_color: Some(Color::new(0, 0, 255)),
                    ..StyleProps::default()
                },
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // 3 flex children share remaining = 20 - 0 = 20, each gets 20/3 = 6
    // spacing=2 between each: gaps = 2*2 = 4
    // remaining after gaps = 20 - 4 = 16, each gets 16/3 = 5 (integer div)
    assert_eq!(placed.children[0].rect.width, 5);
    assert_eq!(placed.children[1].rect.width, 5);
    assert_eq!(placed.children[2].rect.width, 5);
}

#[test]
fn text_pure_mode_in_vstack() {
    // Pure Text in VStack: gets intrinsic width (VStack fill-width is future flex work)
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![
            Widget::Text {
                text: "hi".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Pure text: intrinsic width = 2, height = 1
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 2, 1));
}

#[test]
fn text_decorated_mode_in_vstack() {
    // Decorated Text in VStack: fills container width
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![
            Widget::Text {
                text: "hi".into(),
                color: None,
                style: StyleProps {
                    background_color: Some(Color::new(255, 0, 0)),
                    ..StyleProps::default()
                },
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Decorated text in VStack fills container width (80)
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 80, 1));
}

#[test]
fn text_decorated_mode_with_explicit_width_in_vstack() {
    // Decorated Text with explicit width in VStack: fills container width
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps::default(),
        children: vec![
            Widget::Text {
                text: "hi".into(),
                color: None,
                style: StyleProps {
                    background_color: Some(Color::new(255, 0, 0)),
                    width: Some(Dimension::Pixels(50)),
                    ..StyleProps::default()
                },
            },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    // Decorated text in VStack fills container width (80), explicit width overridden
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 80, 1));
}

#[test]
fn is_text_decorated_false_for_default() {
    assert!(!StyleProps::default().is_text_decorated());
}

#[test]
fn is_text_decorated_true_for_bg() {
    assert!(StyleProps {
        background_color: Some(Color::new(255, 0, 0)),
        ..StyleProps::default()
    }.is_text_decorated());
}

#[test]
fn is_text_decorated_true_for_padding() {
    assert!(StyleProps {
        padding: Some(2),
        ..StyleProps::default()
    }.is_text_decorated());
}

#[test]
fn is_text_decorated_true_for_width() {
    assert!(StyleProps {
        width: Some(Dimension::Pixels(50)),
        ..StyleProps::default()
    }.is_text_decorated());
}

#[test]
fn is_text_decorated_true_for_height() {
    assert!(StyleProps {
        height: Some(Dimension::Pixels(20)),
        ..StyleProps::default()
    }.is_text_decorated());
}

// ── Min/Max size constraint tests ──────────────────────────────────────────

#[test]
fn text_min_width_enforced() {
    // Text "a" (intrinsic width=1) with min_width=5 → width=5
    let root = Widget::Text {
        text: "a".into(),
        color: None,
        style: StyleProps {
            min_width: Some(5),
            ..StyleProps::default()
        },
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.rect.width, 5);
}

#[test]
fn text_max_width_enforced() {
    // Text "hello" (intrinsic width=5) with max_width=3 → width=3
    let root = Widget::Text {
        text: "hello".into(),
        color: None,
        style: StyleProps {
            max_width: Some(3),
            ..StyleProps::default()
        },
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.rect.width, 3);
}

#[test]
fn text_min_and_max_width() {
    // Text "a" (intrinsic width=1) with min_width=3, max_width=10 → width=3
    let root = Widget::Text {
        text: "a".into(),
        color: None,
        style: StyleProps {
            min_width: Some(3),
            max_width: Some(10),
            ..StyleProps::default()
        },
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.rect.width, 3);
}

#[test]
fn text_min_height_enforced() {
    // Text "a" (intrinsic height=1) with min_height=4 → height=4
    let root = Widget::Text {
        text: "a".into(),
        color: None,
        style: StyleProps {
            min_height: Some(4),
            ..StyleProps::default()
        },
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.rect.height, 4);
}

#[test]
fn text_max_height_enforced() {
    // Text "a" with height=10, max_height=3 → height=3
    let root = Widget::Text {
        text: "a".into(),
        color: None,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            max_height: Some(3),
            ..StyleProps::default()
        },
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.rect.height, 3);
}

#[test]
fn button_min_width_enforced() {
    let root = Widget::Button {
        label: "OK".into(),
        style: StyleProps {
            min_width: Some(20),
            ..StyleProps::default()
        },
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.rect.width, 20);
}

#[test]
fn vstack_min_height_enforced() {
    // VStack with one child (height=1), min_height=10 → height=10
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            min_height: Some(10),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text { text: "a".into(), color: None, style: StyleProps::default() },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.rect.height, 10);
}

#[test]
fn hstack_max_width_enforced() {
    // HStack with two 1-char children (intrinsic width=2), max_width=1 → width=1
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps {
            max_width: Some(1),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text { text: "a".into(), color: None, style: StyleProps::default() },
            Widget::Text { text: "b".into(), color: None, style: StyleProps::default() },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.rect.width, 1);
}

// ── flex_grow tests ────────────────────────────────────────────────────────

#[test]
fn flex_grow_in_vstack() {
    // VStack(height: 10) { Text("a"), Text("b", flex_grow: 1.0) }
    // Text("a")=1, remaining=9 → Text("b") gets 9
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text { text: "a".into(), color: None, style: StyleProps::default() },
            Widget::Text { text: "b".into(), color: None, style: StyleProps { flex_grow: Some(1.0), ..StyleProps::default() } },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.children[0].rect.y, 0);
    assert_eq!(placed.children[0].rect.height, 1);
    assert_eq!(placed.children[1].rect.y, 1);
    assert_eq!(placed.children[1].rect.height, 9);
}

#[test]
fn flex_grow_proportional_in_vstack() {
    // VStack(height: 10) { Text("a"), Text("b", flex_grow: 1.0), Text("c", flex_grow: 3.0) }
    // Text("a")=1, remaining=9 → b=2, c=7 (1:3 ratio)
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text { text: "a".into(), color: None, style: StyleProps::default() },
            Widget::Text { text: "b".into(), color: None, style: StyleProps { flex_grow: Some(1.0), ..StyleProps::default() } },
            Widget::Text { text: "c".into(), color: None, style: StyleProps { flex_grow: Some(3.0), ..StyleProps::default() } },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.children[0].rect, Rect::new(0, 0, 1, 1));
    // remaining = 9, total_grow = 4, b = 9*1/4 = 2.25→2, c = 9*3/4 = 6.75→6
    assert_eq!(placed.children[1].rect.height, 2);
    assert_eq!(placed.children[2].rect.height, 6);
}

#[test]
fn flex_grow_in_hstack() {
    // HStack(width: 10) { Text("a"), Text("b", flex_grow: 1.0) }
    // Text("a")=1, remaining=9 → Text("b") gets 9
    let root = Widget::HStack {
        spacing: 0,
        style: StyleProps {
            width: Some(Dimension::Pixels(10)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text { text: "a".into(), color: None, style: StyleProps::default() },
            Widget::Text { text: "b".into(), color: None, style: StyleProps { flex_grow: Some(1.0), ..StyleProps::default() } },
        ],
    };
    let placed = layout(&root, Size { width: 80, height: 24 }, &metrics());
    assert_eq!(placed.children[0].rect.x, 0);
    assert_eq!(placed.children[0].rect.width, 1);
    assert_eq!(placed.children[1].rect.x, 1);
    assert_eq!(placed.children[1].rect.width, 9);
}

#[test]
fn flex_grow_zero_does_not_expand() {
    // VStack(height: 5) { Text("a", flex_grow: 0.0) }
    // flex_grow=0 → no expansion, VStack shrinks to content
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(5)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text { text: "a".into(), color: None, style: StyleProps { flex_grow: Some(0.0), ..StyleProps::default() } },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.children[0].rect.height, 1);
}

#[test]
fn fill_is_flex_grow_1_by_default() {
    // Fill should behave like flex_grow: 1.0
    let root = Widget::VStack {
        spacing: 0,
        style: StyleProps {
            height: Some(Dimension::Pixels(10)),
            ..StyleProps::default()
        },
        children: vec![
            Widget::Text { text: "a".into(), color: None, style: StyleProps::default() },
            Widget::Fill { children: vec![], style: StyleProps::default() },
        ],
    };
    let placed = layout(&root, SURFACE, &metrics());
    assert_eq!(placed.children[0].rect.height, 1);
    assert_eq!(placed.children[1].rect.height, 9);
}
