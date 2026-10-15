//! Backend-agnostic widget tree types.
//!
//! A [`Widget`] tree is the typed description of a rendered UI. It says nothing
//! about pixels or characters: backends lay it out against a surface and turn
//! it into [`PaintOp`]s (see `crate::layout` and `crate::painting`).

/// An RGB color. The terminal backend maps it to ANSI true-color escapes;
/// richer backends (webview, later) use it directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
    };
    pub const GRAY: Color = Color {
        r: 128,
        g: 128,
        b: 128,
    };
    pub const DARK: Color = Color {
        r: 40,
        g: 40,
        b: 40,
    };
    pub const LIGHT: Color = Color {
        r: 240,
        g: 240,
        b: 240,
    };
    pub const ACCENT: Color = Color {
        r: 80,
        g: 140,
        b: 240,
    };

    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parse a `#rgb` or `#rrggbb` hex string (the `bg` prop
    /// format). The `#` is optional.
    pub fn parse_hex(s: &str) -> Option<Color> {
        let s = s.strip_prefix('#').unwrap_or(s);
        if s.len() == 3 {
            let (r, g, b) = (s.as_bytes()[0], s.as_bytes()[1], s.as_bytes()[2]);
            Some(Color {
                r: hex_byte(r, r)?,
                g: hex_byte(g, g)?,
                b: hex_byte(b, b)?,
            })
        } else if s.len() == 6 {
            Some(Color {
                r: hex_byte(s.as_bytes()[0], s.as_bytes()[1])?,
                g: hex_byte(s.as_bytes()[2], s.as_bytes()[3])?,
                b: hex_byte(s.as_bytes()[4], s.as_bytes()[5])?,
            })
        } else {
            None
        }
    }
}

fn hex_byte(hi: u8, lo: u8) -> Option<u8> {
    Some(hex_val(hi)? * 16 + hex_val(lo)?)
}

fn hex_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

/// Pixel size of one monospace character cell (the webview/wasm backend lays
/// out in pixels against an 8×16 cell grid).
pub const CELL_W: u32 = 8;
pub const CELL_H: u32 = 16;

/// A 2D surface size in layout units (character cells for the terminal
/// backend, pixels for the webview/wasm backend).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// An axis-aligned rectangle in layout units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub const fn new(x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// `x + width`.
    pub fn right(&self) -> u32 {
        self.x + self.width
    }

    /// `y + height`.
    pub fn bottom(&self) -> u32 {
        self.y + self.height
    }

    /// Whether the point is inside (edges inclusive on the left/top, exclusive
    /// on the right/bottom, matching cell indexing).
    pub fn contains(&self, x: u32, y: u32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The overlap with `other`, or `None` when the two do not intersect.
    pub fn intersect(&self, other: &Rect) -> Option<Rect> {
        let x0 = self.x.max(other.x);
        let y0 = self.y.max(other.y);
        let x1 = self.right().min(other.right());
        let y1 = self.bottom().min(other.bottom());
        if x0 >= x1 || y0 >= y1 {
            None
        } else {
            Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
        }
    }
}

/// Font weight for text rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FontWeight {
    Normal,
    Bold,
}

/// Horizontal / vertical alignment inside a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Alignment {
    Start,
    Center,
    End,
    /// Fill the cross-axis (yoga's `stretch`, the default).
    Stretch,
}

/// How children are distributed along the main axis of a container.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Justify {
    /// Children packed at the start (default).
    Start,
    /// Children centered in the middle.
    Center,
    /// Children packed at the end.
    End,
    /// Equal spacing between children; no spacing at edges.
    SpaceBetween,
    /// Equal spacing between children; half-size spacing at edges.
    SpaceAround,
    /// Equal spacing everywhere (between and at edges).
    SpaceEvenly,
}

/// Scroll direction for [`Widget::ScrollView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Axes {
    /// Scroll vertically (default).
    Vertical,
    /// Scroll horizontally.
    Horizontal,
    /// Scroll in both directions.
    Both,
}

/// A size dimension: either a fixed pixel value or a keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Dimension {
    /// Fixed size in layout units.
    Pixels(u32),
    /// Fill 100% of the parent's available space.
    Full,
}

