//! Convert the interpreter's render tree (opaque `{ name, props }` values)
//! into the typed [`Widget`] tree `xulo-ui` lays out. When an interpreter is
//! provided, `Button.onClick` closures are extracted into invokable callbacks
//! so interactive backends can trigger them.

use std::rc::Rc;

use xulo_runtime::interpreter::Interpreter;
use xulo_runtime::value::Value;
use xulo_ui::{Color, Widget};

/// A UI callback (a `Button.onClick` closure wired to the interpreter).
pub type ButtonCallback = Box<dyn Fn()>;

/// Convert a render value into a widget, dropping any click callbacks.
/// Strings become text; objects with a `name`/`props` shape are matched by
/// name; anything else becomes an unknown (labeled) widget.
pub fn widget_from_value(value: &Value) -> Widget {
    widget_with(value, None, &mut Vec::new())
}

/// Convert a render value into a widget tree plus its click callbacks, in the
/// same order the `Button` widgets appear in the tree (pre-order). The
/// callbacks close over the interpreter, so invoking one mutates the `@State`
/// cells the component declared.
pub fn widget_tree_with_callbacks(
    value: &Value,
    interp: &Rc<Interpreter>,
) -> (Widget, Vec<ButtonCallback>) {
    let mut callbacks = Vec::new();
    let widget = widget_with(value, Some(interp), &mut callbacks);
    (widget, callbacks)
}

fn widget_with(
    value: &Value,
    interp: Option<&Rc<Interpreter>>,
    callbacks: &mut Vec<ButtonCallback>,
) -> Widget {
    match value {
        Value::String(s) => Widget::Text {
            text: s.to_string(),
            color: None,
        },
        Value::Object(fields) => {
            let get = |fields: &[(String, Value)], key: &str| -> Option<Value> {
                fields
                    .iter()
                    .find(|(k, _)| k == key)
                    .map(|(_, v)| v.clone())
            };
            let fields = fields.borrow();
            let name = match get(&fields, "name") {
                Some(Value::String(n)) => n.to_string(),
                _ => return Widget::Unknown { kind: "?".into() },
            };
            let props = match get(&fields, "props") {
                Some(Value::Object(p)) => p,
                _ => return Widget::Unknown { kind: name },
            };
            let props = props.borrow();
            match name.as_str() {
                "Screen" => {
                    let background = get(&props, "backgroundColor")
                        .and_then(|v| string_of(&v))
                        .and_then(|s| Color::parse_hex(&s));
                    Widget::Screen {
                        background,
                        children: children_with(get(&props, "children"), interp, callbacks),
                    }
                }
                "VStack" => Widget::VStack {
                    spacing: spacing_of(get(&props, "spacing")),
                    children: children_with(get(&props, "children"), interp, callbacks),
                },
                "HStack" => Widget::HStack {
                    spacing: spacing_of(get(&props, "spacing")),
                    children: children_with(get(&props, "children"), interp, callbacks),
                },
                "Text" => {
                    let text = get(&props, "0")
                        .or_else(|| get(&props, "text"))
                        .and_then(|v| string_of(&v))
                        .unwrap_or_default();
                    let color = get(&props, "color")
                        .and_then(|v| string_of(&v))
                        .and_then(|s| Color::parse_hex(&s));
                    Widget::Text { text, color }
                }
                "Button" => {
                    if let Some(interp) = interp {
                        let on_click = get(&props, "onClick");
                        if let Some(on_click) = on_click {
                            if matches!(on_click, Value::Function(_) | Value::Native(_)) {
                                let interp = interp.clone();
                                callbacks.push(Box::new(move || {
                                    let _ = interp.invoke(&on_click);
                                }));
                            }
                        }
                    }
                    Widget::Button {
                        label: button_label(get(&props, "0"), &props, interp, callbacks),
                    }
                }
                "Input" => Widget::Input {
                    value: input_value(get(&props, "value")),
                },
                other => Widget::Unknown {
                    kind: other.to_string(),
                },
            }
        }
        other => Widget::Unknown {
            kind: other.kind_name(),
        },
    }
}

/// Extract `children` from a value: `null`/absent → empty, a scalar → one
/// child, a list → flattened recursively (list rendering / `for` produce
/// nested arrays).
fn children_with(
    value: Option<Value>,
    interp: Option<&Rc<Interpreter>>,
    callbacks: &mut Vec<ButtonCallback>,
) -> Vec<Widget> {
    let mut out = Vec::new();
    collect_children(value, interp, callbacks, &mut out);
    out
}

fn collect_children(
    value: Option<Value>,
    interp: Option<&Rc<Interpreter>>,
    callbacks: &mut Vec<ButtonCallback>,
    out: &mut Vec<Widget>,
) {
    match value {
        None | Some(Value::Null) => {}
        Some(Value::List(list)) => {
            for item in list.borrow().iter() {
                collect_children(Some(item.clone()), interp, callbacks, out);
            }
        }
        Some(Value::Signal(cell)) => {
            collect_children(Some(cell.borrow().clone()), interp, callbacks, out);
        }
        Some(other) => out.push(widget_with(&other, interp, callbacks)),
    }
}

/// A button's label comes from its children (a `Text`), or a `"0"` prop.
fn button_label(
    first: Option<Value>,
    props: &[(String, Value)],
    interp: Option<&Rc<Interpreter>>,
    callbacks: &mut Vec<ButtonCallback>,
) -> String {
    if let Some(value) = first.and_then(|v| string_of(&v)) {
        return value;
    }
    children_with(
        props
            .iter()
            .find(|(k, _)| k == "children")
            .map(|(_, v)| v.clone()),
        interp,
        callbacks,
    )
    .into_iter()
    .find_map(|child| match child {
        Widget::Text { text, .. } => Some(text),
        _ => None,
    })
    .unwrap_or_default()
}

/// An input's value prop may be a raw string or a `$binding` object
/// (`{ value, onChange }`).
fn input_value(value: Option<Value>) -> String {
    match value {
        Some(Value::Object(fields)) => {
            let fields = fields.borrow();
            fields
                .iter()
                .find(|(k, _)| k == "value")
                .and_then(|(_, v)| string_of(v))
                .unwrap_or_default()
        }
        Some(other) => string_of(&other).unwrap_or_default(),
        None => String::new(),
    }
}

fn spacing_of(value: Option<Value>) -> u32 {
    match value {
        Some(Value::Number(n)) => n.max(0.0) as u32,
        _ => 0,
    }
}

fn string_of(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.to_string()),
        Value::Number(n) => Some(n.to_string()),
        Value::Boolean(b) => Some(b.to_string()),
        Value::Signal(cell) => string_of(&cell.borrow()),
        _ => None,
    }
}
