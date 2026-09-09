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
            opacity: 1.0,
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
            opacity: 1.0,
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
            opacity: 1.0,
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
            opacity: 1.0,
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
            opacity: 1.0,
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
                opacity: 1.0,
            },
            PaintOp::DrawText {
                rect: Rect::new(0, 2, 1, 1),
                text: "b",
                color: Color::WHITE,
                font_size: 12,
                bold: false,
                opacity: 1.0,
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
            opacity: 1.0,
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
            opacity: 1.0,
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
            opacity: 1.0,
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
            opacity: 1.0,
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

// ── opacity tests ─────────────────────────────────────────────────────────

#[test]
fn text_with_opacity() {
    let root = Widget::Text {
        text: "hi".into(),
        color: None,
        style: StyleProps {
            opacity: Some(0.5),
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
            font_size: 12,
            bold: false,
            opacity: 0.5,
        }
    );
}

#[test]
fn text_default_opacity_is_one() {
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
            opacity: 1.0,
        }
    );
}

#[test]
fn button_with_opacity() {
    let root = Widget::Button {
        label: "OK".into(),
        style: StyleProps {
            opacity: Some(0.3),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    assert!(ops.iter().any(|op| matches!(op, PaintOp::DrawBorder { opacity: 0.3, .. })));
    assert!(ops.iter().any(|op| matches!(op, PaintOp::DrawText { opacity: 0.3, .. })));
}

#[test]
fn fill_rect_with_opacity() {
    let root = Widget::Text {
        text: "x".into(),
        color: None,
        style: StyleProps {
            background_color: Some(Color::new(0, 0, 0)),
            opacity: Some(0.7),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    assert!(matches!(&ops[0], PaintOp::FillRect { opacity: 0.7, .. }));
}

#[test]
fn fill_children_produce_paint_ops() {
    use xulo_ui::layout::CellMetrics;
    let metrics = CellMetrics;
    let surface = Size { width: 320, height: 192 };
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
    let placed = xulo_ui::layout::layout(&root, surface, &metrics);
    let mut ops = Vec::new();
    xulo_ui::layout::paint(&placed, &xulo_ui::Theme::default(), &mut ops);
    let texts: Vec<_> = ops.iter().filter_map(|op| match op {
        PaintOp::DrawText { text, rect, .. } => Some((*text, *rect)),
        _ => None,
    }).collect();
    assert_eq!(texts.len(), 3);
    assert_eq!(texts[0].0, "top");
    assert_eq!(texts[0].1.y, 0);
    assert_eq!(texts[1].0, "inside");
    assert_eq!(texts[1].1.y, 16);
    assert_eq!(texts[2].0, "bottom");
    assert_eq!(texts[2].1.y, 176);
}

#[test]
fn fill_with_background_produces_fillrect_op() {
    let root = Widget::Screen {
        background: None,
        style: StyleProps::default(),
        children: vec![
            Widget::Fill {
                children: vec![],
                style: StyleProps {
                    background_color: Some(Color::parse_hex("#ff0000").unwrap()),
                    ..StyleProps::default()
                },
            },
        ],
    };
    let ops = paint(&root);
    let rects: Vec<_> = ops.iter().filter_map(|op| match op {
        PaintOp::FillRect { rect, color, .. } => Some((*rect, *color)),
        _ => None,
    }).collect();
    assert_eq!(rects.len(), 1);
    assert_eq!(rects[0].1, Color::parse_hex("#ff0000").unwrap());
}

#[test]
fn scrollview_emits_scrollstart_and_scrollend() {
    xulo_ui::layout::reset_scroll_state();
    let root = Widget::ScrollView {
        axes: xulo_ui::widgets::Axes::Vertical,
        children: vec![
            Widget::Text { text: "item1".into(), color: None, style: StyleProps::default() },
            Widget::Text { text: "item2".into(), color: None, style: StyleProps::default() },
        ],
        style: StyleProps {
            width: Some(xulo_ui::widgets::Dimension::Pixels(200)),
            height: Some(xulo_ui::widgets::Dimension::Pixels(100)),
            ..StyleProps::default()
        },
    };
    let ops = paint(&root);
    let has_scroll_start = ops.iter().any(|op| matches!(op, PaintOp::ScrollStart { .. }));
    let has_scroll_end = ops.iter().any(|op| matches!(op, PaintOp::ScrollEnd));
    assert!(has_scroll_start, "ScrollView should emit ScrollStart");
    assert!(has_scroll_end, "ScrollView should emit ScrollEnd");
}
