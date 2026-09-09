//! Terminal platform backend — reads input events from stdin.

use xulo_render_core::{
    PlatformBackend, PlatformEvent, SystemIntegration, WindowConfig, WindowId, PlatformError,
};

/// Terminal platform backend. Implements [`PlatformBackend`] for stdin-based
/// interactive sessions.
pub struct TerminalPlatform {
    windows: Vec<WindowId>,
}

impl Default for TerminalPlatform {
    fn default() -> Self {
        Self::new()
    }
}

impl TerminalPlatform {
    pub fn new() -> Self {
        Self {
            windows: Vec::new(),
        }
    }

    /// Read a line from stdin and convert it to platform events.
    pub fn read_stdin_line(&self) -> Option<PlatformEvent> {
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) | Err(_) => return Some(PlatformEvent::CloseRequested),
            _ => {}
        }
        let trimmed = line.trim();
        match trimmed {
            "q" | "Q" => Some(PlatformEvent::CloseRequested),
            "r" | "R" | "" => Some(PlatformEvent::Redraw),
            _ if trimmed.starts_with('i') && trimmed.contains('=') => {
                // Parse `i1=hello` format — return as TextInput for now.
                Some(PlatformEvent::TextInput(trimmed.to_string()))
            }
            _ => {
                // Button click: "1", "2", etc.
                Some(PlatformEvent::TextInput(trimmed.to_string()))
            }
        }
    }
}

impl PlatformBackend for TerminalPlatform {
    fn name(&self) -> &'static str {
        "terminal"
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
        // Non-blocking: return empty. The caller should use `read_stdin_line`
        // for the blocking interactive loop.
        Vec::new()
    }

    fn request_redraw(&self, _id: WindowId) {
        // Terminal redraws are synchronous — no-op.
    }
}

impl SystemIntegration for TerminalPlatform {
    fn get_clipboard(&self) -> Option<String> {
        None
    }

    fn set_clipboard(&self, _text: &str) {}
}