impl Dimension {
    /// Resolve this dimension to a concrete value. `parent_size` is the
    /// available space from the parent. For `Full` it returns the parent size;
    /// for `Pixels(n)` it returns `n`.
    pub fn resolve(self, parent_size: u32) -> u32 {
        match self {
            Dimension::Pixels(n) => n,
            Dimension::Full => parent_size,
        }
    }
}

/// A bag of optional CSS-like style properties. Every field is `None` by
/// default; the layout / paint engines fall back to theme values or hardcoded
/// defaults when a field is absent.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Default)]
pub struct StyleProps {
    /// Foreground / text color override.
    pub color: Option<Color>,
    /// Background fill color.
    pub background_color: Option<Color>,
    /// Background image URL.
    pub bg_image: Option<String>,
    /// Border color (buttons, inputs, unknowns).
    pub border_color: Option<Color>,
    /// Font size in pixels (currently only 12 is rendered by the canvas backend;
    /// reserved for future use).
    pub font_size: Option<u32>,
    /// Bold / normal weight.
    pub font_weight: Option<FontWeight>,
    /// Inner padding on all four sides (layout units).
    pub padding: Option<u32>,
    /// Outer margin on all four sides (layout units).
    pub margin: Option<u32>,
    /// Explicit width override (layout units). `"full"` fills the parent.
    pub width: Option<Dimension>,
    /// Explicit height override (layout units). `"full"` fills the parent.
    pub height: Option<Dimension>,
    /// Corner radius for border / fill (layout units).
    pub border_radius: Option<u32>,
    /// Opacity multiplier (0.0 – 1.0). `None` means fully opaque.
    pub opacity: Option<f32>,
    /// Child alignment on the **cross** axis (VStack: horizontal, HStack: vertical).
    pub alignment: Option<Alignment>,
    /// Child distribution along the **main** axis (VStack: vertical, HStack: horizontal).
    pub justify: Option<Justify>,

    // ── Flex layout properties (yoga-layout compatible) ────────────────────
    /// Flex grow factor. Children with higher values get more remaining space.
    /// Default is 0 (no grow). Setting to 1.0 is equivalent to `Fill`.
    pub flex_grow: Option<f32>,
    /// Flex shrink factor. Children with higher values shrink more when space
    /// is limited. Default is 1.0.
    pub flex_shrink: Option<f32>,
    /// Fixed basis for flex layout. If set, this is the child's initial main-axis
    /// size before flex grow/shrink is applied. If `None`, the child is measured
    /// first and its intrinsic size is used as the basis.
    pub flex_basis: Option<u32>,
    /// Cross-axis alignment override for this specific child (overrides parent's
    /// `alignment` for this child only).
    pub align_self: Option<Alignment>,

    // ── Min/Max size constraints ──────────────────────────────────────────
    /// Minimum width the widget must occupy (layout units).
    pub min_width: Option<u32>,
    /// Maximum width the widget may occupy (layout units).
    pub max_width: Option<u32>,
    /// Minimum height the widget must occupy (layout units).
    pub min_height: Option<u32>,
    /// Maximum height the widget may occupy (layout units).
    pub max_height: Option<u32>,
}

impl StyleProps {
    /// Returns `true` when a Text component should be treated as "decorated"
    /// (has a minimal box model with padding, background, or explicit size).
    /// When `false`, Text is pure text with no box-model overhead.
    pub fn is_text_decorated(&self) -> bool {
        self.background_color.is_some()
            || self.padding.is_some()
            || self.width.is_some()
            || self.height.is_some()
    }

    /// Returns `true` when every field is `None`.
    pub fn is_empty(&self) -> bool {
        self.color.is_none()
            && self.background_color.is_none()
            && self.bg_image.is_none()
            && self.border_color.is_none()
            && self.font_size.is_none()
            && self.font_weight.is_none()
            && self.padding.is_none()
            && self.margin.is_none()
            && self.width.is_none()
            && self.height.is_none()
            && self.border_radius.is_none()
            && self.opacity.is_none()
            && self.alignment.is_none()
            && self.justify.is_none()
            && self.flex_grow.is_none()
            && self.flex_shrink.is_none()
            && self.flex_basis.is_none()
            && self.align_self.is_none()
            && self.min_width.is_none()
            && self.max_width.is_none()
            && self.min_height.is_none()
            && self.max_height.is_none()
    }

