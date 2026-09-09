//! [`Masking`] — clip region management.

/// Clip region management.
///
/// Backends that support clipping implement this trait. The clip stack
/// mirrors the `save`/`restore` discipline: each `save` captures the
/// current clip, and `restore` reverts to the previous one.
pub trait Masking {
    /// Clip to the current path using the nonzero winding rule.
    /// After this call, only pixels inside both the previous clip and the
    /// current path are drawn.
    fn clip(&mut self);
}
