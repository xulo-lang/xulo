//! Test: layout.xulo rendered with TaffyLayout engine.

use xulo_ui::{
    CellMetrics, LayoutEngine, Size, StyleProps, TaffyLayout, Widget,
};

fn main() {
    // Recreate the layout.xulo structure
    let root = Widget::Screen {
        background: None,
        children: vec![Widget::VStack {
            spacing: 0,
            children: vec![
                // Top bar
                Widget::HStack {
                    spacing: 0,
                    children: vec![Widget::Text {
                        text: "Top".into(),
                        color: Some(xulo_ui::Color::WHITE),
                        style: StyleProps {
                            width: Some(xulo_ui::Dimension::Full),
                            padding: Some(2),
                            background_color: Some(xulo_ui::Color {
                                r: 0xe9,
                                g: 0x45,
                                b: 0x60,
                            }),
                            ..StyleProps::default()
                        },
                    }],
                    style: StyleProps {
                        width: Some(xulo_ui::Dimension::Full),
                        padding: Some(2),
                        background_color: Some(xulo_ui::Color {
                            r: 0xe9,
                            g: 0x45,
                            b: 0x60,
                        }),
                        ..StyleProps::default()
                    },
                },
                // Fill area
                Widget::Fill {
                    children: vec![Widget::VStack {
                        spacing: 1,
                        children: vec![
                            // Row with A, B, C
                            Widget::HStack {
                                spacing: 2,
                                children: vec![
                                    Widget::Text {
                                        text: "A".into(),
                                        color: Some(xulo_ui::Color {
                                            r: 0xcc,
                                            g: 0xd6,
                                            b: 0xf6,
                                        }),
                                        style: StyleProps {
                                            background_color: Some(xulo_ui::Color {
                                                r: 0x4a,
                                                g: 0x90,
                                                b: 0xd9,
                                            }),
                                            padding: Some(1),
                                            ..StyleProps::default()
                                        },
                                    },
                                    Widget::Text {
                                        text: "B".into(),
                                        color: Some(xulo_ui::Color {
                                            r: 0xcc,
                                            g: 0xd6,
                                            b: 0xf6,
                                        }),
                                        style: StyleProps {
                                            background_color: Some(xulo_ui::Color {
                                                r: 0xd9,
                                                g: 0x4a,
                                                b: 0x4a,
                                            }),
                                            padding: Some(1),
                                            ..StyleProps::default()
                                        },
                                    },
                                    Widget::Text {
                                        text: "C".into(),
                                        color: Some(xulo_ui::Color {
                                            r: 0xcc,
                                            g: 0xd6,
                                            b: 0xf6,
                                        }),
                                        style: StyleProps {
                                            background_color: Some(xulo_ui::Color {
                                                r: 0x55,
                                                g: 0x55,
                                                b: 0x55,
                                            }),
                                            padding: Some(1),
                                            ..StyleProps::default()
                                        },
                                    },
                                ],
                                style: StyleProps {
                                    width: Some(xulo_ui::Dimension::Full),
                                    background_color: Some(xulo_ui::Color {
                                        r: 0xbe,
                                        g: 0x42,
                                        b: 0x19,
                                    }),
                                    ..StyleProps::default()
                                },
                            },
                            // Row with Left, Center, Right
                            Widget::HStack {
                                spacing: 0,
                                children: vec![
                                    Widget::Text {
                                        text: "Left".into(),
                                        color: Some(xulo_ui::Color {
                                            r: 0xcc,
                                            g: 0xd6,
                                            b: 0xf6,
                                        }),
                                        style: StyleProps::default(),
                                    },
                                    Widget::Text {
                                        text: "Center".into(),
                                        color: Some(xulo_ui::Color {
                                            r: 0xcc,
                                            g: 0xd6,
                                            b: 0xf6,
                                        }),
                                        style: StyleProps::default(),
                                    },
                                    Widget::Text {
                                        text: "Right".into(),
                                        color: Some(xulo_ui::Color {
                                            r: 0xcc,
                                            g: 0xd6,
                                            b: 0xf6,
                                        }),
                                        style: StyleProps::default(),
                                    },
                                ],
                                style: StyleProps {
                                    width: Some(xulo_ui::Dimension::Full),
                                    padding: Some(2),
                                    background_color: Some(xulo_ui::Color {
                                        r: 0x0f,
                                        g: 0x34,
                                        b: 0x60,
                                    }),
                                    justify: Some(xulo_ui::Justify::SpaceBetween),
                                    ..StyleProps::default()
                                },
                            },
                        ],
                        style: StyleProps {
                            padding: Some(2),
                            ..StyleProps::default()
                        },
                    }],
                    style: StyleProps {
                        background_color: Some(xulo_ui::Color {
                            r: 0x1d,
                            g: 0xa9,
                            b: 0x6f,
                        }),
                        ..StyleProps::default()
                    },
                },
                // Bottom bar
                Widget::HStack {
                    spacing: 0,
                    children: vec![Widget::Text {
                        text: "Bottom".into(),
                        color: Some(xulo_ui::Color::WHITE),
                        style: StyleProps::default(),
                    }],
                    style: StyleProps {
                        width: Some(xulo_ui::Dimension::Full),
                        padding: Some(2),
                        background_color: Some(xulo_ui::Color {
                            r: 0x55,
                            g: 0x55,
                            b: 0x55,
                        }),
                        ..StyleProps::default()
                    },
                },
            ],
            style: StyleProps {
                justify: Some(xulo_ui::Justify::SpaceBetween),
                ..StyleProps::default()
            },
        }],
        style: StyleProps {
            background_color: Some(xulo_ui::Color {
                r: 0x0a,
                g: 0x0a,
                b: 0x1a,
            }),
            ..StyleProps::default()
        },
    };

    let surface = Size {
        width: 80,
        height: 24,
    };
    let metrics = CellMetrics;

    // Taffy engine
    let tf = TaffyLayout;
    let placed_tf = tf.layout(&root, surface, &metrics);
    println!("=== TaffyLayout (layout.xulo) ===");
    print_placed(&placed_tf, 0);

    // Handwritten engine for comparison
    let hw = xulo_ui::HandwrittenLayout;
    let placed_hw = hw.layout(&root, surface, &metrics);
    println!("\n=== HandwrittenLayout (layout.xulo) ===");
    print_placed(&placed_hw, 0);
}

fn print_placed(p: &xulo_ui::Placed<'_>, indent: usize) {
    let pad = "  ".repeat(indent);
    let name = match &p.widget {
        Widget::Screen { .. } => "Screen",
        Widget::VStack { .. } => "VStack",
        Widget::HStack { .. } => "HStack",
        Widget::Fill { .. } => "Fill",
        Widget::Text { text, .. } => {
            println!(
                "{}Text \"{}\" @ ({}, {}) {}×{}",
                pad, text, p.rect.x, p.rect.y, p.rect.width, p.rect.height
            );
            return;
        }
        Widget::Button { label, .. } => {
            println!(
                "{}Button \"{}\" @ ({}, {}) {}×{}",
                pad, label, p.rect.x, p.rect.y, p.rect.width, p.rect.height
            );
            return;
        }
        Widget::Input { .. } => "Input",
        Widget::Unknown { kind, .. } => kind.as_str(),
        Widget::ScrollView { .. } => "ScrollView",
    };
    println!(
        "{}{} @ ({}, {}) {}×{}",
        pad, name, p.rect.x, p.rect.y, p.rect.width, p.rect.height
    );
    for child in &p.children {
        print_placed(child, indent + 1);
    }
}
