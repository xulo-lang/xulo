//! Layout integration tests using a character-cell font metric.

use xulo_ui::layout::layout;
use xulo_ui::{StyleProps, Color, FontMetrics, Justify, Rect, Size, Widget};

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
            height: Some(5),
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
            height: Some(5),
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
            height: Some(5),
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
            height: Some(10),
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
            height: Some(10),
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
            height: Some(10),
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
            height: Some(10),
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
            height: Some(10),
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
            height: Some(10),
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
            height: Some(10),
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
            width: Some(20),
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
            width: Some(20),
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
            width: Some(10),
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
            width: Some(10),
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
            width: Some(10),
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
            width: Some(20),
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
