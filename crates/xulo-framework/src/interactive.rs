//! Interactive UI sessions: a render tree that can be re-rendered in place.
//!
//! [`ReactiveUi`] owns the interpreter and the entry program. Clicking a
//! button invokes its `onClick` callback — which mutates the `@State` cells
//! the component declared — then re-invokes `main` to rebuild the render tree.
//! Because the interpreter reuses `@State` cells across renders, state survives
//! the re-render.

use std::path::Path;
use std::rc::Rc;

use xulo_core::ast::Program;
use xulo_core::error::XuloError;
use xulo_runtime::interpreter::Interpreter;
use xulo_ui::{PaintOp, Rect, Size, Widget};

use crate::convert::{widget_tree_with_callbacks, UiCallback};
use crate::run::execute_in;

/// Builds one frame (paint commands + button/input rectangles) for a widget
/// tree against a surface, using a backend's metrics. `None` for backends
/// (webview) that let the wasm engine do layout.
pub type FrameBuilder =
    Box<dyn Fn(&Widget, Size) -> (Vec<PaintOp<'_>>, Vec<Rect>, Vec<Rect>)>;

/// A live, re-renderable UI session.
pub struct ReactiveUi {
    pub interp: Rc<Interpreter>,
    /// The entry program; its `main` is re-invoked on each re-render.
    pub program: Program,
    /// The surface the tree is laid out against (native backends only).
    pub surface: Size,
    frame: Option<FrameBuilder>,
    /// `print` output collected while loading the program.
    pub output: Vec<String>,
    /// The widget tree of the current frame (kept so the wasm backend can
    /// serialize it to the page).
    pub widget: Widget,
    /// Button rectangles, in the same (tree pre-)order as `callbacks`.
    pub buttons: Vec<Rect>,
    /// Input field rectangles, in the same order as `input_callbacks`.
    pub inputs: Vec<Rect>,
    pub callbacks: Vec<UiCallback>,
    /// Indices into `callbacks` for each input field (parallel to `inputs`).
    pub input_callback_indices: Vec<usize>,
}

impl ReactiveUi {
    /// Load and execute `entry`, then render its first frame.
    pub fn load(
        entry: &Path,
        surface: Size,
        frame: Option<FrameBuilder>,
    ) -> Result<Self, XuloError> {
        let interp = Rc::new(Interpreter::new());
        let program = execute_in(entry, &interp)?;
        let mut ui = Self {
            interp,
            program,
            surface,
            frame,
            output: Vec::new(),
            widget: Widget::Screen {
                background: None,
                children: Vec::new(),
            },
            buttons: Vec::new(),
            inputs: Vec::new(),
            callbacks: Vec::new(),
            input_callback_indices: Vec::new(),
        };
        ui.output = ui.interp.take_output();
        ui.render_frame()?;
        Ok(ui)
    }

    /// Compute paint commands from the current widget tree.
    pub fn ops(&self) -> Vec<PaintOp<'_>> {
        if let Some(frame) = &self.frame {
            let (ops, _, _) = frame(&self.widget, self.surface);
            ops
        } else {
            Vec::new()
        }
    }

    /// Rebuild the frame from the interpreter's current render tree.
    pub fn render_frame(&mut self) -> Result<(), XuloError> {
        let view = self.interp.take_root_view().ok_or_else(|| {
            XuloError::new(
                xulo_core::error::ErrorKind::Runtime,
                "program did not produce a `View`; cannot render",
            )
        })?;
        let (widget, callbacks) = widget_tree_with_callbacks(&view, &self.interp);
        self.widget = widget;
        self.callbacks = callbacks;
        if let Some(frame) = &self.frame {
            let (_, buttons, inputs) = frame(&self.widget, self.surface);
            self.buttons = buttons;
            self.inputs = inputs;
            // Build the input → callback index mapping.
            self.input_callback_indices.clear();
            for cb_idx in 0..self.callbacks.len() {
                if self.callbacks[cb_idx].on_change.is_some() {
                    self.input_callback_indices.push(cb_idx);
                }
            }
        }
        Ok(())
    }

    /// Click the button whose rectangle contains `(x, y)`. Returns whether a
    /// button was hit (and the frame re-rendered).
    pub fn handle_click(&mut self, x: u32, y: u32) -> Result<bool, XuloError> {
        if let Some(index) = self.buttons.iter().position(|r| r.contains(x, y)) {
            self.click_button(index)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Click the `index`-th button (0-based, tree order): run its `onClick`,
    /// re-invoke `main`, and re-render the frame.
    pub fn click_button(&mut self, index: usize) -> Result<(), XuloError> {
        if let Some(callback) = self.callbacks.get(index) {
            if let Some(ref on_click) = callback.on_click {
                on_click();
            }
            self.interp
                .rerender_main(&self.program)
                .map_err(crate::run::map_run_error)?;
            self.render_frame()?;
        }
        Ok(())
    }

    /// Change the value of the `index`-th input field (0-based, tree order):
    /// run its `onChange` callback, re-invoke `main`, and re-render.
    pub fn input_change(&mut self, index: usize, new_value: String) -> Result<(), XuloError> {
        let cb_idx = self.input_callback_indices.get(index).copied();
        if let Some(cb_idx) = cb_idx {
            if let Some(callback) = self.callbacks.get(cb_idx) {
                if let Some(ref on_change) = callback.on_change {
                    on_change(new_value);
                }
            }
            self.interp
                .rerender_main(&self.program)
                .map_err(crate::run::map_run_error)?;
            self.render_frame()?;
        }
        Ok(())
    }
}
