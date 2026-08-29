//! Painting (paint-command emission) and color parsing tests.

use xulo_ui::{StyleProps, Color, PaintOp, Rect, Size, Widget};

struct CharMetrics;

impl xulo_ui::FontMetrics for CharMetrics {
    fn text_width(&self, text: &str) -> u32 {
        text.chars().count() as u32
    }
    fn line_height(&self) -> u32 {
        1
    }
}

fn paint(root: &Widget) -> Vec<PaintOp<'_>> {
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
            style: StyleProps::default(),
        }],
        style: StyleProps::default(),
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
            text: "hi",
            color: Color::WHITE,
            font_size: 12,
            bold: false,
        }
    );
}

#[test]
fn button_paints_border_and_accent_text() {
    let root = Widget::Button { label: "+".into(), style: StyleProps::default() };
    let ops = paint(&root);
    assert_eq!(
        ops[0],
        PaintOp::DrawBorder {
            rect: Rect::new(0, 0, 3, 3),
            color: Color::GRAY,
            border_radius: 0,
        }
    );
    assert_eq!(
        ops[1],
        PaintOp::DrawText {
            rect: Rect::new(1, 1, 1, 1),
            text: "+",
            color: Color::ACCENT,
            font_size: 12,
            bold: false,
        }
    );
}

#[test]
fn text_honors_its_color() {
    let root = Widget::Text {
        text: "hi".into(),
        color: Some(Color::new(1, 2, 3)),
        style: StyleProps::default(),
    };
    let ops = paint(&root);
    assert_eq!(
        ops[0],
        PaintOp::DrawText {
            rect: Rect::new(0, 0, 2, 1),
            text: "hi",
            color: Color::new(1, 2, 3),
            font_size: 12,
            bold: false,
        }
    );
}

