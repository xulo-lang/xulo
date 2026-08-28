//! Host-side tests of the wasm engine's pure core (`layout_tree`, `hit_index`).

use xulo_ui::{StyleProps, PaintOp, Widget};
use xulo_ui_wasm::{hit_index, layout_tree};

#[test]
fn lays_out_a_stack_and_collects_buttons() {
    let tree = Widget::VStack {
        spacing: 1,
        children: vec![
            Widget::Text {
                text: "Count: 0".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Button { label: "+".into(), style: StyleProps::default() },
            Widget::Button { label: "-".into(), style: StyleProps::default() },
        ],
        style: StyleProps::default(),
    };
    // Pixel surface: 40 cols x 12 rows of 8x16 cells.
    let (ops, buttons, _inputs) = layout_tree(&tree, 320, 192);
    // Screen-less root: first op draws the text; buttons appear as border ops.
    assert!(ops
        .iter()
        .any(|op| matches!(op, PaintOp::DrawText { text, .. } if *text == "Count: 0")));
    assert_eq!(buttons.len(), 2);
    // Text at y=0 (16px tall); spacing is 1px; button 1 at y = 17.
    let b1 = buttons[0];
    assert_eq!(b1.y, 17);
    assert_eq!(b1.width, 10); // "+" (8px) + 2px padding
                              // Button height = 16 + 2*1px padding = 18; button 2 after 1px spacing.
    let b2 = buttons[1];
    assert_eq!(b2.y, 36);
    assert_eq!(b2.x, 0);
}

#[test]
fn hit_test_finds_buttons_by_position() {
    let tree = Widget::HStack {
        spacing: 2,
        children: vec![
            Widget::Button { label: "+".into(), style: StyleProps::default() },
            Widget::Button { label: "-".into(), style: StyleProps::default() },
        ],
        style: StyleProps::default(),
    };
    let (_, buttons, _) = layout_tree(&tree, 320, 192);
    // "+" at x=0 (w=10), "-" at x=10+2=12.
    assert_eq!(buttons.len(), 2);
    assert_eq!(hit_index(&buttons, 5.0, 16.0), 0);
    assert_eq!(hit_index(&buttons, 13.0, 16.0), 1);
    assert_eq!(hit_index(&buttons, 9.0, 16.0), 0); // inside "+" before spacing
    assert_eq!(hit_index(&buttons, 11.0, 16.0), -1); // the spacing gap
    assert_eq!(hit_index(&buttons, 300.0, 300.0), -1); // off-buttons
}

#[test]
fn widget_tree_round_trips_through_json() {
    // The native side serializes the tree, the wasm side parses it.
    let tree = Widget::VStack {
        spacing: 8,
        children: vec![
            Widget::Text {
                text: "hi".into(),
                color: None,
                style: StyleProps::default(),
            },
            Widget::Button { label: "+".into(), style: StyleProps::default() },
        ],
        style: StyleProps::default(),
    };
    let json = serde_json::to_string(&tree).unwrap();
    let back: Widget = serde_json::from_str(&json).unwrap();
    assert_eq!(tree, back);
}
