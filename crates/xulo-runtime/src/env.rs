use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::value::Value;

/// A lexical scope chain. Bindings are looked up through parent scopes;
/// assignment walks the chain to the scope that declared the name (mirroring
/// JavaScript, where `let` reassignment targets the nearest existing binding).
#[derive(Default)]
pub struct Env {
    parent: Option<Rc<RefCell<Env>>>,
    bindings: HashMap<String, Value>,
}

impl Env {
    /// A new top-level environment with no parent.
    pub fn root() -> Rc<RefCell<Env>> {
        Rc::new(RefCell::new(Env::default()))
    }

    /// A fresh scope whose parent is `parent` (used for blocks, loop bodies,
    /// and function calls).
    pub fn child(parent: &Rc<RefCell<Env>>) -> Rc<RefCell<Env>> {
        Rc::new(RefCell::new(Env {
            parent: Some(parent.clone()),
            bindings: HashMap::new(),
        }))
    }

    /// Bind `name` in this scope, shadowing any outer binding of the same name.
    pub fn define(&mut self, name: &str, value: Value) {
        self.bindings.insert(name.to_string(), value);
    }

    /// Drop every binding in this scope. Used when the owning
    /// [`Interpreter`](crate::interpreter::Interpreter) is dropped: a top-level
    /// `fn` binds into the root env while its closure captures that same env, so
    /// `root -> "f" -> FunctionValue { closure: root }` is an Rc cycle that
    /// would otherwise outlive the interpreter (known issue D11). Clearing the
    /// root's bindings releases the function values (and any other reachable
    /// values), breaking the cycle so the whole env graph is reclaimed.
    pub fn clear(&mut self) {
        self.bindings.clear();
    }

    /// Look `name` up through the scope chain.
    pub fn get(&self, name: &str) -> Option<Value> {
        if let Some(value) = self.bindings.get(name) {
            return Some(value.clone());
        }
        self.parent.as_ref().and_then(|p| p.borrow().get(name))
    }

    /// Reassign `name`, walking outward to the scope that declared it. Returns
    /// `false` when the name is not declared anywhere.
    pub fn assign(&mut self, name: &str, value: Value) -> bool {
        if self.bindings.contains_key(name) {
            self.bindings.insert(name.to_string(), value);
            return true;
        }
        if let Some(parent) = &self.parent {
            return parent.borrow_mut().assign(name, value);
        }
        false
    }
}
