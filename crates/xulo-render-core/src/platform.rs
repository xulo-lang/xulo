//! Platform abstraction — windowing, input, system integration.

use super::types::{PlatformError, PlatformEvent, WindowConfig, WindowId};

/// Platform backend — window creation, event polling, redraw requests.
///
/// Each platform (desktop/winit, terminal, web) provides an implementation.
pub trait PlatformBackend: Send + Sync {
    /// Human-readable backend name (e.g. `"winit"`, `"terminal"`).
    fn name(&self) -> &'static str;

    /// Create a new window with the given configuration.
    fn create_window(&mut self, config: WindowConfig) -> Result<WindowId, PlatformError>;

    /// Close a previously created window.
    fn close_window(&mut self, id: WindowId) -> Result<(), PlatformError>;

    /// Return the primary (first) window, if any.
    fn primary_window(&self) -> Option<WindowId>;

    /// Drain all pending events since the last call.
    fn poll_events(&mut self) -> Vec<PlatformEvent>;

    /// Request a redraw for the given window.
    fn request_redraw(&self, id: WindowId);
}

/// System-level integration (clipboard, theme, etc.).
pub trait SystemIntegration {
    /// Read the clipboard contents, if available.
    fn get_clipboard(&self) -> Option<String>;

    /// Write text to the clipboard.
    fn set_clipboard(&self, text: &str);
}

// ── MockPlatform (for tests) ────────────────────────────────────────────────

/// A mock platform for unit testing.
pub struct MockPlatform {
    windows: Vec<WindowId>,
    events: Vec<PlatformEvent>,
}

impl Default for MockPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl MockPlatform {
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
            events: Vec::new(),
        }
    }

    /// Push an event to be returned by the next `poll_events` call.
    pub fn push_event(&mut self, event: PlatformEvent) {
        self.events.push(event);
    }
}

impl PlatformBackend for MockPlatform {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn create_window(&mut self, _config: WindowConfig) -> Result<WindowId, PlatformError> {
        let id = WindowId::new();
        self.windows.push(id);
        Ok(id)
    }

    fn close_window(&mut self, id: WindowId) -> Result<(), PlatformError> {
        self.windows.retain(|w| *w != id);
        Ok(())
    }

    fn primary_window(&self) -> Option<WindowId> {
        self.windows.first().copied()
    }

    fn poll_events(&mut self) -> Vec<PlatformEvent> {
        std::mem::take(&mut self.events)
    }

    fn request_redraw(&self, _id: WindowId) {}
}

impl SystemIntegration for MockPlatform {
    fn get_clipboard(&self) -> Option<String> {
        None
    }

    fn set_clipboard(&self, _text: &str) {}
}
