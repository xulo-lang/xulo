//! Convert the interpreter's render tree (opaque `{ name, props }` values)
//! into the typed [`Widget`] tree `xulo-ui` lays out. When an interpreter is
//! provided, `Button.onClick` closures are extracted into invokable callbacks
//! so interactive backends can trigger them.

use std::rc::Rc;

use xulo_runtime::interpreter::Interpreter;
use xulo_runtime::value::Value;
use xulo_ui::{Widget, from_props, Props, UiCallback, StyleProps};

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
) -> (Widget, Vec<UiCallback>) {
    let mut callbacks = Vec::new();
    let widget = widget_with(value, Some(interp), &mut callbacks);
    (widget, callbacks)
}

fn widget_with(
    value: &Value,
    interp: Option<&Rc<Interpreter>>,
    callbacks: &mut Vec<UiCallback>,
) -> Widget {
    match value {
        Value::String(s) => Widget::Text {
            text: s.to_string(),
            color: None,
            style: StyleProps::default(),
        },
        Value::Object(fields) => {
            let fields = fields.borrow();
            let name = match get(&fields, "name") {
                Some(Value::String(n)) => n.to_string(),
                _ => return Widget::Unknown { kind: "?".into(), style: StyleProps::default() },
            };
            let props_obj = match get(&fields, "props") {
                Some(Value::Object(p)) => p,
                _ => return Widget::Unknown { kind: name, style: StyleProps::default() },
            };
            let props_obj = props_obj.borrow();

            // Convert interpreter props to xulo-ui Props
            let mut props = Props::new();

            // Extract common props based on component name
            match name.as_str() {
                "Screen" => {
                    if let Some(bg) = get(&props_obj, "backgroundColor")
                        .and_then(|v| string_of(v))
                    {
                        props = props.string("backgroundColor", bg);
                    }
                    let children = children_with(get(&props_obj, "children"), interp, callbacks);
                    props = props.children(children);
                }
                "VStack" | "HStack" => {
                    if let Some(Value::Number(spacing)) = get(&props_obj, "spacing") {
                        props = props.number("spacing", *spacing);
                    }
                    let children = children_with(get(&props_obj, "children"), interp, callbacks);
                    props = props.children(children);
                }
                "Text" => {
                    let text = get(&props_obj, "0")
                        .or_else(|| get(&props_obj, "text"))
                        .and_then(|v| string_of(v))
                        .unwrap_or_default();
                    props = props.string("0", text);
                    if let Some(color) = get(&props_obj, "color")
                        .and_then(|v| string_of(v))
                    {
                        props = props.string("color", color);
                    }
                }
                "Button" => {
                    // Extract onClick callback before converting props
                    if let Some(interp) = interp {
                        let on_click = get(&props_obj, "onClick");
                        if let Some(on_click) = on_click {
                            if matches!(on_click, Value::Function(_) | Value::Native(_)) {
                                let interp = interp.clone();
                                let on_click = on_click.clone();
                                callbacks.push(UiCallback {
                                    on_click: Some(Box::new(move || {
                                        let _ = interp.invoke(&on_click);
                                    })),
                                    on_change: None,
                                });
                            }
                        }
                    }
                    // Extract label from "0" prop or children
                    let label = button_label(get(&props_obj, "0"), &props_obj, interp, callbacks);
                    props = props.string("0", label);
                }
                "Input" => {
                    let value = input_value(get(&props_obj, "value"));
                    props = props.string("value", value);
                    if let Some(Value::Number(width)) = get(&props_obj, "width") {
                        props = props.number("width", *width);
                    }
                    if let Some(placeholder) = get(&props_obj, "placeholder")
                        .and_then(|v| string_of(v))
                    {
                        props = props.string("placeholder", placeholder);
                    }
                    // Extract onChange callback from $binding
                    if let Some(interp) = interp {
                        let on_change = get(&props_obj, "value")
                            .and_then(|v| extract_on_change(v));
                        if let Some(on_change) = on_change {
                            let interp = interp.clone();
                            callbacks.push(UiCallback {
                                on_click: None,
                                on_change: Some(Box::new(move |new_val: String| {
                                    let _ = interp.call_value_with_values(
                                        &on_change,
                                        &[Value::String(Rc::from(new_val.as_str()))],
                                    );
                                })),
                            });
                        }
                    }
                }
                _ => {
                    // Unknown component — pass through all props as strings
                    for (key, value) in props_obj.iter() {
                        if let Some(s) = string_of(value) {
                            props = props.string(key, s);
                        }
                    }
                }
            }

            // Extract style props (color, backgroundColor, padding, margin, etc.)
            props = extract_style(&props_obj, props);

            // Use the unified entry point
            from_props(&name, &props, callbacks)
        }
        other => Widget::Unknown {
            kind: other.kind_name(),
            style: StyleProps::default(),
        },
    }
}

