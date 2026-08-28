//! Component conversion tests: verify `from_props` produces the expected
//! `Widget` values for all built-in components.

use xulo_ui::{from_props, Props, Widget, Color, StyleProps};

#[test]
fn text_from_string_prop() {
    let props = Props::new().string("0", "hello");
    let widget = from_props("Text", &props, &mut Vec::new());
    assert_eq!(widget, Widget::Text { text: "hello".into(), color: None, style: StyleProps::default() });
}

#[test]
fn text_with_explicit_name() {
    let props = Props::new().string("text", "world");
    let widget = from_props("Text", &props, &mut Vec::new());
    assert_eq!(widget, Widget::Text { text: "world".into(), color: None, style: StyleProps::default() });
}

#[test]
fn text_with_color() {
    let props = Props::new()
        .string("0", "error")
        .string("color", "#ff0000");
    let widget = from_props("Text", &props, &mut Vec::new());
    assert_eq!(
        widget,
        Widget::Text {
            text: "error".into(),
            color: Some(Color { r: 255, g: 0, b: 0 }),
            style: StyleProps { color: Some(Color { r: 255, g: 0, b: 0 }), ..StyleProps::default() },
        }
    );
}

#[test]
fn text_with_short_hex_color() {
    let props = Props::new()
        .string("0", "red")
        .string("color", "#f00");
    let widget = from_props("Text", &props, &mut Vec::new());
    assert_eq!(
        widget,
        Widget::Text {
            text: "red".into(),
            color: Some(Color { r: 255, g: 0, b: 0 }),
            style: StyleProps { color: Some(Color { r: 255, g: 0, b: 0 }), ..StyleProps::default() },
        }
    );
}

#[test]
fn text_empty_defaults() {
    let props = Props::new();
    let widget = from_props("Text", &props, &mut Vec::new());
    assert_eq!(widget, Widget::Text { text: String::new(), color: None, style: StyleProps::default() });
}

#[test]
fn button_with_label() {
    let props = Props::new().string("0", "OK");
    let widget = from_props("Button", &props, &mut Vec::new());
    assert_eq!(widget, Widget::Button { label: "OK".into(), style: StyleProps::default() });
}

#[test]
fn button_empty_label() {
    let props = Props::new();
    let widget = from_props("Button", &props, &mut Vec::new());
    assert_eq!(widget, Widget::Button { label: String::new(), style: StyleProps::default() });
}

#[test]
fn input_basic() {
    let props = Props::new()
        .string("value", "hello")
        .number("width", 200.0)
        .string("placeholder", "Enter text");
    let widget = from_props("Input", &props, &mut Vec::new());
    assert_eq!(
        widget,
        Widget::Input {
            value: "hello".into(),
            width: Some(200),
            placeholder: "Enter text".into(),
            style: StyleProps { width: Some(200), ..StyleProps::default() },
        }
    );
}

#[test]
fn input_without_width() {
    let props = Props::new()
        .string("value", "test")
        .string("placeholder", "Enter");
    let widget = from_props("Input", &props, &mut Vec::new());
    assert_eq!(
        widget,
        Widget::Input {
            value: "test".into(),
            width: None,
            placeholder: "Enter".into(),
            style: StyleProps::default(),
        }
    );
}

#[test]
fn input_empty_defaults() {
    let props = Props::new();
    let widget = from_props("Input", &props, &mut Vec::new());
    assert_eq!(
        widget,
        Widget::Input {
            value: String::new(),
            width: None,
            placeholder: String::new(),
            style: StyleProps::default(),
        }
    );
}

#[test]
fn vstack_with_spacing() {
    let props = Props::new()
        .number("spacing", 2.0)
        .children(vec![
            Widget::Text { text: "a".into(), color: None, style: StyleProps::default() },
            Widget::Text { text: "b".into(), color: None, style: StyleProps::default() },
        ]);
    let widget = from_props("VStack", &props, &mut Vec::new());
    assert_eq!(
        widget,
        Widget::VStack {
            spacing: 2,
            children: vec![
                Widget::Text { text: "a".into(), color: None, style: StyleProps::default() },
                Widget::Text { text: "b".into(), color: None, style: StyleProps::default() },
            ],
            style: StyleProps::default(),
        }
    );
}

