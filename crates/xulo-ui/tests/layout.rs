//! Layout integration tests using a character-cell font metric.

use xulo_ui::layout::layout;
use xulo_ui::{Color, FontMetrics, Rect, Size, Widget};

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
        children: vec![
            Widget::Text {
                text: "aaa".into(),
                color: None,
            },
            Widget::Text {
                text: "bb".into(),
                color: None,
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
        children: vec![Widget::Text {
            text: "x".into(),
            color: None,
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
        children: vec![
            Widget::Text {
                text: "a".into(),
                color: None,
            },
            Widget::Text {
                text: "b".into(),
                color: None,
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
        children: vec![
            Widget::Button { label: "+".into() },
            Widget::Button { label: "-".into() },
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
        children: vec![
            Widget::VStack {
                spacing: 0,
                children: vec![
                    Widget::Text {
                        text: "hello".into(),
                        color: None,
                    },
                    Widget::Text {
                        text: "x".into(),
                        color: None,
                    },
                ],
            },
            Widget::Text {
                text: "!".into(),
                color: None,
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
        children: vec![Widget::Text {
            text: "a_very_long_line".into(),
            color: None,
        }],
    };
    // Give the layout a narrow bound via a Screen, whose width is the surface.
    let screen = Widget::Screen {
        background: None,
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
        children: vec![Widget::Text {
            text: "hi".into(),
            color: None,
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
