//! Test: height property handling.

use xulo_ui::{
    FontMetrics, LayoutEngine, Size, StyleProps, TaffyLayout, Widget,
};

struct TestMetrics;

impl FontMetrics for TestMetrics {
    fn text_width(&self, text: &str) -> u32 {
        text.chars().count() as u32
    }
    fn line_height(&self) -> u32 {
        1
    }
}

fn main() {
    // Test 1: height: "full"
    let root1 = Widget::Screen {
        background: None,
        children: vec![Widget::VStack {
            spacing: 0,
            children: vec![Widget::Fill {
                children: vec![],
                style: StyleProps::default(),
            }],
            style: StyleProps {
                height: Some(xulo_ui::Dimension::Full),
                ..StyleProps::default()
            },
        }],
        style: StyleProps::default(),
    };

    // Test 2: height: 10 (as number)
    let root2 = Widget::Screen {
        background: None,
        children: vec![Widget::VStack {
            spacing: 0,
            children: vec![Widget::Fill {
                children: vec![],
                style: StyleProps::default(),
            }],
            style: StyleProps {
                height: Some(xulo_ui::Dimension::Pixels(10)),
                ..StyleProps::default()
            },
        }],
        style: StyleProps::default(),
    };

    let surface = Size { width: 40, height: 20 };
    let metrics = TestMetrics;

    println!("=== Test 1: height=full ===");
    let tf = TaffyLayout;
    let placed = tf.layout(&root1, surface, &metrics);
    print_placed(&placed, 0);

    println!("\n=== Test 2: height=10 ===");
    let placed = tf.layout(&root2, surface, &metrics);
    print_placed(&placed, 0);
}

fn print_placed(p: &xulo_ui::Placed<'_>, indent: usize) {
    let pad = "  ".repeat(indent);
    let name = match &p.widget {
        Widget::Screen { .. } => "Screen",
        Widget::VStack { .. } => "VStack",
        Widget::HStack { .. } => "HStack",
        Widget::Fill { .. } => "Fill",
        Widget::Text { text, .. } => {
            println!("{}Text \"{}\" @ ({}, {}) {}×{}", pad, text, p.rect.x, p.rect.y, p.rect.width, p.rect.height);
            return;
        }
        _ => "Other",
    };
    println!("{}{} @ ({}, {}) {}×{}", pad, name, p.rect.x, p.rect.y, p.rect.width, p.rect.height);
    for child in &p.children {
        print_placed(child, indent + 1);
    }
}
