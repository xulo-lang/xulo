//! Component conversion: unified entry point for creating widgets from
//! `(name, props)` pairs. This module decouples the framework layer from
//! the internal `Widget` structure — the framework calls [`from_props`]
//! and gets back a ready-to-render `Widget`.

use crate::{Alignment, Color, FontWeight, Justify, StyleProps, Widget};

/// A simple key-value property bag. Framework converts interpreter values
/// into this type, avoiding a direct dependency on `xulo-runtime`.
pub struct Props {
    entries: Vec<(String, PropValue)>,
}

impl Default for Props {
    fn default() -> Self {
        Self::new()
    }
}

/// A single property value. Only the variants needed for UI components
/// are represented — no interpreter-specific types.
pub enum PropValue {
    String(String),
    Number(f64),
    Boolean(bool),
    Children(Vec<Widget>),
}

/// A UI callback extracted during widget conversion (e.g. button click,
/// input change). The framework fills these in; renderers invoke them
/// on user interaction.
pub struct UiCallback {
    pub on_click: Option<Box<dyn Fn()>>,
    pub on_change: Option<Box<dyn Fn(String)>>,
}

// ── Props API ───────────────────────────────────────────────────────────────

impl Props {
    /// Create an empty `Props`.
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    /// Add a string property.
    pub fn string(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.entries.push((key.into(), PropValue::String(value.into())));
        self
    }

    /// Add a numeric property.
    pub fn number(mut self, key: impl Into<String>, value: f64) -> Self {
        self.entries.push((key.into(), PropValue::Number(value)));
        self
    }

    /// Add a boolean property.
    pub fn boolean(mut self, key: impl Into<String>, value: bool) -> Self {
        self.entries.push((key.into(), PropValue::Boolean(value)));
        self
    }

    /// Add a children property.
    pub fn children(mut self, value: Vec<Widget>) -> Self {
        self.entries.push(("children".into(), PropValue::Children(value)));
        self
    }

    /// Get a string property by key.
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.entries.iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| match v {
                PropValue::String(s) => Some(s.as_str()),
                _ => None,
            })
    }

    /// Get a numeric property by key.
    pub fn get_number(&self, key: &str) -> Option<f64> {
        self.entries.iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| match v {
                PropValue::Number(n) => Some(*n),
                _ => None,
            })
    }

    /// Get a boolean property by key.
    pub fn get_boolean(&self, key: &str) -> Option<bool> {
        self.entries.iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| match v {
                PropValue::Boolean(b) => Some(*b),
                _ => None,
            })
    }

    /// Get the children property.
    pub fn get_children(&self) -> &[Widget] {
        self.entries.iter()
            .find(|(k, _)| k == "children")
            .and_then(|(_, v)| match v {
                PropValue::Children(c) => Some(c.as_slice()),
                _ => None,
            })
            .unwrap_or(&[])
    }

    /// Extract a [`StyleProps`] from the style-related entries in this bag.
    pub fn get_style(&self) -> StyleProps {
        let color = self.get_string("color").and_then(Color::parse_hex);
        let background_color = self.get_string("backgroundColor").and_then(Color::parse_hex);
        let border_color = self.get_string("borderColor").and_then(Color::parse_hex);
        let font_size = self.get_number("fontSize").map(|n| n.max(0.0) as u32);
        let font_weight = self.get_string("fontWeight").map(|s| match s {
            "bold" => FontWeight::Bold,
            _ => FontWeight::Normal,
        });
        let padding = self.get_number("padding").map(|n| n.max(0.0) as u32);
        let margin = self.get_number("margin").map(|n| n.max(0.0) as u32);
        let width = self.get_number("width").map(|n| n.max(0.0) as u32);
        let height = self.get_number("height").map(|n| n.max(0.0) as u32);
        let border_radius = self.get_number("borderRadius").map(|n| n.max(0.0) as u32);
        let opacity = self.get_number("opacity").map(|n| (n as f32).clamp(0.0, 1.0));
        let alignment = self.get_string("alignment").map(|s| match s {
            "center" => Alignment::Center,
            "end" => Alignment::End,
            _ => Alignment::Start,
        });
        let justify = self.get_string("justify").map(|s| match s {
            "center" => Justify::Center,
            "end" => Justify::End,
            "space-between" => Justify::SpaceBetween,
            "space-around" => Justify::SpaceAround,
            "space-evenly" => Justify::SpaceEvenly,
            _ => Justify::Start,
        });
        StyleProps {
            color,
            background_color,
            border_color,
            font_size,
            font_weight,
            padding,
            margin,
            width,
            height,
            border_radius,
            opacity,
            alignment,
            justify,
        }
    }
}