    /// Returns `(horizontal_padding, vertical_padding)`. If a custom `padding`
    /// is set it is used for both axes; otherwise the default button/input
    /// padding constants are returned.
    pub fn effective_padding(&self) -> (u32, u32) {
        match self.padding {
            Some(pad) if pad > 0 => (pad, pad),
            _ => (super::layout::PAD_X, super::layout::PAD_Y),
        }
    }

    /// Resolve width to a concrete `u32`. If width is `None` or `Full`,
    /// returns `parent_size`. If `Pixels(n)`, returns `n`.
    pub fn resolved_width(&self, parent_size: u32) -> u32 {
        match self.width {
            Some(Dimension::Pixels(n)) => n,
            _ => parent_size,
        }
    }

    /// Resolve height to a concrete `u32`. If height is `None` or `Full`,
    /// returns `parent_size`. If `Pixels(n)`, returns `n`.
    pub fn resolved_height(&self, parent_size: u32) -> u32 {
        match self.height {
            Some(Dimension::Pixels(n)) => n,
            _ => parent_size,
        }
    }
}

/// A node in the widget tree. The tree is backend-agnostic: layout assigns each
/// node a rectangle, painting turns it into [`PaintOp`]s.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Widget {
    /// Root node: fills the whole surface, optionally with a background color.
    Screen {
        background: Option<Color>,
        children: Vec<Widget>,
        style: StyleProps,
    },
    /// Children stacked vertically, separated by `spacing`; children fill the
    /// stack's width (text longer than the width is truncated).
    VStack {
        spacing: u32,
        children: Vec<Widget>,
        style: StyleProps,
    },
    /// Children laid side by side, separated by `spacing`; each child keeps its
    /// intrinsic width.
    HStack {
        spacing: u32,
        children: Vec<Widget>,
        style: StyleProps,
    },
    /// A container that has zero intrinsic size and expands to fill remaining
    /// space in the parent VStack/HStack. Multiple Fills share equally.
    Fill {
        children: Vec<Widget>,
        style: StyleProps,
    },
    /// A single line of text, optionally tinted (falls back to the theme).
    Text {
        text: String,
        color: Option<Color>,
        style: StyleProps,
    },
    /// A tappable boxed label.
    Button { label: String, style: StyleProps },
    /// A single-line text field.
    Input {
        value: String,
        width: Option<u32>,
        placeholder: String,
        style: StyleProps,
    },
    /// A widget the UI layer does not recognize yet; rendered as a labeled box.
    Unknown { kind: String, style: StyleProps },
    /// A scrollable container. Clips children to its bounds and allows
    /// scrolling via wheel/drag. `axes` controls which directions can scroll.
    ScrollView {
        axes: Axes,
        children: Vec<Widget>,
        style: StyleProps,
    },
}

impl Widget {
    /// Returns a reference to the widget's `StyleProps`.
    pub fn style(&self) -> &StyleProps {
        match self {
            Widget::Screen { style, .. }
            | Widget::VStack { style, .. }
            | Widget::HStack { style, .. }
            | Widget::Fill { style, .. }
            | Widget::Text { style, .. }
            | Widget::Button { style, .. }
            | Widget::Input { style, .. }
            | Widget::Unknown { style, .. }
            | Widget::ScrollView { style, .. } => style,
        }
    }
}