/// Extract `children` from a value: `null`/absent → empty, a scalar → one
/// child, a list → flattened recursively (list rendering / `for` produce
/// nested arrays).
fn children_with(
    value: Option<&Value>,
    interp: Option<&Rc<Interpreter>>,
    callbacks: &mut Vec<UiCallback>,
) -> Vec<Widget> {
    let mut out = Vec::new();
    collect_children(value, interp, callbacks, &mut out);
    out
}

fn collect_children<'a>(
    value: Option<&'a Value>,
    interp: Option<&Rc<Interpreter>>,
    callbacks: &mut Vec<UiCallback>,
    out: &mut Vec<Widget>,
) {
    match value {
        None | Some(Value::Null) => {}
        Some(Value::List(list)) => {
            for item in list.borrow().iter() {
                collect_children(Some(item), interp, callbacks, out);
            }
        }
        Some(Value::Signal(cell)) => {
            collect_children(Some(&cell.borrow()), interp, callbacks, out);
        }
        Some(other) => out.push(widget_with(&other, interp, callbacks)),
    }
}

/// A button's label comes from its children (a `Text`), or a `"0"` prop.
fn button_label(
    first: Option<&Value>,
    props: &[(String, Value)],
    interp: Option<&Rc<Interpreter>>,
    callbacks: &mut Vec<UiCallback>,
) -> String {
    if let Some(value) = first.and_then(|v| string_of(v)) {
        return value;
    }
    children_with(
        props
            .iter()
            .find(|(k, _)| k == "children")
            .map(|(_, v)| v),
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
fn input_value(value: Option<&Value>) -> String {
    match value {
        Some(Value::Object(fields)) => {
            let fields = fields.borrow();
            fields
                .iter()
                .find(|(k, _)| k == "value")
                .and_then(|(_, v)| string_of(v))
                .unwrap_or_default()
        }
        Some(other) => string_of(other).unwrap_or_default(),
        None => String::new(),
    }
}

fn get<'a>(fields: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v)
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

/// Extract the `onChange` closure from a `$binding` value (`{ value, onChange }`).
fn extract_on_change(value: &Value) -> Option<Value> {
    match value {
        Value::Object(fields) => {
            let fields = fields.borrow();
            fields
                .iter()
                .find(|(k, _)| k == "onChange")
                .map(|(_, v)| v.clone())
        }
        _ => None,
    }
}

/// Extract style-related props from interpreter Value objects and add them
/// to the Props bag. Called for every component so `from_props` can build
/// a complete `StyleProps`.
fn extract_style(fields: &[(String, Value)], mut props: Props) -> Props {
    // Color props
    if let Some(s) = get(fields, "color").and_then(|v| string_of(v)) {
        props = props.string("color", s);
    }
    if let Some(s) = get(fields, "backgroundColor").and_then(|v| string_of(v)) {
        props = props.string("backgroundColor", s);
    }
    if let Some(s) = get(fields, "borderColor").and_then(|v| string_of(v)) {
        props = props.string("borderColor", s);
    }
    // Numeric style props
    if let Some(Value::Number(n)) = get(fields, "fontSize") {
        props = props.number("fontSize", *n);
    }
    if let Some(s) = get(fields, "fontWeight").and_then(|v| string_of(v)) {
        props = props.string("fontWeight", s);
    }
    if let Some(Value::Number(n)) = get(fields, "padding") {
        props = props.number("padding", *n);
    }
    if let Some(Value::Number(n)) = get(fields, "margin") {
        props = props.number("margin", *n);
    }
    if let Some(Value::Number(n)) = get(fields, "width") {
        props = props.number("width", *n);
    }
    if let Some(Value::Number(n)) = get(fields, "height") {
        props = props.number("height", *n);
    }
    if let Some(Value::Number(n)) = get(fields, "borderRadius") {
        props = props.number("borderRadius", *n);
    }
    if let Some(Value::Number(n)) = get(fields, "opacity") {
        props = props.number("opacity", *n);
    }
    if let Some(s) = get(fields, "alignment").and_then(|v| string_of(v)) {
        props = props.string("alignment", s);
    }
    if let Some(s) = get(fields, "justify").and_then(|v| string_of(v)) {
        props = props.string("justify", s);
    }
    props
}
