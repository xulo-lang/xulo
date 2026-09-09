//! Shared platform types — window identifiers, errors, configuration.

use std::sync::atomic::{AtomicUsize, Ordering};

// ── WindowId ────────────────────────────────────────────────────────────────

/// Opaque window identifier. Each created window gets a unique id.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct WindowId(usize);

impl WindowId {
    pub fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for WindowId {
    fn default() -> Self {
        Self::new()
    }
}

// ── PlatformError ───────────────────────────────────────────────────────────

/// Errors returned by platform operations.
#[derive(Debug, Clone, thiserror::Error)]
pub enum PlatformError {
    #[error("Window not found")]
    WindowNotFound,
    #[error("Failed to create window: {0}")]
    CreationFailed(String),
    #[error("Platform operation not supported")]
    NotSupported,
    #[error("System error: {0}")]
    SystemError(String),
}

// ── WindowConfig ────────────────────────────────────────────────────────────

/// Configuration for creating a new window.
#[derive(Debug, Clone)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub resizable: bool,
    pub decorations: bool,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: "xulo".to_string(),
            width: 800,
            height: 600,
            resizable: true,
            decorations: true,
        }
    }
}

impl WindowConfig {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Default::default()
        }
    }
}

// ── EventResult ─────────────────────────────────────────────────────────────

/// Result of processing a single platform event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventResult {
    /// Keep the event loop running.
    Continue,
    /// A redraw is requested.
    Redraw,
    /// The application should exit.
    Exit,
}

// ── RenderBackend ───────────────────────────────────────────────────────────

/// Available rendering backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum RenderBackend {
    /// CPU rasterizer using character cells (terminal).
    #[default]
    Terminal,
    /// Browser Canvas 2D context (webview/wasm).
    Canvas2d,
    /// GPU-accelerated via wgpu (future).
    Wgpu,
}

// ── PlatformEvent ───────────────────────────────────────────────────────────

/// Cross-platform window event.
#[derive(Debug, Clone)]
pub enum PlatformEvent {
    /// The window needs to be redrawn.
    Redraw,
    /// The user requested closing the window.
    CloseRequested,
    /// The window was resized.
    Resized { width: u32, height: u32 },
    /// A mouse button was pressed or released.
    MouseInput {
        x: f64,
        y: f64,
        pressed: bool,
    },
    /// The mouse cursor moved.
    MouseMove { x: f64, y: f64 },
    /// Scroll wheel input.
    Wheel {
        delta_x: f64,
        delta_y: f64,
    },
    /// Text input (from keyboard).
    TextInput(String),
    /// A key was pressed (key name string).
    KeyInput(String),
}
