//! UI context: the surface, text metrics, and theme a widget tree renders with.

use crate::layout::{FontMetrics, HandwrittenLayout, LayoutEngine, Placed, Theme};
use crate::painting::PaintOp;
use crate::widgets::{Size, Widget};

/// The state a render pass needs: how big the surface is, how text measures,
/// and the color palette. Rendering a widget tree with [`UiContext::paint`]
/// produces backend-agnostic [`PaintOp`]s.
pub struct UiContext {
    pub surface: Size,
    pub metrics: Box<dyn FontMetrics>,
    pub theme: Theme,
    engine: Box<dyn LayoutEngine>,
}

impl UiContext {
    pub fn new(surface: Size, metrics: Box<dyn FontMetrics>) -> Self {
        Self {
            surface,
            metrics,
            theme: Theme::default(),
            engine: Box::new(HandwrittenLayout),
        }
    }

    /// Create a context with a specific layout engine.
    pub fn with_engine(
        surface: Size,
        metrics: Box<dyn FontMetrics>,
        engine: Box<dyn LayoutEngine>,
    ) -> Self {
        Self {
            surface,
            metrics,
            theme: Theme::default(),
            engine,
        }
    }

    /// Switch the layout engine at runtime.
    pub fn set_engine(&mut self, engine: Box<dyn LayoutEngine>) {
        self.engine = engine;
    }

    /// Lay `root` out against the surface and return the placed tree.
    pub fn layout<'a>(&self, root: &'a Widget) -> Placed<'a> {
        self.engine
            .layout(root, self.surface, self.metrics.as_ref())
    }

    /// Lay `root` out and flatten it into paint commands.
    pub fn paint<'a>(&self, root: &'a Widget) -> Vec<PaintOp<'a>> {
        let placed = self.layout(root);
        let mut ops = Vec::new();
        crate::layout::paint(&placed, &self.theme, &mut ops);
        ops
    }
}