// ── Unified entry point ─────────────────────────────────────────────────────

/// Convert a component `name` and its `props` into a `Widget`.
///
/// This is the single function the framework layer needs to call. It handles
/// all built-in components (`Text`, `Button`, `Input`, `VStack`, `HStack`,
/// `Screen`) and returns `Widget::Unknown` for unrecognized names.
///
/// # Callbacks
///
/// Interactive components (`Button`, `Input`) may produce callbacks. The
/// caller must provide a mutable `Vec<UiCallback>` to collect them — the
/// renderer later maps screen coordinates back to these callbacks.
pub fn from_props(
    name: &str,
    props: &Props,
    callbacks: &mut Vec<UiCallback>,
) -> Widget {
    let style = props.get_style();
    match name {
        "Text" => props_to_text(props, style),
        "Button" => props_to_button(props, callbacks, style),
        "Input" => props_to_input(props, callbacks, style),
        "VStack" => props_to_vstack(props, callbacks, style),
        "HStack" => props_to_hstack(props, callbacks, style),
        "Screen" => props_to_screen(props, callbacks, style),
        other => Widget::Unknown { kind: other.to_string(), style },
    }
}

// ── Built-in component factories ────────────────────────────────────────────

fn props_to_text(props: &Props, style: StyleProps) -> Widget {
    let text = props.get_string("0")
        .or_else(|| props.get_string("text"))
        .unwrap_or_default()
        .to_string();
    // style.color is already populated from the "color" prop by get_style().
    Widget::Text { text, color: style.color, style }
}

fn props_to_button(props: &Props, callbacks: &mut Vec<UiCallback>, style: StyleProps) -> Widget {
    let on_click: Option<Box<dyn Fn()>> = None;
    if on_click.is_some() {
        callbacks.push(UiCallback {
            on_click,
            on_change: None,
        });
    }
    let label = props.get_string("0")
        .unwrap_or_default()
        .to_string();
    Widget::Button { label, style }
}

fn props_to_input(props: &Props, callbacks: &mut Vec<UiCallback>, style: StyleProps) -> Widget {
    let value = props.get_string("value")
        .unwrap_or_default()
        .to_string();
    // Explicit "width" prop on Input is merged into style.width.
    let width = props.get_number("width")
        .map(|n| n.max(0.0) as u32)
        .or(style.width);
    let placeholder = props.get_string("placeholder")
        .unwrap_or_default()
        .to_string();
    let on_change: Option<Box<dyn Fn(String)>> = None;
    if on_change.is_some() {
        callbacks.push(UiCallback {
            on_click: None,
            on_change,
        });
    }
    Widget::Input { value, width, placeholder, style }
}

fn props_to_vstack(props: &Props, callbacks: &mut Vec<UiCallback>, style: StyleProps) -> Widget {
    let spacing = props.get_number("spacing")
        .map(|n| n.max(0.0) as u32)
        .unwrap_or(0);
    let children = props.get_children().to_vec();
    let _ = callbacks;
    Widget::VStack { spacing, children, style }
}

fn props_to_hstack(props: &Props, callbacks: &mut Vec<UiCallback>, style: StyleProps) -> Widget {
    let spacing = props.get_number("spacing")
        .map(|n| n.max(0.0) as u32)
        .unwrap_or(0);
    let children = props.get_children().to_vec();
    let _ = callbacks;
    Widget::HStack { spacing, children, style }
}