// ── Component conversion: Props → Widget ────────────────────────────────────

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
        Self {
            entries: Vec::new(),
        }
    }

    /// Add a string property.
    pub fn string(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.entries
            .push((key.into(), PropValue::String(value.into())));
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
        self.entries
            .push(("children".into(), PropValue::Children(value)));
        self
    }

    /// Get a string property by key.
    pub fn get_string(&self, key: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| match v {
                PropValue::String(s) => Some(s.as_str()),
                _ => None,
            })
    }

    /// Get a numeric property by key.
    pub fn get_number(&self, key: &str) -> Option<f64> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| match v {
                PropValue::Number(n) => Some(*n),
                _ => None,
            })
    }

    /// Get a boolean property by key.
    pub fn get_boolean(&self, key: &str) -> Option<bool> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| match v {
                PropValue::Boolean(b) => Some(*b),
                _ => None,
            })
    }

    /// Get the children property.
    pub fn get_children(&self) -> &[Widget] {
        self.entries
            .iter()
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
        let background_color = self
            .get_string("bg")
            .or_else(|| self.get_string("backgroundColor"))
            .and_then(Color::parse_hex);
        let bg_image = self
            .get_string("bgImage")
            .or_else(|| self.get_string("backgroundImage"))
            .map(|s| s.to_string());
        let border_color = self
            .get_string("border")
            .or_else(|| self.get_string("borderColor"))
            .and_then(Color::parse_hex);
        let font_size = self
            .get_number("size")
            .or_else(|| self.get_number("fontSize"))
            .map(|n| n.max(0.0) as u32);
        let font_weight = self
            .get_string("weight")
            .or_else(|| self.get_string("fontWeight"))
            .map(|s| match s {
                "bold" => FontWeight::Bold,
                _ => FontWeight::Normal,
            });
        let padding = self.get_number("padding").map(|n| n.max(0.0) as u32);
        let margin = self.get_number("margin").map(|n| n.max(0.0) as u32);
        let width = self
            .get_string("width")
            .map(|s| match s {
                "full" => Dimension::Full,
                _ => Dimension::Pixels(s.parse::<u32>().unwrap_or(0)),
            })
            .or_else(|| {
                self.get_number("width")
                    .map(|n| Dimension::Pixels(n.max(0.0) as u32))
            });
        let height = self
            .get_string("height")
            .map(|s| match s {
                "full" => Dimension::Full,
                _ => Dimension::Pixels(s.parse::<u32>().unwrap_or(0)),
            })
            .or_else(|| {
                self.get_number("height")
                    .map(|n| Dimension::Pixels(n.max(0.0) as u32))
            });
        let border_radius = self
            .get_number("radius")
            .or_else(|| self.get_number("borderRadius"))
            .map(|n| n.max(0.0) as u32);
        let opacity = self
            .get_number("opacity")
            .map(|n| (n as f32).clamp(0.0, 1.0));
        let alignment = self.get_string("alignment").map(|s| match s {
            "center" => Alignment::Center,
            "end" => Alignment::End,
            "stretch" => Alignment::Stretch,
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
        let flex_grow = self
            .get_number("flexGrow")
            .or_else(|| self.get_number("flex_grow"))
            .map(|n| n.max(0.0) as f32);
        let flex_shrink = self
            .get_number("flexShrink")
            .or_else(|| self.get_number("flex_shrink"))
            .map(|n| n.max(0.0) as f32);
        let flex_basis = self
            .get_number("flexBasis")
            .or_else(|| self.get_number("flex_basis"))
            .map(|n| n.max(0.0) as u32);
        let align_self = self
            .get_string("alignSelf")
            .or_else(|| self.get_string("align_self"))
            .map(|s| match s {
                "center" => Alignment::Center,
                "end" => Alignment::End,
                "stretch" => Alignment::Stretch,
                _ => Alignment::Start,
            });
        let min_width = self
            .get_number("minWidth")
            .or_else(|| self.get_number("min_width"))
            .map(|n| n.max(0.0) as u32);
        let max_width = self
            .get_number("maxWidth")
            .or_else(|| self.get_number("max_width"))
            .map(|n| n.max(0.0) as u32);
        let min_height = self
            .get_number("minHeight")
            .or_else(|| self.get_number("min_height"))
            .map(|n| n.max(0.0) as u32);
        let max_height = self
            .get_number("maxHeight")
            .or_else(|| self.get_number("max_height"))
            .map(|n| n.max(0.0) as u32);
        StyleProps {
            color,
            background_color,
            bg_image,
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
            flex_grow,
            flex_shrink,
            flex_basis,
            align_self,
            min_width,
            max_width,
            min_height,
            max_height,
        }
    }
}

// ── Unified entry point ─────────────────────────────────────────────────────

/// Convert a component `name` and its `props` into a `Widget`.
///
/// This is the single function the framework layer needs to call. It handles
/// all built-in components (`Text`, `Button`, `Input`, `VStack`, `HStack`,
/// `Fill`, `Screen`) and returns `Widget::Unknown` for unrecognized names.
///
/// # Callbacks
///
/// Interactive components (`Button`, `Input`) may produce callbacks. The
/// caller must provide a mutable `Vec<UiCallback>` to collect them — the
/// renderer later maps screen coordinates back to these callbacks.
pub fn from_props(name: &str, props: &Props, callbacks: &mut Vec<UiCallback>) -> Widget {
    let style = props.get_style();
    match name {
        "Text" => props_to_text(props, style),
        "Button" => props_to_button(props, callbacks, style),
        "Input" => props_to_input(props, callbacks, style),
        "VStack" => props_to_vstack(props, callbacks, style),
        "HStack" => props_to_hstack(props, callbacks, style),
        "Fill" => props_to_fill(props, callbacks, style),
        "ScrollView" => props_to_scrollview(props, callbacks, style),
        "Screen" => props_to_screen(props, callbacks, style),
        other => Widget::Unknown {
            kind: other.to_string(),
            style,
        },
    }
}

// ── Built-in component factories ────────────────────────────────────────────

fn props_to_text(props: &Props, style: StyleProps) -> Widget {
    let text = props
        .get_string("0")
        .or_else(|| props.get_string("text"))
        .unwrap_or_default()
        .to_string();
    Widget::Text {
        text,
        color: style.color,
        style,
    }
}

fn props_to_button(props: &Props, callbacks: &mut Vec<UiCallback>, style: StyleProps) -> Widget {
    let on_click: Option<Box<dyn Fn()>> = None;
    if on_click.is_some() {
        callbacks.push(UiCallback {
            on_click,
            on_change: None,
        });
    }
    let label = props.get_string("0").unwrap_or_default().to_string();
    Widget::Button { label, style }
}

fn props_to_input(props: &Props, callbacks: &mut Vec<UiCallback>, style: StyleProps) -> Widget {
    let value = props.get_string("value").unwrap_or_default().to_string();
    let width = props
        .get_number("width")
        .map(|n| n.max(0.0) as u32)
        .or(style.width.and_then(|d| match d {
            Dimension::Pixels(n) => Some(n),
            _ => None,
        }));
    let placeholder = props
        .get_string("placeholder")
        .unwrap_or_default()
        .to_string();
    let on_change: Option<Box<dyn Fn(String)>> = None;
    if on_change.is_some() {
        callbacks.push(UiCallback {
            on_click: None,
            on_change,
        });
    }
    Widget::Input {
        value,
        width,
        placeholder,
        style,
    }
}

fn props_to_vstack(props: &Props, callbacks: &mut Vec<UiCallback>, style: StyleProps) -> Widget {
    let spacing = props
        .get_number("spacing")
        .map(|n| n.max(0.0) as u32)
        .unwrap_or(0);
    let children = props.get_children().to_vec();
    let _ = callbacks;
    Widget::VStack {
        spacing,
        children,
        style,
    }
}

fn props_to_hstack(props: &Props, callbacks: &mut Vec<UiCallback>, style: StyleProps) -> Widget {
    let spacing = props
        .get_number("spacing")
        .map(|n| n.max(0.0) as u32)
        .unwrap_or(0);
    let children = props.get_children().to_vec();
    let _ = callbacks;
    Widget::HStack {
        spacing,
        children,
        style,
    }
}

fn props_to_fill(props: &Props, callbacks: &mut Vec<UiCallback>, mut style: StyleProps) -> Widget {
    let children = props.get_children().to_vec();
    let _ = callbacks;
    // Fill is a convenience wrapper for flex_grow: 1.0
    if style.flex_grow.is_none() {
        style.flex_grow = Some(1.0);
    }
    Widget::Fill { children, style }
}

fn props_to_scrollview(
    props: &Props,
    callbacks: &mut Vec<UiCallback>,
    style: StyleProps,
) -> Widget {
    let axes = props
        .get_string("axes")
        .map(|s| match s {
            "horizontal" | "Horizontal" => Axes::Horizontal,
            "both" | "Both" => Axes::Both,
            _ => Axes::Vertical,
        })
        .unwrap_or(Axes::Vertical);
    let children = props.get_children().to_vec();
    let _ = callbacks;
    Widget::ScrollView {
        axes,
        children,
        style,
    }
}

fn props_to_screen(props: &Props, callbacks: &mut Vec<UiCallback>, style: StyleProps) -> Widget {
    let background = style.background_color;
    let children = props.get_children().to_vec();
    let _ = callbacks;
    Widget::Screen {
        background,
        children,
        style,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_with_string_prop() {
        let props = Props::new().string("0", "hello");
        let widget = from_props("Text", &props, &mut Vec::new());
        assert_eq!(
            widget,
            Widget::Text {
                text: "hello".into(),
                color: None,
                style: StyleProps::default()
            }
        );
    }

    #[test]
    fn text_with_color() {
        let props = Props::new().string("0", "error").string("color", "#ff0000");
        let widget = from_props("Text", &props, &mut Vec::new());
        assert_eq!(
            widget,
            Widget::Text {
                text: "error".into(),
                color: Some(Color { r: 255, g: 0, b: 0 }),
                style: StyleProps {
                    color: Some(Color { r: 255, g: 0, b: 0 }),
                    ..StyleProps::default()
                }
            }
        );
    }

    #[test]
    fn vstack_with_spacing() {
        let props = Props::new().number("spacing", 2.0).children(vec![
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
        ]);
        let widget = from_props("VStack", &props, &mut Vec::new());
        assert_eq!(
            widget,
            Widget::VStack {
                spacing: 2,
                children: vec![
                    Widget::Text {
                        text: "a".into(),
                        color: None,
                        style: StyleProps::default()
                    },
                    Widget::Text {
                        text: "b".into(),
                        color: None,
                        style: StyleProps::default()
                    },
                ],
                style: StyleProps::default(),
            }
        );
    }

    #[test]
    fn screen_with_background() {
        let props = Props::new()
            .string("backgroundColor", "#101010")
            .children(vec![Widget::Text {
                text: "hi".into(),
                color: None,
                style: StyleProps::default(),
            }]);
        let widget = from_props("Screen", &props, &mut Vec::new());
        assert_eq!(
            widget,
            Widget::Screen {
                background: Some(Color {
                    r: 16,
                    g: 16,
                    b: 16
                }),
                children: vec![Widget::Text {
                    text: "hi".into(),
                    color: None,
                    style: StyleProps::default()
                }],
                style: StyleProps {
                    background_color: Some(Color {
                        r: 16,
                        g: 16,
                        b: 16
                    }),
                    ..StyleProps::default()
                },
            }
        );
    }

    #[test]
    fn unknown_component() {
        let props = Props::new();
        let widget = from_props("MyCustomWidget", &props, &mut Vec::new());
        assert_eq!(
            widget,
            Widget::Unknown {
                kind: "MyCustomWidget".into(),
                style: StyleProps::default()
            }
        );
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
                assert_eq!(
                    style.background_color,
                    Some(Color {
                        r: 74,
                        g: 144,
                        b: 217
                    })
                );
                assert_eq!(
                    style.color,
                    Some(Color {
                        r: 255,
                        g: 255,
                        b: 255
                    })
                );
                assert_eq!(
                    style.border_color,
                    Some(Color {
                        r: 51,
                        g: 51,
                        b: 51
                    })
                );
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
            Widget::Input {
                value,
                width,
                placeholder,
                style,
            } => {
                assert_eq!(value, "hello");
                assert_eq!(width, Some(300));
                assert_eq!(placeholder, "");
                assert_eq!(
                    style.background_color,
                    Some(Color {
                        r: 26,
                        g: 26,
                        b: 26
                    })
                );
                assert_eq!(
                    style.border_color,
                    Some(Color {
                        r: 68,
                        g: 68,
                        b: 68
                    })
                );
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
