//! Terminal backend integration tests.

use xulo_renderer_terminal::{render_ansi, render_plain, CharMetrics, TerminalSize};
use xulo_ui::{Color, PaintOp, Rect, Size, Widget};

const SIZE: TerminalSize = TerminalSize { cols: 40, rows: 12 };

fn paint(root: &Widget) -> Vec<PaintOp<'_>> {
    xulo_ui::UiContext::new(
        Size {
            width: SIZE.cols,
            height: SIZE.rows,
        },
        Box::new(CharMetrics),
    )
    .paint(root)
}

#[test]
fn renders_text_rows_plain() {
    let root = Widget::VStack {
        spacing: 1,
        children: vec![
            Widget::Text {
                text: "hello".into(),
                color: None,
            },
            Widget::Text {
                text: "world".into(),
                color: None,
            },
        ],
    };
    let ops = paint(&root);
    assert_eq!(render_plain(&ops, SIZE), "hello\n\nworld");
}

#[test]
fn button_draws_border_and_label() {
    let root = Widget::Button { label: "+".into() };
    let ops = paint(&root);
    assert_eq!(render_plain(&ops, SIZE), "┌─┐\n│+│\n└─┘");
}

#[test]
fn hstack_places_buttons_next_to_each_other() {
    let root = Widget::HStack {
        spacing: 2,
        children: vec![
            Widget::Button { label: "+".into() },
            Widget::Button { label: "-".into() },
        ],
    };
    let ops = paint(&root);
    assert_eq!(render_plain(&ops, SIZE), "┌─┐  ┌─┐\n│+│  │-│\n└─┘  └─┘");
}

#[test]
fn long_text_is_clipped_at_screen_edge() {
    // A full-width Screen + a text longer than the surface must not overrun.
    let root = Widget::Screen {
        background: None,
        children: vec![Widget::Text {
            text: "abcdefghijklmnopqrstuvwxyz".into(),
            color: None,
        }],
    };
    let small = TerminalSize { cols: 5, rows: 3 };
    let ctx = xulo_ui::UiContext::new(
        Size {
            width: small.cols,
            height: small.rows,
        },
        Box::new(CharMetrics),
    );
    let ops = ctx.paint(&root);
    assert_eq!(render_plain(&ops, small), "abcde");
}

#[test]
fn ansi_output_carries_truecolor_escapes() {
    let root = Widget::Text {
        text: "hi".into(),
        color: None,
    };
    let ops = paint(&root);
    let ansi = render_ansi(&ops, SIZE);
    assert!(ansi.contains("\x1b[38;2;255;255;255mhi"), "got: {ansi:?}");
}

#[test]
fn fill_rect_sets_cell_background() {
    let ops = vec![PaintOp::FillRect {
        rect: Rect::new(0, 0, 3, 2),
        color: Color::new(1, 2, 3),
    }];
    let ansi = render_ansi(&ops, SIZE);
    assert!(ansi.contains("\x1b[48;2;1;2;3m"));
}
