//! Painting (paint-command emission) and color parsing tests.

use xulo_ui::{Color, PaintOp, Rect, Size, Widget};

struct CharMetrics;

impl xulo_ui::FontMetrics for CharMetrics {
    fn text_width(&self, text: &str) -> u32 {
        text.chars().count() as u32
    }
    fn line_height(&self) -> u32 {
        1
    }
}

fn paint(root: &Widget) -> Vec<PaintOp> {
    xulo_ui::UiContext::new(
        Size {
            width: 80,
            height: 24,
        },
        Box::new(CharMetrics),
    )
    .paint(root)
}

#[test]
fn screen_clears_with_its_background() {
    let root = Widget::Screen {
        background: Some(Color::parse_hex("#f0f0f0").unwrap()),
        children: vec![Widget::Text {
            text: "hi".into(),
            color: None,
        }],
    };
    let ops = paint(&root);
    assert_eq!(
        ops[0],
        PaintOp::Clear {
            color: Color::new(240, 240, 240)
        }
    );
    assert_eq!(
        ops[1],
        PaintOp::DrawText {
            rect: Rect::new(0, 0, 2, 1),
            text: "hi".into(),
            color: Color::WHITE,
        }
    );
}

#[test]
fn screen_defaults_to_theme_background() {
    let root = Widget::Screen {
        background: None,
        children: Vec::new(),
    };
    let ops = paint(&root);
    assert_eq!(ops[0], PaintOp::Clear { color: Color::DARK });
}

#[test]
fn button_paints_border_and_accent_text() {
    let root = Widget::Button { label: "+".into() };
    let ops = paint(&root);
    assert_eq!(
        ops[0],
        PaintOp::DrawBorder {
            rect: Rect::new(0, 0, 3, 3),
            color: Color::GRAY,
        }
    );
    assert_eq!(
        ops[1],
        PaintOp::DrawText {
            rect: Rect::new(1, 1, 1, 1),
            text: "+".into(),
            color: Color::ACCENT,
        }
    );
}

#[test]
fn text_honors_its_color() {
    let root = Widget::Text {
        text: "hi".into(),
        color: Some(Color::new(1, 2, 3)),
    };
    let ops = paint(&root);
    assert_eq!(
        ops[0],
        PaintOp::DrawText {
            rect: Rect::new(0, 0, 2, 1),
            text: "hi".into(),
            color: Color::new(1, 2, 3),
        }
    );
}

#[test]
fn text_without_color_uses_theme() {
    let root = Widget::Text {
        text: "hi".into(),
        color: None,
    };
    let ops = paint(&root);
    assert_eq!(
        ops[0],
        PaintOp::DrawText {
            rect: Rect::new(0, 0, 2, 1),
            text: "hi".into(),
            color: Color::WHITE,
        }
    );
}

#[test]
fn vstack_paints_children_in_order() {
    let root = Widget::VStack {
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
    let ops = paint(&root);
    assert_eq!(
        ops,
        vec![
            PaintOp::DrawText {
                rect: Rect::new(0, 0, 1, 1),
                text: "a".into(),
                color: Color::WHITE,
            },
            PaintOp::DrawText {
                rect: Rect::new(0, 2, 1, 1),
                text: "b".into(),
                color: Color::WHITE,
            },
        ]
    );
}

#[test]
fn parse_hex_colors() {
    assert_eq!(Color::parse_hex("#f0f0f0"), Some(Color::new(240, 240, 240)));
    assert_eq!(Color::parse_hex("f00"), Some(Color::new(255, 0, 0)));
    assert_eq!(Color::parse_hex("#fff"), Some(Color::new(255, 255, 255)));
    assert_eq!(Color::parse_hex("#123456"), Some(Color::new(18, 52, 86)));
    assert_eq!(Color::parse_hex("#12345"), None);
    assert_eq!(Color::parse_hex("#zzzzzz"), None);
    assert_eq!(Color::parse_hex("notacolor"), None);
}

#[test]
fn rect_helpers() {
    let r = Rect::new(2, 3, 4, 5);
    assert_eq!(r.right(), 6);
    assert_eq!(r.bottom(), 8);
    assert!(r.contains(2, 3));
    assert!(r.contains(5, 7));
    assert!(!r.contains(6, 3));
    assert!(!r.contains(2, 8));
    let a = Rect::new(0, 0, 10, 10);
    let b = Rect::new(5, 5, 10, 10);
    assert_eq!(a.intersect(&b), Some(Rect::new(5, 5, 5, 5)));
    assert_eq!(a.intersect(&Rect::new(20, 20, 1, 1)), None);
}
