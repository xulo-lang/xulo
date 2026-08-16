use std::collections::HashMap;

use xulo_core::ast::{Param, Type};

#[derive(Debug, Clone, PartialEq)]
pub enum SymbolKind {
    Variable,
    /// A `@State` variable (reactive, mutable component state).
    State,
    /// A `@Store` binding (reactive store-derived value).
    Store,
    Function(Vec<String>, Vec<Param>, Type),
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
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    pub fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub fn pop_scope(&mut self) {
        if self.scopes.len() > 1 {
            self.scopes.pop();
        }
    }

    /// Declare a symbol in the current (innermost) scope.
    ///
    /// Returns `false` if the name is already declared in the innermost scope.
    pub fn declare(&mut self, symbol: Symbol) -> bool {
        let scope = self.scopes.last_mut().expect("scope stack never empty");
        if scope.contains_key(&symbol.name) {
            false
        } else {
            scope.insert(symbol.name.clone(), symbol);
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
}
