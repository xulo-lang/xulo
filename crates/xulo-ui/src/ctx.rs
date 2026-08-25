//! UI context: the surface, text metrics, and theme a widget tree renders with.

use crate::layout::{FontMetrics, Placed, Theme};
use crate::painting::PaintOp;
use crate::widgets::{Size, Widget};

/// The state a render pass needs: how big the surface is, how text measures,
/// and the color palette. Rendering a widget tree with [`UiContext::paint`]
/// produces backend-agnostic [`PaintOp`]s.
pub struct UiContext {
    pub surface: Size,
    pub metrics: Box<dyn FontMetrics>,
    pub theme: Theme,
}

impl UiContext {
    pub fn new(surface: Size, metrics: Box<dyn FontMetrics>) -> Self {
        Self {
            surface,
            metrics,
            theme: Theme::default(),
        }
    }

    /// Lay `root` out against the surface and return the placed tree.
    pub fn layout<'a>(&self, root: &'a Widget) -> Placed<'a> {
        crate::layout::layout(root, self.surface, self.metrics.as_ref())
    }

    /// Lay `root` out and flatten it into paint commands.
    pub fn paint<'a>(&self, root: &'a Widget) -> Vec<PaintOp<'a>> {
        let placed = self.layout(root);
        let mut ops = Vec::new();
        crate::layout::paint(&placed, &self.theme, &mut ops);
        ops
    }
}
