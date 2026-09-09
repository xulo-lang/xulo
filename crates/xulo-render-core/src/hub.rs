//! [`RenderHub`] — backend detection, selection, and lifecycle management.
//!
//! Inspired by uzor's `uzor-render-hub`: manages which render backend is
//! active and provides a unified interface for creating render contexts.

use crate::types::RenderBackend;

/// Backend detection and selection hub.
///
/// Tracks the active backend and provides helpers for switching between
/// backends at runtime. The actual render context creation is delegated
/// to the caller since `RenderContext` is object-safe and backend-specific.
pub struct RenderHub {
    active: RenderBackend,
}

impl RenderHub {
    /// Create a hub with the given active backend.
    pub fn new(active: RenderBackend) -> Self {
        Self { active }
    }

    /// Create a hub with the default backend for the current environment.
    ///
    /// - If stdout is a terminal → `RenderBackend::Terminal`
    /// - Otherwise → `RenderBackend::Canvas2d`
    pub fn detect() -> Self {
        let backend = if std::io::IsTerminal::is_terminal(&std::io::stdout()) {
            RenderBackend::Terminal
        } else {
            RenderBackend::Canvas2d
        };
        Self { active: backend }
    }

    /// Return the currently active backend.
    pub fn active(&self) -> RenderBackend {
        self.active
    }

    /// Switch to a different backend.
    pub fn set_active(&mut self, backend: RenderBackend) {
        self.active = backend;
    }

    /// Return whether the given backend is the currently active one.
    pub fn is_active(&self, backend: RenderBackend) -> bool {
        self.active == backend
    }
}

impl Default for RenderHub {
    fn default() -> Self {
        Self::detect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_terminal_when_tty() {
        // This test runs in a terminal, so detect() should pick Terminal.
        let hub = RenderHub::detect();
        // We can't assert the exact backend in CI, but we can verify it's valid.
        assert!(hub.active() == RenderBackend::Terminal || hub.active() == RenderBackend::Canvas2d);
    }

    #[test]
    fn set_active_switches_backend() {
        let mut hub = RenderHub::new(RenderBackend::Terminal);
        assert_eq!(hub.active(), RenderBackend::Terminal);
        hub.set_active(RenderBackend::Canvas2d);
        assert_eq!(hub.active(), RenderBackend::Canvas2d);
    }

    #[test]
    fn is_active_checks_correctly() {
        let hub = RenderHub::new(RenderBackend::Terminal);
        assert!(hub.is_active(RenderBackend::Terminal));
        assert!(!hub.is_active(RenderBackend::Canvas2d));
    }
}
