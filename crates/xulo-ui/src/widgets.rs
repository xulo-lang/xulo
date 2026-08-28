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

    /// Parse a `#rgb` or `#rrggbb` hex string (the `backgroundColor` prop
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
    /// Explicit width override (layout units).
    pub width: Option<u32>,
    /// Explicit height override (layout units).
    pub height: Option<u32>,
    /// Corner radius for border / fill (layout units).
    pub border_radius: Option<u32>,
    /// Opacity multiplier (0.0 – 1.0). `None` means fully opaque.
    pub opacity: Option<f32>,
    /// Child alignment inside a container.
    pub alignment: Option<Alignment>,
}

impl StyleProps {
    /// Returns `true` when every field is `None`.
    pub fn is_empty(&self) -> bool {
        self.color.is_none()
            && self.background_color.is_none()
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
    /// A single line of text, optionally tinted (falls back to the theme).
    Text {
        text: String,
        color: Option<Color>,
        style: StyleProps,
    },
    /// A tappable boxed label.
    Button {
        label: String,
        style: StyleProps,
    },
    /// A single-line text field.
    Input {
        value: String,
        width: Option<u32>,
        placeholder: String,
        style: StyleProps,
    },
    /// A widget the UI layer does not recognize yet; rendered as a labeled box.
    Unknown {
        kind: String,
        style: StyleProps,
    },
}