fn props_to_screen(props: &Props, callbacks: &mut Vec<UiCallback>, style: StyleProps) -> Widget {
    // style.background_color is already populated from "backgroundColor" by get_style().
    let background = style.background_color;
    let children = props.get_children().to_vec();
    let _ = callbacks;
    Widget::Screen { background, children, style }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_with_string_prop() {
        let props = Props::new().string("0", "hello");
        let widget = from_props("Text", &props, &mut Vec::new());
        assert_eq!(widget, Widget::Text { text: "hello".into(), color: None, style: StyleProps::default() });
    }

    #[test]
    fn text_with_color() {
        let props = Props::new()
            .string("0", "error")
            .string("color", "#ff0000");
        let widget = from_props("Text", &props, &mut Vec::new());
        assert_eq!(
            widget,
            Widget::Text { text: "error".into(), color: Some(Color { r: 255, g: 0, b: 0 }), style: StyleProps { color: Some(Color { r: 255, g: 0, b: 0 }), ..StyleProps::default() } }
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
    fn unknown_component() {
        let props = Props::new();
        let widget = from_props("MyCustomWidget", &props, &mut Vec::new());
        assert_eq!(widget, Widget::Unknown { kind: "MyCustomWidget".into(), style: StyleProps::default() });
    }

    #[test]
    fn text_with_style_props() {
        let props = Props::new()
            .string("0", "styled")
            .string("color", "#00ff00")
            .number("fontSize", 16.0)
            .string("fontWeight", "bold");
        let widget = from_props("Text", &props, &mut Vec::new());
        match widget {
            Widget::Text { text, color, style } => {
                assert_eq!(text, "styled");
                assert_eq!(color, Some(Color { r: 0, g: 255, b: 0 }));
                assert_eq!(style.font_size, Some(16));
                assert_eq!(style.font_weight, Some(FontWeight::Bold));
            }
            _ => panic!("expected Text widget"),
        }
    }

    #[test]
    fn button_with_style() {
        let props = Props::new()
            .string("0", "Click me")
            .string("backgroundColor", "#4a90d9")
            .string("color", "#ffffff")
            .string("borderColor", "#333333")
            .number("borderRadius", 4.0)
            .number("padding", 8.0);
        let widget = from_props("Button", &props, &mut Vec::new());
        match widget {
            Widget::Button { label, style } => {
                assert_eq!(label, "Click me");
                assert_eq!(style.background_color, Some(Color { r: 74, g: 144, b: 217 }));
                assert_eq!(style.color, Some(Color { r: 255, g: 255, b: 255 }));
                assert_eq!(style.border_color, Some(Color { r: 51, g: 51, b: 51 }));
                assert_eq!(style.border_radius, Some(4));
                assert_eq!(style.padding, Some(8));
            }
            _ => panic!("expected Button widget"),
        }
    }

    #[test]
    fn input_with_style() {
        let props = Props::new()
            .string("value", "hello")
            .number("width", 300.0)
            .string("backgroundColor", "#1a1a1a")
            .string("borderColor", "#444444")
            .number("padding", 4.0);
        let widget = from_props("Input", &props, &mut Vec::new());
        match widget {
            Widget::Input { value, width, placeholder, style } => {
                assert_eq!(value, "hello");
                assert_eq!(width, Some(300));
                assert_eq!(placeholder, "");
                assert_eq!(style.background_color, Some(Color { r: 26, g: 26, b: 26 }));
                assert_eq!(style.border_color, Some(Color { r: 68, g: 68, b: 68 }));
                assert_eq!(style.padding, Some(4));
            }
            _ => panic!("expected Input widget"),
        }
    }

    #[test]
    fn vstack_with_alignment() {
        let props = Props::new()
            .number("spacing", 2.0)
            .string("alignment", "center")
            .number("padding", 8.0);
        let widget = from_props("VStack", &props, &mut Vec::new());
        match widget {
            Widget::VStack { spacing, style, .. } => {
                assert_eq!(spacing, 2);
                assert_eq!(style.alignment, Some(Alignment::Center));
                assert_eq!(style.padding, Some(8));
            }
            _ => panic!("expected VStack widget"),
        }
    }
}
