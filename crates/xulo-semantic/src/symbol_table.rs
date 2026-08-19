use std::collections::HashMap;
use std::ops::Range;

use xulo_core::ast::{FnBound, Param, Type};

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Variable,
    /// A `@State` variable (reactive, mutable component state).
    State,
    /// A `@Store` binding (reactive store-derived value).
    Store,
    Function(Vec<String>, Vec<Param>, Type, Vec<FnBound>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    pub name: String,
    pub type_: Type,
    pub kind: SymbolKind,
    /// `true` when the binding may not be reassigned (`const`, or a parameter).
    pub is_const: bool,
}

/// A stack of scopes mapping names to symbols. The innermost scope is last.
#[derive(Debug, Default)]
pub struct SymbolTable {
    scopes: Vec<HashMap<String, Symbol>>,
    /// Parallel to `scopes`: the declaration span of the binding that declares
    /// the innermost scope's entry with the same name. `None` marks a
    /// synthesized/imported symbol whose source location is not statically
    /// known (kept distinct from "unknown" by the enclosing `Option`).
    defs: Vec<HashMap<String, Option<Range<usize>>>>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            defs: vec![HashMap::new()],
        }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.defs.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
            self.defs.pop();
        }
    }

    /// Declare a symbol in the current (innermost) scope.
    ///
    /// Returns `false` if the name is already declared in the innermost scope.
    pub fn declare(&mut self, symbol: Symbol) -> bool {
        self.declare_with_def(symbol, None)
    }

    /// Declare a symbol in the current (innermost) scope, recording the source
    /// span of its declaration site for editor tooling / name diagnostics.
    ///
    /// Returns `false` if the name is already declared in the innermost scope.
    pub fn declare_with_def(&mut self, symbol: Symbol, def: Option<Range<usize>>) -> bool {
        let scope = self.scopes.last_mut().expect("scope stack never empty");
        let defs = self.defs.last_mut().expect("scope stack never empty");
        if scope.contains_key(&symbol.name) {
            false
        } else {
            let name = symbol.name.clone();
            scope.insert(name.clone(), symbol);
            defs.insert(name, def);
            true
        }
    }

    /// Look up a symbol walking outward from the innermost scope.
    pub fn lookup(&self, name: &str) -> Option<&Symbol> {
        for scope in self.scopes.iter().rev() {
            if let Some(sym) = scope.get(name) {
                return Some(sym);
            }
        }
        None
    }

    /// The declaration span of the symbol `name` resolves to, or `None` when
    /// the name is unbound. The span itself is `Option` because some symbols
    /// (imports, synthesized bindings) have no static declaration site.
    pub fn lookup_def(&self, name: &str) -> Option<Option<&Range<usize>>> {
        for defs in self.defs.iter().rev() {
            if let Some(def) = defs.get(name) {
                return Some(def.as_ref());
            }
        }
        None
    }
}