#[test]
fn text_without_color_uses_theme() {
    let root = Widget::Text {
        text: "hi".into(),
        color: None,
        style: StyleProps::default(),
    };
    let ops = paint(&root);
    assert_eq!(
        ops[0],
        PaintOp::DrawText {
            rect: Rect::new(0, 0, 2, 1),
            text: "hi",
            color: Color::WHITE,
            font_size: 12,
            bold: false,
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
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "b".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
        style: StyleProps::default(),
    };
    let ops = paint(&root);
    assert_eq!(
        ops,
        vec![
            PaintOp::DrawText {
                rect: Rect::new(0, 0, 1, 1),
                text: "a",
                color: Color::WHITE,
                font_size: 12,
                bold: false,
            },
            PaintOp::DrawText {
                rect: Rect::new(0, 2, 1, 1),
                text: "b",
                color: Color::WHITE,
                font_size: 12,
                bold: false,
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

// ── fontSize tests ──────────────────────────────────────────────────────────

#[test]
fn text_with_custom_font_size() {
    let root = Widget::Text {
        text: "hi".into(),
        color: None,
        style: StyleProps {
            font_size: Some(16),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    assert_eq!(
        ops[0],
        PaintOp::DrawText {
            rect: Rect::new(0, 0, 2, 1),
            text: "hi",
            color: Color::WHITE,
            font_size: 16,
            bold: false,
        }
    );
}

#[test]
fn button_with_custom_font_size() {
    let root = Widget::Button {
        label: "OK".into(),
        style: StyleProps {
            font_size: Some(20),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    // Button has 3 ops: DrawBorder, DrawText
    assert!(ops.iter().any(|op| matches!(op, PaintOp::DrawText { font_size: 20, .. })));
}

#[test]
fn text_default_font_size() {
    let root = Widget::Text {
        text: "hello".into(),
        color: None,
        style: StyleProps::default(),
    };
    let ops = paint(&root);
    assert_eq!(
        ops[0],
        PaintOp::DrawText {
            rect: Rect::new(0, 0, 5, 1),
            text: "hello",
            color: Color::WHITE,
            font_size: 12,
            bold: false,
        }
    );
}

// ── fontWeight (bold) tests ─────────────────────────────────────────────────

#[test]
fn text_with_bold_weight() {
    let root = Widget::Text {
        text: "bold".into(),
        color: None,
        style: StyleProps {
            font_weight: Some(xulo_ui::FontWeight::Bold),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    assert_eq!(
        ops[0],
        PaintOp::DrawText {
            rect: Rect::new(0, 0, 4, 1),
            text: "bold",
            color: Color::WHITE,
            font_size: 12,
            bold: true,
        }
    );
}

#[test]
fn text_with_normal_weight() {
    let root = Widget::Text {
        text: "normal".into(),
        color: None,
        style: StyleProps {
            font_weight: Some(xulo_ui::FontWeight::Normal),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    assert_eq!(
        ops[0],
        PaintOp::DrawText {
            rect: Rect::new(0, 0, 6, 1),
            text: "normal",
            color: Color::WHITE,
            font_size: 12,
            bold: false,
        }
    );
}

#[test]
fn button_with_bold_label() {
    let root = Widget::Button {
        label: "OK".into(),
        style: StyleProps {
            font_weight: Some(xulo_ui::FontWeight::Bold),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    assert!(ops.iter().any(|op| matches!(op, PaintOp::DrawText { bold: true, .. })));
}

// ── Text backgroundColor tests ──────────────────────────────────────────────

#[test]
fn text_with_background_color() {
    let root = Widget::Text {
        text: "hi".into(),
        color: None,
        style: StyleProps {
            background_color: Some(Color::new(255, 0, 0)),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    // Should have FillRect + DrawText
    assert_eq!(ops.len(), 2);
    assert!(matches!(&ops[0], PaintOp::FillRect { color: Color { r: 255, g: 0, b: 0 }, .. }));
    assert!(matches!(&ops[1], PaintOp::DrawText { .. }));
}

#[test]
fn text_without_background_color() {
    let root = Widget::Text {
        text: "hi".into(),
        color: None,
        style: StyleProps::default(),
    };
    let ops = paint(&root);
    // Should have only DrawText (no FillRect)
    assert_eq!(ops.len(), 1);
    assert!(matches!(&ops[0], PaintOp::DrawText { .. }));
}

#[test]
fn text_with_background_and_border_radius() {
    let root = Widget::Text {
        text: "hi".into(),
        color: None,
        style: StyleProps {
            background_color: Some(Color::new(0, 255, 0)),
            border_radius: Some(5),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    assert!(matches!(&ops[0], PaintOp::FillRect { border_radius: 5, .. }));
}

// ── Input style props tests ─────────────────────────────────────────────────

#[test]
fn input_with_border_color() {
    let root = Widget::Input {
        value: "hello".into(),
        width: None,
        placeholder: "Enter".into(),
        style: StyleProps {
            border_color: Some(Color::new(255, 0, 0)),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    // Should have DrawBorder + Input
    assert!(ops.iter().any(|op| matches!(op, PaintOp::DrawBorder { color: Color { r: 255, g: 0, b: 0 }, .. })));
}

#[test]
fn input_with_background_color() {
    let root = Widget::Input {
        value: "".into(),
        width: None,
        placeholder: "Enter".into(),
        style: StyleProps {
            background_color: Some(Color::new(240, 240, 240)),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    // Should have FillRect + Input
    assert!(ops.iter().any(|op| matches!(op, PaintOp::FillRect { color: Color { r: 240, g: 240, b: 240 }, .. })));
}

#[test]
fn input_empty_shows_placeholder_color() {
    let root = Widget::Input {
        value: "".into(),
        width: None,
        placeholder: "Enter name".into(),
        style: StyleProps::default(),
    };
    let ops = paint(&root);
    // Input op should have gray color for placeholder
    if let Some(PaintOp::Input { color, text, .. }) = ops.last() {
        assert_eq!(*text, "");
        assert_eq!(*color, Color::GRAY);
    } else {
        panic!("Expected Input paint op");
    }
}

#[test]
fn input_with_value_shows_text_color() {
    let root = Widget::Input {
        value: "hello".into(),
        width: None,
        placeholder: "Enter".into(),
        style: StyleProps {
            color: Some(Color::new(255, 0, 0)),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    if let Some(PaintOp::Input { color, text, .. }) = ops.last() {
        assert_eq!(*text, "hello");
        assert_eq!(*color, Color::new(255, 0, 0));
    } else {
        panic!("Expected Input paint op");
    }
}
