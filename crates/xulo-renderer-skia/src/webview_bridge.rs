//! Webview bridge — renders widget tree to RGBA bitmap via Skia CPU surface.
//!
//! The bitmap can be embedded in HTML as a data URI or transferred via shared
//! memory for display in a webview.

use crate::skia_context::SkiaRenderContext;

/// Bridge between Skia CPU rendering and webview display.
///
/// Maintains an off-screen Skia surface and provides the rendered RGBA
/// pixel data for embedding in HTML or transfer over IPC.
pub struct SkiaBitmapBridge {
    ctx: SkiaRenderContext,
}

impl SkiaBitmapBridge {
    /// Create a new bridge with the given dimensions.
    pub fn new(width: i32, height: i32) -> Option<Self> {
        let ctx = SkiaRenderContext::new_cpu(width, height)?;
        Some(Self { ctx })
    }

    /// Get a mutable reference to the underlying render context.
    pub fn context_mut(&mut self) -> &mut SkiaRenderContext {
        &mut self.ctx
    }

    /// Get the width of the bitmap.
    pub fn width(&self) -> i32 {
        self.ctx.width()
    }

    /// Get the height of the bitmap.
    pub fn height(&self) -> i32 {
        self.ctx.height()
    }

    /// Read the rendered pixel data as RGBA bytes.
    pub fn read_pixels(&mut self) -> Option<Vec<u8>> {
        self.ctx.read_pixels()
    }

    /// Encode the rendered pixels as a PNG data URI string.
    ///
    /// Returns `None` if encoding fails.
    pub fn to_data_uri(&mut self) -> Option<String> {
        let png_bytes = self.ctx.encode_png()?;
        let b64 = base64_encode(&png_bytes);
        Some(format!("data:image/png;base64,{b64}"))
    }

    /// Generate an HTML `<img>` tag with the rendered bitmap embedded as
    /// a data URI.
    pub fn to_img_tag(&mut self) -> Option<String> {
        let uri = self.to_data_uri()?;
        let w = self.width();
        let h = self.height();
        Some(format!(
            r#"<img src="{uri}" width="{w}" height="{h}" style="image-rendering:pixelated;" />"#
        ))
    }

    /// Generate a complete HTML page with the rendered bitmap displayed.
    ///
    /// The page includes no-op `window.resize()` and `window.redraw()`
    /// handlers so the webview's resize IPC calls don't error.
    pub fn to_html_page(&mut self) -> Option<String> {
        let uri = self.to_data_uri()?;
        let w = self.width();
        let h = self.height();
        Some(format!(
            r#"<!doctype html>
<html>
<head><meta charset="utf-8"><title>Skia Render</title>
<style>html,body{{margin:0;padding:0;background:#000;overflow:hidden;}}</style>
</head>
<body>
<div id="wrap" style="position:relative;display:inline-block;">
<img id="frame" src="{uri}" width="{w}" height="{h}" style="image-rendering:pixelated;" />
</div>
<script>
// No-op handlers for webview IPC resize/redraw calls.
window.resize = function(w, h) {{}};
window.redraw = function(tree) {{}};
// IPC hook — forward click events to native side.
function ipc(msg) {{ window.ipc.postMessage(msg); }}
</script>
</body>
</html>"#
        ))
    }

    /// Resize the surface to the given dimensions.
    pub fn resize(&mut self, width: i32, height: i32) {
        self.ctx.resize(width, height);
    }
}

/// Simple base64 encoding (no external dependency).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use xulo_render_core::Painter;

    #[test]
    fn bridge_create_and_read() {
        let mut bridge = SkiaBitmapBridge::new(100, 100).unwrap();
        let pixels = bridge.read_pixels();
        assert!(pixels.is_some());
        assert_eq!(pixels.unwrap().len(), 100 * 100 * 4);
    }

    #[test]
    fn bridge_draw_and_read() {
        let mut bridge = SkiaBitmapBridge::new(100, 100).unwrap();
        {
            let ctx = bridge.context_mut();
            ctx.set_fill_color("#ff0000");
            ctx.begin_path();
            ctx.rect(0.0, 0.0, 50.0, 50.0);
            ctx.fill();
        }
        let pixels = bridge.read_pixels().unwrap();
        assert_eq!(pixels[0], 255);
        assert_eq!(pixels[1], 0);
        assert_eq!(pixels[2], 0);
        assert_eq!(pixels[3], 255);
        let last = (100 * 100 - 1) * 4;
        assert_eq!(pixels[last], 0);
    }

    #[test]
    fn to_data_uri() {
        let mut bridge = SkiaBitmapBridge::new(10, 10).unwrap();
        {
            let ctx = bridge.context_mut();
            ctx.set_fill_color("#00ff00");
            ctx.begin_path();
            ctx.rect(0.0, 0.0, 10.0, 10.0);
            ctx.fill();
        }
        let uri = bridge.to_data_uri().unwrap();
        assert!(uri.starts_with("data:image/png;base64,"));
        assert!(uri.len() > 30);
    }

    #[test]
    fn to_html_page() {
        let mut bridge = SkiaBitmapBridge::new(10, 10).unwrap();
        {
            let ctx = bridge.context_mut();
            ctx.set_fill_color("#0000ff");
            ctx.begin_path();
            ctx.rect(0.0, 0.0, 10.0, 10.0);
            ctx.fill();
        }
        let html = bridge.to_html_page().unwrap();
        assert!(html.contains("<img"));
        assert!(html.contains("data:image/png;base64,"));
    }

    #[test]
    fn base64_encode_basic() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
    }
}
