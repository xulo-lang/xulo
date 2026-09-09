//! [`Effects`] — visual effects (shadow, blur).

/// Visual effects applied to subsequent draw calls.
///
/// MVP: all methods are no-ops. Backends that support shadows or blur
/// override the relevant methods.
pub trait Effects {
    /// Set the shadow offset. Default no-op.
    fn set_shadow_offset(&mut self, _x: f64, _y: f64) {}

    /// Set the shadow blur radius. Default no-op.
    fn set_shadow_blur(&mut self, _radius: f64) {}

    /// Set the shadow color (CSS hex string). Default no-op.
    fn set_shadow_color(&mut self, _color: &str) {}

    /// Clear all shadow settings. Default no-op.
    fn clear_shadow(&mut self) {}
}
