//! Example: comparing HandwrittenLayout vs TaffyLayout.

use xulo_ui::{
    CellMetrics, HandwrittenLayout, LayoutEngine, Size, StyleProps, TaffyLayout, Widget,
};

fn main() {
    let root = Widget::VStack {
        spacing: 1,
        children: vec![
            Widget::Text {
                text: "Top".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Fill {
                children: vec![Widget::Text {
                    text: "Fill".into(),
                    color: None,
                    style: StyleProps::default(),
                }],
                style: StyleProps::default(),
            },
            Widget::Text {
                text: "Bottom".into(),
                color: None,
                style: StyleProps::default(),
            },
        ],
        style: StyleProps::default(),
    };

    let surface = Size {
        width: 40,
        height: 10,
    };
    let metrics = CellMetrics;

    // Handwritten engine
    let hw = HandwrittenLayout;
    let placed_hw = hw.layout(&root, surface, &metrics);
    println!("=== HandwrittenLayout ===");
    print_placed(&placed_hw, 0);

    // Taffy engine
    let tf = TaffyLayout;
    let placed_tf = tf.layout(&root, surface, &metrics);
    println!("\n=== TaffyLayout ===");
    print_placed(&placed_tf, 0);
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