#[test]
fn vstack_no_spacing() {
    let props = Props::new()
        .children(vec![Widget::Text { text: "x".into(), color: None, style: StyleProps::default() }]);
    let widget = from_props("VStack", &props, &mut Vec::new());
    assert_eq!(
        widget,
        Widget::VStack {
            spacing: 0,
            children: vec![Widget::Text { text: "x".into(), color: None, style: StyleProps::default() }],
            style: StyleProps::default(),
        }
    );
}

#[test]
fn hstack_with_spacing() {
    let props = Props::new()
        .number("spacing", 3.0)
        .children(vec![
            Widget::Text { text: "left".into(), color: None, style: StyleProps::default() },
            Widget::Text { text: "right".into(), color: None, style: StyleProps::default() },
        ]);
    let widget = from_props("HStack", &props, &mut Vec::new());
    assert_eq!(
        widget,
        Widget::HStack {
            spacing: 3,
            children: vec![
                Widget::Text { text: "left".into(), color: None, style: StyleProps::default() },
                Widget::Text { text: "right".into(), color: None, style: StyleProps::default() },
            ],
            style: StyleProps::default(),
        }
    );
}

#[test]
fn screen_with_background() {
    let props = Props::new()
        .string("backgroundColor", "#101010")
        .children(vec![
            Widget::Text { text: "hi".into(), color: None, style: StyleProps::default() },
        ]);
    let widget = from_props("Screen", &props, &mut Vec::new());
    assert_eq!(
        widget,
        Widget::Screen {
            background: Some(Color { r: 16, g: 16, b: 16 }),
            children: vec![Widget::Text { text: "hi".into(), color: None, style: StyleProps::default() }],
            style: StyleProps { background_color: Some(Color { r: 16, g: 16, b: 16 }), ..StyleProps::default() },
        }
    );
}

#[test]
fn screen_without_background() {
    let props = Props::new()
        .children(vec![Widget::Text { text: "test".into(), color: None, style: StyleProps::default() }]);
    let widget = from_props("Screen", &props, &mut Vec::new());
    assert_eq!(
        widget,
        Widget::Screen {
            background: None,
            children: vec![Widget::Text { text: "test".into(), color: None, style: StyleProps::default() }],
            style: StyleProps::default(),
        }
    );
}

#[test]
fn unknown_component() {
    let props = Props::new();
    let widget = from_props("MyCustomWidget", &props, &mut Vec::new());
    assert_eq!(widget, Widget::Unknown { kind: "MyCustomWidget".into(), style: StyleProps::default() });
}

#[test]
fn nested_components() {
    let props = Props::new()
        .number("spacing", 1.0)
        .children(vec![
            Widget::Text { text: "title".into(), color: None, style: StyleProps::default() },
            Widget::HStack {
                spacing: 2,
                children: vec![
                    Widget::Button { label: "OK".into(), style: StyleProps::default() },
                    Widget::Button { label: "Cancel".into(), style: StyleProps::default() },
                ],
                style: StyleProps::default(),
            },
        ]);
    let widget = from_props("VStack", &props, &mut Vec::new());
    assert_eq!(
        widget,
        Widget::VStack {
            spacing: 1,
            children: vec![
                Widget::Text { text: "title".into(), color: None, style: StyleProps::default() },
                Widget::HStack {
                    spacing: 2,
                    children: vec![
                        Widget::Button { label: "OK".into(), style: StyleProps::default() },
                        Widget::Button { label: "Cancel".into(), style: StyleProps::default() },
                    ],
                    style: StyleProps::default(),
                },
            ],
            style: StyleProps::default(),
        }
    );
}

#[test]
fn props_builder_chain() {
    let props = Props::new()
        .string("0", "hello")
        .string("color", "#00ff00")
        .number("width", 100.0)
        .boolean("disabled", false);
    assert_eq!(props.get_string("0"), Some("hello"));
    assert_eq!(props.get_string("color"), Some("#00ff00"));
    assert_eq!(props.get_number("width"), Some(100.0));
    assert_eq!(props.get_boolean("disabled"), Some(false));
    assert_eq!(props.get_string("nonexistent"), None);
}

#[test]
fn props_children() {
    let children = vec![
        Widget::Text { text: "a".into(), color: None, style: StyleProps::default() },
        Widget::Text { text: "b".into(), color: None, style: StyleProps::default() },
    ];
    let props = Props::new().children(children.clone());
    assert_eq!(props.get_children(), &children);
}

#[test]
fn props_empty_children() {
    let props = Props::new();
    assert_eq!(props.get_children(), &[]);
}
