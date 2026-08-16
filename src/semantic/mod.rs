pub mod symbol_table;

use std::collections::HashMap;
use std::ops::Range;

use crate::ast::{
    AssignStmt, AssignTarget, BinaryOperator, Block, Call, CallArg, EnumVariant, Expression,
    IfExpr, LetBinding, Literal, ObjectField, Program, Statement, Type, TypeAlias,
};
use crate::error::{ErrorKind, XuloError};
use symbol_table::{Symbol, SymbolKind, SymbolTable};

/// Result of a single semantic error, without a source span (spans are
/// attached by the lexer/parser; semantic checks target names).
type SResult<T> = Result<T, XuloError>;

/// A named type known to the program: either a user `type` alias or an `enum`.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeEntryKind {
    Alias(Type),
    Enum(Vec<EnumVariant>),
}

impl TypeEntryKind {
    pub fn variants(&self) -> Option<&Vec<EnumVariant>> {
        match self {
            TypeEntryKind::Enum(variants) => Some(variants),
            TypeEntryKind::Alias(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TypeEntry {
    pub type_params: Vec<String>,
    pub kind: TypeEntryKind,
}

pub struct Analyzer {
    table: SymbolTable,
    current_return: Option<Type>,
    /// Whether the current function declared a return type (docs §21.2: only
    /// then does a trailing expression count as its implicit return).
    declared_return: bool,
    type_table: HashMap<String, TypeEntry>,
    /// Type-parameter names currently in scope (these are valid as `Named`)
    /// and are erased to `Any` for kind/arithmetic checks.
    generics: Vec<String>,
    /// Depth of enclosing async functions; `await` is only valid when > 0.
    async_depth: usize,
    /// Depth of enclosing `Component`-returning functions; `@State`/`@Store`/
    /// `@Effect`/`@Environment` are only valid when > 0.
    component_depth: usize,
    /// Depth of nested blocks below the current function body's top level;
    /// decorators are only allowed when this is 0 (docs §12).
    block_depth: usize,
    /// Depth of `if`/`match` expressions checked in value position; their
    /// arms compile to a plain (non-`async`) function, so `await` is
    /// rejected there regardless of the enclosing `async_depth`.
    no_await_depth: usize,
    /// Imported names available to this module (name -> symbol).
    imports: HashMap<String, Symbol>,
    /// Names imported from an unseeded external package (no module graph): the
    /// loader provides no signature, so calls to these are checked opaquely.
    opaque: std::collections::HashSet<String>,
    /// Imported type/alias names available to this module.
    imported_types: HashMap<String, TypeEntry>,
    /// Names exported from this module for codegen / module resolution.
    exported: Vec<String>,
    exported_default: Option<String>,
    /// Exported symbols captured for cross-module type checking.
    exported_symbols: Vec<(String, Symbol)>,
    /// Non-fatal diagnostics raised during analysis (e.g. ignored return
    /// values); surfaced by the CLI after a successful compile.
    warnings: Vec<XuloError>,
    /// Per-component stack of names declared by the component's *render* code
    /// (plain `let`s and nested function declarations in the function body).
    /// These live inside the `__component(...)` render closure at runtime, so
    /// `@Effect`/`@State`/`@Store` setup code (hoisted above it) may not
    /// reference them.
    render_locals: Vec<std::collections::HashSet<String>>,
    /// Set while checking an `@Effect` closure (and its deps): identifiers that
    /// resolve to render-scoped locals are rejected (they are out of scope when
    /// the effect runs).
    in_effect: bool,
    /// Span of the expression most recently entered by `check_expression`;
    /// semantic errors are attached to it (they cannot name their own span).
    current_span: Range<usize>,
}

/// The result of analyzing a module: the names/symbols/types it exports, so a
/// loader can seed dependent modules.
#[derive(Debug, Clone, Default)]
pub struct AnalysisResult {
    pub exported_symbols: Vec<(String, Symbol)>,
    pub exported_types: Vec<(String, TypeEntry)>,
    pub default: Option<String>,
    /// Non-fatal diagnostics from this module.
    pub warnings: Vec<XuloError>,
}

/// Run all semantic checks over a parsed program.
pub fn analyze(program: &Program) -> Result<(), XuloError> {
    analyze_with(program, &[], &[]).map(|_: AnalysisResult| ())
}

/// Semantic checks with imported module symbols/types registered before the
/// module's own declarations. Returns the names/symbols the module exports.
pub fn analyze_with(
    program: &Program,
    imports: &[Symbol],
    imported_types: &[(String, TypeEntry)],
) -> Result<AnalysisResult, XuloError> {
    let mut analyzer = Analyzer {
        table: SymbolTable::new(),
        current_return: None,
        declared_return: false,
        type_table: HashMap::new(),
        generics: Vec::new(),
        async_depth: 0,
        component_depth: 0,
        block_depth: 0,
        no_await_depth: 0,
        imports: imports
            .iter()
            .map(|s| (s.name.clone(), s.clone()))
            .collect(),
        opaque: std::collections::HashSet::new(),
        imported_types: imported_types
            .iter()
            .map(|(n, t)| (n.clone(), t.clone()))
            .collect(),
        exported: Vec::new(),
        exported_default: None,
        exported_symbols: Vec::new(),
        warnings: Vec::new(),
        render_locals: Vec::new(),
        in_effect: false,
        current_span: 0..0,
    };
    for statement in &program.statements {
        analyzer.check_statement(statement)?;
    }
    let mut result = AnalysisResult {
        exported_symbols: analyzer.exported_symbols,
        exported_types: Vec::new(),
        default: analyzer.exported_default.clone(),
        warnings: analyzer.warnings.clone(),
    };
    for name in &analyzer.exported {
        if let Some(entry) = analyzer.type_table.get(name).cloned() {
            result.exported_types.push((name.clone(), entry));
        }
    }
    Ok(result)
}

fn assign_target_name(target: &AssignTarget) -> String {
    match target {
        AssignTarget::Name(name) => name.clone(),
        AssignTarget::Member(object, property) => {
            format!("{}.{property}", target_base(object))
        }
        AssignTarget::Index(object, _) => format!("{}[...]", target_base(object)),
    }
}

fn target_base(expr: &Expression) -> String {
    match expr {
        Expression::Identifier { name, .. } => name.clone(),
        Expression::Member(m) => format!("{}.{}", target_base(&m.object), m.property),
        Expression::Index(i) => format!("{}[...]", target_base(&i.object)),
        Expression::Call(c) => c.callee.clone(),
        _ => "expr".into(),
    }
}

impl Analyzer {
    fn warn(&mut self, message: String) {
        self.warnings
            .push(XuloError::new(ErrorKind::Warning, message).at(self.current_span.clone()));
    }

    /// Raise a semantic error attached to the most recently checked
    /// expression's span (see [`Analyzer::current_span`]).
    fn err(&self, message: impl Into<String>) -> XuloError {
        XuloError::new(ErrorKind::Semantic, message).at(self.current_span.clone())
    }
}

impl Analyzer {
    /// Resolve a name to its symbol, rejecting references from inside an
    /// `@Effect` closure to bindings that live only in the component's render
    /// code: those are hoisted above the render closure and are not in scope
    /// when the effect runs.
    fn lookup_symbol(&self, name: &str) -> SResult<Symbol> {
        let render_scoped = self
            .render_locals
            .last()
            .is_some_and(|locals| locals.contains(name));
        if self.in_effect && render_scoped {
            return Err(self.err(format!(
                "`@Effect` closures cannot reference `{name}`: it is declared inside the component body and is out of scope when the effect runs"
            )));
        }
        self.table
            .lookup(name)
            .cloned()
            .ok_or_else(|| self.err(format!("undefined variable `{name}`")))
    }
}

impl Analyzer {
    fn check_statement(&mut self, statement: &Statement) -> SResult<()> {
        match statement {
            Statement::Fn(f) => {
                self.generics.extend(f.type_params.iter().cloned());
                let result = self.check_fn(f);
                self.generics
                    .truncate(self.generics.len().saturating_sub(f.type_params.len()));
                result
            }
            Statement::Let(binding) => {
                if let Some(annotation) = &binding.type_annotation {
                    self.check_type(annotation)?;
                }
                let value_type = match &binding.value {
                    Some(value) => self.check_expression(value)?,
                    None => binding.type_annotation.clone().unwrap_or(Type::Any),
                };
                let mut ok = true;
                if let Some(annotation) = &binding.type_annotation
                    && !self.assignable(&value_type, annotation)
                {
                    // Value-aware fallback for string-literal types
                    // (`"active" | "inactive"`): a direct literal may match.
                    let literal_ok = match &binding.value {
                        Some(value) => self.literal_matches(value, annotation),
                        None => false,
                    };
                    if !literal_ok {
                        ok = false;
                    }
                }
                if !ok {
                    return Err(self.err(format!(
                        "cannot bind a value of type `{}` to `let {}: {}`",
                        value_type.name(),
                        binding.name,
                        binding.type_annotation.as_ref().unwrap().name()
                    )));
                }
                let declared = self.table.declare(Symbol {
                    name: binding.name.clone(),
                    type_: binding.type_annotation.clone().unwrap_or(value_type),
                    kind: SymbolKind::Variable,
                    is_const: binding.is_const,
                });
                if !declared {
                    return Err(self.err(format!(
                        "binding `{}` is already declared in this scope",
                        binding.name
                    )));
                }
                Ok(())
            }
            Statement::Assign(assign) => self.check_assign(assign),
            Statement::TypeAlias(alias) => self.check_type_alias(alias),
            Statement::Enum(e) => self.check_enum(e),
            Statement::Return(stmt) => {
                let Some(value) = &stmt.value else {
                    // Bare `return;` is valid in any function (docs EBNF §7),
                    // but only inside a function body.
                    if self.current_return.is_none() {
                        return Err(self
                            .err("`return` may only be used at the top level of a function body"));
                    }
                    return Ok(());
                };
                let value_type = self.check_expression(value)?;
                if let Some(expected) = &self.current_return {
                    let target = match expected {
                        Type::Async(inner) => inner.as_ref(),
                        other => other,
                    };
                    if !self.assignable(&value_type, target) {
                        return Err(self.err(format!(
                            "return type mismatch: expected `{}`, found `{}`",
                            expected.name(),
                            value_type.name()
                        )));
                    }
                } else {
                    return Err(self.err(
                        format!(
                            "`return` may only be used at the top level of a function body (found a value of type `{}`)",
                            value_type.name()
                        ),
                    ));
                }
                Ok(())
            }
            Statement::For(stmt) => {
                let iterable_type = self.check_expression(&stmt.iterable)?;
                match iterable_type {
                    Type::List(_) | Type::Any => {}
                    other => {
                        return Err(self.err(format!(
                            "for loop must iterate over a `list`, found `{}`",
                            other.name()
                        )));
                    }
                }
                self.table.push_scope();
                self.table.declare(Symbol {
                    name: stmt.iter_var.clone(),
                    type_: Type::Any,
                    kind: SymbolKind::Variable,
                    is_const: false,
                });
                let result = self.check_block(&stmt.body);
                self.table.pop_scope();
                result?;
                Ok(())
            }
            Statement::While(stmt) => {
                let condition = self.check_expression(&stmt.condition)?;
                if !self.assignable(&condition, &Type::Boolean) {
                    return Err(self.err(format!(
                        "while condition must be a `boolean`, found `{}`",
                        condition.name()
                    )));
                }
                self.check_block(&stmt.body)?;
                Ok(())
            }
            Statement::Expr(stmt) => {
                self.check_expr_stmt(stmt)?;
                Ok(())
            }
            Statement::Block(block) => {
                self.check_block(block)?;
                Ok(())
            }
            Statement::Try(try_stmt) => {
                self.check_block(&try_stmt.try_block)?;
                self.table.push_scope();
                self.table.declare(Symbol {
                    name: try_stmt.catch_var.clone(),
                    type_: Type::Any,
                    kind: SymbolKind::Variable,
                    is_const: true,
                });
                let result = self.check_block(&try_stmt.catch_block);
                self.table.pop_scope();
                result
            }
            Statement::Throw(expr) => {
                self.check_expression(expr)?;
                Ok(())
            }
            Statement::Import(import) => self.check_import(import),
            Statement::Export(export) => self.check_export(export),
            Statement::State(state) => self.check_state(&state.binding),
            Statement::Store(store) => self.check_store(store),
            Statement::Effect(effect) => self.check_effect(effect),
            Statement::Environment(env) => self.check_environment(env),
            Statement::Component(component) => self.check_component(component),
        }
    }

    fn check_fn(&mut self, f: &crate::ast::FnDef) -> SResult<()> {
        let mut param_names = std::collections::HashSet::new();
        for p in &f.params {
            if !param_names.insert(p.name.clone()) {
                return Err(self.err(format!(
                    "parameter `{}` shadows an earlier parameter of `{}`",
                    p.name, f.name
                )));
            }
            if let Some(ty) = &p.type_annotation {
                self.check_type(ty)?;
            }
            if let Some(default) = &p.default {
                let default_type = self.check_expression(default)?;
                let param_type = p.type_annotation.clone().unwrap_or(Type::Any);
                if !self.assignable(&default_type, &param_type) {
                    return Err(self.err(format!(
                        "default value for parameter `{}` must be `{}`, found `{}`",
                        p.name,
                        param_type.name(),
                        default_type.name()
                    )));
                }
            }
        }
        let return_type = f.return_type.clone().unwrap_or(Type::Any);
        self.check_type(&return_type)?;
        let symbol = Symbol {
            name: f.name.clone(),
            type_: return_type.clone(),
            kind: SymbolKind::Function(
                f.type_params.clone(),
                f.params.clone(),
                return_type.clone(),
            ),
            is_const: true,
        };
        if !self.table.declare(symbol) {
            return Err(self.err(format!("function `{}` is already defined", f.name)));
        }

        self.table.push_scope();
        for param in &f.params {
            let ty = param.type_annotation.clone().unwrap_or(Type::Any);
            self.table.declare(Symbol {
                name: param.name.clone(),
                type_: ty,
                kind: SymbolKind::Variable,
                is_const: true,
            });
        }
        let saved = self.current_return.replace(return_type.clone());
        let saved_declared = self.declared_return;
        self.declared_return = f.return_type.is_some();
        let saved_async = self.async_depth;
        self.async_depth = if f.is_async { saved_async + 1 } else { 0 };
        // This function is its own scope: decorators are only legal at the top
        // level of a `Component` function (itself), never inside a nested
        // function, and `await` needs this function to be `async` (docs §12).
        let is_component = self.is_component_type(&return_type);
        let saved_component = self.component_depth;
        let saved_block = self.block_depth;
        self.component_depth = if is_component { 1 } else { 0 };
        self.block_depth = 0;
        if is_component {
            // Top-level `let`s and nested `fn` declarations of a component body
            // become part of the render closure at codegen time, so they are
            // off-limits to hoisted setup code (`@Effect`/`@State`/`@Store`).
            let mut render_locals = std::collections::HashSet::new();
            for stmt in &f.body.statements {
                match stmt {
                    Statement::Let(b) => {
                        render_locals.insert(b.name.clone());
                    }
                    Statement::Fn(nested) => {
                        render_locals.insert(nested.name.clone());
                    }
                    _ => {}
                }
            }
            self.render_locals.push(render_locals);
        }
        let result = self.check_block_implicit(&f.body);
        if is_component {
            self.render_locals.pop();
        }
        self.block_depth = saved_block;
        self.component_depth = saved_component;
        self.async_depth = saved_async;
        self.current_return = saved;
        self.declared_return = saved_declared;
        result?;
        self.table.pop_scope();
        Ok(())
    }

    /// Validate a parameter list and collect each parameter's type (defaults are
    /// checked against the annotation).
    fn check_params_types(&mut self, params: &[crate::ast::Param]) -> SResult<Vec<Type>> {
        let mut types = Vec::with_capacity(params.len());
        for p in params {
            if let Some(ty) = &p.type_annotation {
                self.check_type(ty)?;
            }
            if let Some(default) = &p.default {
                let default_type = self.check_expression(default)?;
                let param_type = p.type_annotation.clone().unwrap_or(Type::Any);
                if !self.assignable(&default_type, &param_type) {
                    return Err(self.err(format!(
                        "default value for parameter `{}` must be `{}`, found `{}`",
                        p.name,
                        param_type.name(),
                        default_type.name()
                    )));
                }
            }
            types.push(p.type_annotation.clone().unwrap_or(Type::Any));
        }
        Ok(types)
    }

    /// An anonymous function literal. Captured names are resolved through the
    /// enclosing scope (closures); the literal's type is a `fn(...) -> ...`
    /// signature.
    fn check_fn_expr(&mut self, f: &crate::ast::FnExpr) -> SResult<Type> {
        let param_types = self.check_params_types(&f.params)?;
        let return_type = f.return_type.clone().unwrap_or(Type::Any);
        self.check_type(&return_type)?;

        self.table.push_scope();
        for param in &f.params {
            let ty = param.type_annotation.clone().unwrap_or(Type::Any);
            self.table.declare(Symbol {
                name: param.name.clone(),
                type_: ty,
                kind: SymbolKind::Variable,
                is_const: true,
            });
        }
        let saved = self.current_return.replace(return_type.clone());
        let saved_declared = self.declared_return;
        self.declared_return = f.return_type.is_some();
        let saved_async = self.async_depth;
        self.async_depth = if f.is_async { saved_async + 1 } else { 0 };
        // A closure is its own (ordinary) function: `@State`/`@Store`/
        // `@Effect`/`@Environment` are not allowed inside it, even when the
        // closure appears at the top level of a `Component` function (docs §12).
        let saved_component = self.component_depth;
        let saved_block = self.block_depth;
        self.component_depth = 0;
        self.block_depth = 0;
        let result = self.check_block_implicit(&f.body);
        self.component_depth = saved_component;
        self.block_depth = saved_block;
        self.async_depth = saved_async;
        self.current_return = saved;
        self.declared_return = saved_declared;
        result?;
        self.table.pop_scope();

        Ok(Type::FnSig {
            params: param_types,
            ret: Some(Box::new(return_type)),
        })
    }

    fn check_import(&mut self, import: &crate::ast::ImportStmt) -> SResult<()> {
        match &import.spec {
            crate::ast::ImportSpec::Bare => Ok(()),
            crate::ast::ImportSpec::Namespace(ns) => {
                if import.type_only {
                    // `import type * as ns` is erased at runtime.
                    return Ok(());
                }
                if !self.table.declare(Symbol {
                    name: ns.clone(),
                    type_: Type::Any,
                    kind: SymbolKind::Variable,
                    is_const: true,
                }) {
                    return Err(self.err(format!("`{ns}` is already declared",)));
                }
                Ok(())
            }
            crate::ast::ImportSpec::Named(names) => {
                for (name, alias) in names {
                    let local = alias.clone().unwrap_or_else(|| name.clone());
                    if import.type_only {
                        // Type-only imports feed the type table (as an opaque
                        // alias unless the loader supplied the real entry).
                        if !self.imported_types.contains_key(&local) {
                            self.imported_types.insert(
                                local.clone(),
                                TypeEntry {
                                    type_params: Vec::new(),
                                    kind: TypeEntryKind::Alias(Type::Any),
                                },
                            );
                        }
                        continue;
                    }
                    let sym = match self.imports.get(name) {
                        Some(symbol) => Symbol {
                            name: local.clone(),
                            type_: symbol.type_.clone(),
                            kind: symbol.kind.clone(),
                            is_const: true,
                        },
                        None => {
                            self.opaque.insert(local.clone());
                            Symbol {
                                name: local.clone(),
                                type_: Type::Any,
                                kind: SymbolKind::Function(Vec::new(), Vec::new(), Type::Any),
                                is_const: true,
                            }
                        }
                    };
                    if !self.table.declare(sym) {
                        return Err(self.err(format!("`{local}` is already declared",)));
                    }
                }
                Ok(())
            }
            crate::ast::ImportSpec::Default(name) => {
                if import.type_only {
                    if !self.imported_types.contains_key(name) {
                        self.imported_types.insert(
                            name.clone(),
                            TypeEntry {
                                type_params: Vec::new(),
                                kind: TypeEntryKind::Alias(Type::Any),
                            },
                        );
                    }
                    return Ok(());
                }
                let sym = match self.imports.get(name) {
                    Some(symbol) => Symbol {
                        name: name.clone(),
                        type_: symbol.type_.clone(),
                        kind: symbol.kind.clone(),
                        is_const: true,
                    },
                    None => {
                        self.opaque.insert(name.clone());
                        Symbol {
                            name: name.clone(),
                            type_: Type::Any,
                            kind: SymbolKind::Function(Vec::new(), Vec::new(), Type::Any),
                            is_const: true,
                        }
                    }
                };
                if !self.table.declare(sym) {
                    return Err(self.err(format!("`{name}` is already declared",)));
                }
                Ok(())
            }
        }
    }

    fn check_export(&mut self, export: &crate::ast::ExportStmt) -> SResult<()> {
        match &export.item {
            crate::ast::ExportItem::Fn(f) => {
                self.check_fn(f)?;
                self.exported.push(f.name.clone());
                if let Some(sym) = self.table.lookup(&f.name).cloned() {
                    self.exported_symbols.push((f.name.clone(), sym));
                }
                Ok(())
            }
            crate::ast::ExportItem::Let(b) => {
                self.check_statement(&Statement::Let(b.clone()))?;
                self.exported.push(b.name.clone());
                if let Some(sym) = self.table.lookup(&b.name).cloned() {
                    self.exported_symbols.push((b.name.clone(), sym));
                }
                Ok(())
            }
            crate::ast::ExportItem::Type(alias) => {
                self.check_type_alias(alias)?;
                self.exported.push(alias.name.clone());
                Ok(())
            }
            crate::ast::ExportItem::Enum(e) => {
                self.check_enum(e)?;
                self.exported.push(e.name.clone());
                self.exported_symbols.push((
                    e.name.clone(),
                    Symbol {
                        name: e.name.clone(),
                        type_: Type::Named(e.name.clone()),
                        kind: symbol_table::SymbolKind::Variable,
                        is_const: true,
                    },
                ));
                Ok(())
            }
            crate::ast::ExportItem::Default(item) => {
                // `export default fn main() {...}` exports the function under
                // its own name (the module system resolves `main` for `run`).
                if let crate::ast::ExportItem::Fn(f) = item.as_ref() {
                    self.check_fn(f)?;
                    if self.exported_default.is_some() {
                        return Err(self.err("only one `export default` is allowed per module"));
                    }
                    self.exported_default = Some(f.name.clone());
                    self.exported.push(f.name.clone());
                    if let Some(sym) = self.table.lookup(&f.name).cloned() {
                        self.exported_symbols.push((f.name.clone(), sym));
                    }
                    Ok(())
                } else {
                    Err(self.err("`export default` requires a function declaration"))
                }
            }
            crate::ast::ExportItem::Names(names) => {
                for name in names {
                    if self.table.lookup(name).is_none() {
                        return Err(self.err(format!(
                            "cannot export `{name}`: it is not declared in this module"
                        )));
                    }
                }
                for name in names {
                    if let Some(sym) = self.table.lookup(name).cloned() {
                        self.exported_symbols.push((name.clone(), sym));
                    }
                }
                self.exported.extend(names.iter().cloned());
                Ok(())
            }
        }
    }

    fn check_assign(&mut self, assign: &AssignStmt) -> SResult<()> {
        let target_type = self.assign_target_type(&assign.target)?;
        let value_type = self.check_expression(&assign.value)?;
        if !self.assignable(&value_type, &target_type) {
            return Err(self.err(format!(
                "cannot assign a value of type `{}` to `{}: {}`",
                value_type.name(),
                assign_target_name(&assign.target),
                target_type.name()
            )));
        }
        Ok(())
    }

    /// Type-check the left-hand side of an assignment and return the type its
    /// value must have.
    fn assign_target_type(&mut self, target: &AssignTarget) -> SResult<Type> {
        match target {
            AssignTarget::Name(name) => {
                let Some(sym) = self.table.lookup(name) else {
                    return Err(self.err(format!("undefined variable `{name}` cannot be assigned")));
                };
                if sym.is_const {
                    return Err(
                        self.err(format!("cannot assign to `{name}`: binding is immutable"))
                    );
                }
                match &sym.kind {
                    SymbolKind::Variable | SymbolKind::State => Ok(sym.type_.clone()),
                    SymbolKind::Store => Err(self.err(format!(
                        "cannot assign to `{name}`: store bindings are read-only"
                    ))),
                    SymbolKind::Function(_, _, _) => {
                        Err(self.err(format!("cannot assign to `{name}`: it is a function")))
                    }
                }
            }
            AssignTarget::Member(object, property) => {
                let object_type = self.check_expression(object)?;
                let resolved = self.resolve_alias(&object_type, 0);
                match resolved {
                    Type::Object | Type::Any | Type::Named(_) => Ok(Type::Any),
                    Type::ObjectType(fields) => fields
                        .iter()
                        .find(|(n, _)| n == property)
                        .map(|(_, t)| t.clone())
                        .ok_or_else(|| self.err(format!("type has no member `{property}`"))),
                    other => Err(self.err(format!(
                        "cannot assign member `{property}` of `{}`",
                        other.name()
                    ))),
                }
            }
            AssignTarget::Index(object, index) => {
                let object_type = self.check_expression(object)?;
                self.check_expression(index)?;
                let resolved = self.resolve_alias(&object_type, 0);
                match resolved {
                    Type::List(inner) => Ok(*inner),
                    Type::Object | Type::Any | Type::Named(_) => Ok(Type::Any),
                    other => {
                        Err(self.err(format!("cannot assign into `{}` by index", other.name())))
                    }
                }
            }
        }
    }

    /// `@State` / `@Store` / `@Effect` / `@Environment` are only valid at the
    /// top level of a function whose return type is `Component` (docs §12).
    fn require_component_top_level(&self, what: &str) -> SResult<()> {
        if self.component_depth == 0 {
            return Err(self.err(format!(
                "`{what}` may only be used at the top level of a function returning `Component`"
            )));
        }
        if self.block_depth > 0 {
            return Err(self.err(format!("`{what}` may not be used inside a nested block")));
        }
        Ok(())
    }

    /// Is this type `Component` (optionally wrapped in `async`)?
    fn is_component_type(&self, ty: &Type) -> bool {
        match self.resolve_alias(ty, 0) {
            Type::Named(n) => n == "Component",
            Type::Async(inner) => matches!(inner.as_ref(), Type::Named(n) if n == "Component"),
            _ => false,
        }
    }

    fn check_state(&mut self, binding: &LetBinding) -> SResult<()> {
        self.require_component_top_level("@State")?;
        if let Some(annotation) = &binding.type_annotation {
            self.check_type(annotation)?;
        }
        let value_type = match &binding.value {
            Some(value) => self.check_expression(value)?,
            None => binding.type_annotation.clone().unwrap_or(Type::Any),
        };
        if let Some(annotation) = &binding.type_annotation {
            let literal_ok = match &binding.value {
                Some(value) => self.literal_matches(value, annotation),
                None => false,
            };
            if !self.assignable(&value_type, annotation) && !literal_ok {
                return Err(self.err(format!(
                    "cannot bind a value of type `{}` to `@State {}: {}`",
                    value_type.name(),
                    binding.name,
                    annotation.name()
                )));
            }
        }
        let declared = self.table.declare(Symbol {
            name: binding.name.clone(),
            type_: binding.type_annotation.clone().unwrap_or(value_type),
            kind: SymbolKind::State,
            is_const: binding.is_const,
        });
        if !declared {
            return Err(self.err(format!(
                "binding `{}` is already declared in this scope",
                binding.name
            )));
        }
        Ok(())
    }

    fn check_store(&mut self, store: &crate::ast::StoreStmt) -> SResult<()> {
        self.require_component_top_level("@Store")?;
        self.check_expression(&store.value)?;
        match &store.pattern {
            crate::ast::BindingPattern::Ident(name) => {
                if !self.table.declare(Symbol {
                    name: name.clone(),
                    type_: Type::Any,
                    kind: SymbolKind::Store,
                    is_const: true,
                }) {
                    return Err(self.err(format!(
                        "binding `{name}` is already declared in this scope"
                    )));
                }
            }
            crate::ast::BindingPattern::Destructure(fields) => {
                for (name, alias) in fields {
                    let local = alias.clone().unwrap_or_else(|| name.clone());
                    if !self.table.declare(Symbol {
                        name: local.clone(),
                        type_: Type::Any,
                        kind: SymbolKind::Store,
                        is_const: true,
                    }) {
                        return Err(self.err(format!(
                            "binding `{local}` is already declared in this scope"
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    fn check_effect(&mut self, effect: &crate::ast::EffectStmt) -> SResult<()> {
        self.require_component_top_level("@Effect")?;
        let saved = self.in_effect;
        self.in_effect = true;
        let result = (|| {
            self.check_fn_expr(&effect.closure)?;
            if let Some(deps) = &effect.deps {
                for dep in deps {
                    self.check_expression(dep)?;
                }
            }
            Ok(())
        })();
        self.in_effect = saved;
        result
    }

    fn check_environment(&mut self, env: &crate::ast::EnvStmt) -> SResult<()> {
        self.require_component_top_level("@Environment")?;
        self.check_type(&env.type_)?;
        if !self.table.declare(Symbol {
            name: env.name.clone(),
            type_: env.type_.clone(),
            kind: SymbolKind::Store,
            is_const: true,
        }) {
            return Err(self.err(format!(
                "binding `{}` is already declared in this scope",
                env.name
            )));
        }
        Ok(())
    }

    fn check_component(&mut self, component: &crate::ast::ComponentStmt) -> SResult<()> {
        // Uppercase calls lower to UI components (`Name({ key: value })`);
        // props are not validated against a function signature (see
        // `component_call_props_are_loosely_typed`).
        for arg in &component.args {
            self.check_expression(&arg.value)?;
        }
        self.block_depth += 1;
        let mut result = Ok(());
        for child in &component.children {
            if let Err(e) = self.check_ui_element(child) {
                result = Err(e);
                break;
            }
        }
        self.block_depth -= 1;
        result
    }

    fn check_ui_element(&mut self, el: &crate::ast::UiElement) -> SResult<()> {
        match el {
            crate::ast::UiElement::Component(c) => self.check_component(c),
            crate::ast::UiElement::Text(_) => Ok(()),
            crate::ast::UiElement::Expr(e) => {
                let ty = self.check_expression(e)?;
                let allowed = match &ty {
                    Type::String | Type::Any => true,
                    other if self.is_component_type(other) => true,
                    // A list/optional child forwards ready-made children (e.g.
                    // a `children: list<Component>` parameter); only element
                    // types that can themselves be children make sense.
                    Type::Optional(inner) => self.is_component_type(inner),
                    Type::List(inner) => {
                        self.is_component_type(inner)
                            || matches!(inner.as_ref(), Type::String | Type::Any)
                    }
                    _ => false,
                };
                if !allowed {
                    return Err(self.err(format!(
                        "component children must be strings, components, or lists of components, found `{}`",
                        ty.name()
                    )));
                }
                Ok(())
            }
            crate::ast::UiElement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let cond = self.check_expression(condition)?;
                if !self.assignable(&cond, &Type::Boolean) {
                    return Err(self.err(format!(
                        "if condition must be a `boolean`, found `{}`",
                        cond.name()
                    )));
                }
                for e in then_branch {
                    self.check_ui_element(e)?;
                }
                if let Some(els) = else_branch {
                    for e in els {
                        self.check_ui_element(e)?;
                    }
                }
                Ok(())
            }
            crate::ast::UiElement::For {
                iter_var,
                iterable,
                body,
            } => {
                let iterable_type = self.check_expression(iterable)?;
                match iterable_type {
                    Type::List(_) | Type::Any => {}
                    other => {
                        return Err(self.err(format!(
                            "for loop must iterate over a `list`, found `{}`",
                            other.name()
                        )));
                    }
                }
                self.table.push_scope();
                self.table.declare(Symbol {
                    name: iter_var.clone(),
                    type_: Type::Any,
                    kind: SymbolKind::Variable,
                    is_const: false,
                });
                let mut result = Ok(());
                for e in body {
                    if let Err(err) = self.check_ui_element(e) {
                        result = Err(err);
                        break;
                    }
                }
                self.table.pop_scope();
                result
            }
            crate::ast::UiElement::Group(group) => {
                for e in group {
                    self.check_ui_element(e)?;
                }
                Ok(())
            }
        }
    }

    fn check_type_alias(&mut self, alias: &TypeAlias) -> SResult<()> {
        self.generics.extend(alias.type_params.iter().cloned());
        let result = self.check_type(&alias.type_);
        self.generics
            .truncate(self.generics.len().saturating_sub(alias.type_params.len()));
        result?;
        if self.type_table.contains_key(&alias.name) {
            return Err(self.err(format!("type `{}` is already defined", alias.name)));
        }
        self.type_table.insert(
            alias.name.clone(),
            TypeEntry {
                type_params: alias.type_params.clone(),
                kind: TypeEntryKind::Alias(alias.type_.clone()),
            },
        );
        Ok(())
    }

    fn check_enum(&mut self, e: &crate::ast::EnumDef) -> SResult<()> {
        if self.type_table.contains_key(&e.name) {
            return Err(self.err(format!("type `{}` is already defined", e.name)));
        }
        let mut seen = std::collections::HashSet::new();
        self.generics.extend(e.type_params.iter().cloned());
        for variant in &e.variants {
            if !seen.insert(variant.name.clone()) {
                self.generics
                    .truncate(self.generics.len().saturating_sub(e.type_params.len()));
                return Err(self.err(format!(
                    "enum `{}` has a duplicate member `{}`",
                    e.name, variant.name
                )));
            }
            if let Some(params) = &variant.payload
                && let Err(e2) = params.iter().try_for_each(|p| self.check_type(&p.type_))
            {
                self.generics
                    .truncate(self.generics.len().saturating_sub(e.type_params.len()));
                return Err(e2);
            }
        }
        self.generics
            .truncate(self.generics.len().saturating_sub(e.type_params.len()));
        self.type_table.insert(
            e.name.clone(),
            TypeEntry {
                type_params: e.type_params.clone(),
                kind: TypeEntryKind::Enum(e.variants.clone()),
            },
        );
        Ok(())
    }

    fn check_block(&mut self, block: &Block) -> SResult<()> {
        self.check_block_tail(block).map(|_| ())
    }

    /// Check an expression statement and infer its value type. Statement-position
    /// `if` compiles to a direct `if` statement, so `await` in its arms is fine
    /// (unlike a value-position `if`, which `check_expression` wraps in
    /// `no_await`).
    fn check_expr_stmt(&mut self, stmt: &crate::ast::ExprStmt) -> SResult<Type> {
        if let Expression::If(if_expr) = &stmt.expr {
            self.check_if(if_expr)
        } else {
            self.check_expression(&stmt.expr)
        }
    }

    /// Check a block and return the type of its trailing expression (the
    /// block's value), if any. Runs while the block scope is active. A
    /// trailing expression is the block's value, so it is checked via
    /// `check_expression` (a trailing `if`/`match` is therefore value
    /// position and `await` inside it is rejected).
    fn check_block_tail(&mut self, block: &Block) -> SResult<Option<Type>> {
        self.table.push_scope();
        self.block_depth += 1;
        let tail = match block.statements.last() {
            Some(Statement::Expr(e)) => {
                for statement in &block.statements[..block.statements.len() - 1] {
                    self.check_statement(statement)?;
                }
                // A trailing `if`/`match` here is still statement position
                // (codegen emits it via `if_stmt`, whose `await` is fine). Only
                // an enclosing value-position context (`check_expression` for
                // `If`/`Match`) wraps arms in `no_await`.
                Some(self.check_expr_stmt(e)?)
            }
            _ => {
                for statement in &block.statements {
                    self.check_statement(statement)?;
                }
                None
            }
        };
        self.block_depth -= 1;
        self.table.pop_scope();
        Ok(tail)
    }

    /// Check a function body block, then validate a trailing expression as its
    /// implicit return (docs §21.2): when the enclosing function declares a
    /// return type, a trailing expression statement is that function's value.
    /// Mirrors codegen's `fn_def`/`fn_expr` rule and must run while the body
    /// scope is still active.
    fn check_block_implicit(&mut self, block: &Block) -> SResult<()> {
        self.table.push_scope();
        if self.declared_return
            && let Some(Statement::Expr(last)) = block.statements.last()
        {
            let expected = self.current_return.clone().unwrap();
            let target = match &expected {
                Type::Async(inner) => inner.as_ref(),
                other => other,
            };
            for statement in &block.statements[..block.statements.len() - 1] {
                self.check_statement(statement)?;
            }
            if last.has_semicolon {
                // A trailing `expr;` is an ordinary statement, not an implicit
                // return (docs §21.2) → warn that its value is discarded.
                self.check_expr_stmt(last)?;
                self.warn(format!(
                    "ignored return value: trailing expression with `;` is not the function's return value (expected `{}`)",
                    expected.name()
                ));
            } else {
                // A trailing `expr` is the function's implicit return → value
                // position, so a trailing `if`/`match` rejects `await` inside.
                let value_type = self.check_expression(&last.expr)?;
                if !self.assignable(&value_type, target) {
                    return Err(self.err(format!(
                        "return type mismatch: expected `{}`, found `{}`",
                        expected.name(),
                        value_type.name()
                    )));
                }
            }
        } else {
            for statement in &block.statements {
                self.check_statement(statement)?;
            }
        }
        self.table.pop_scope();
        Ok(())
    }

    /// Check an expression and infer its type.
    fn check_expression(&mut self, expr: &Expression) -> SResult<Type> {
        self.current_span = expr.span().clone();
        match expr {
            Expression::Literal { value: lit, .. } => self.check_literal(lit),
            Expression::Identifier { name, .. } => {
                let sym = self.lookup_symbol(name)?;
                Ok(sym.type_.clone())
            }
            Expression::BinaryOp(bin) => self.check_binary(bin),
            Expression::Unary(un) => self.check_unary(un),
            Expression::Call(call) => self.check_call(call),
            Expression::EnumRef(r) => self.check_enum_ref(r.enum_name.clone(), &r.variant),
            Expression::If(if_expr) => self.no_await(|this| this.check_if(if_expr)),
            Expression::Ternary(tr) => self.check_ternary(tr),
            Expression::Match(m) => self.no_await(|this| this.check_match(m)),
            Expression::Member(m) => self.check_member(m),
            Expression::Index(idx) => self.check_index(idx),
            Expression::Nullish(n) => self.check_nullish(n),
            Expression::Range(r) => {
                self.check_expression(&r.start)?;
                self.check_expression(&r.end)?;
                Ok(Type::List(Box::new(Type::Number)))
            }
            Expression::Await { expr: operand, .. } => self.check_await(operand),
            Expression::FnExpr(f) => self.check_fn_expr(f),
            Expression::Binding { name, .. } => {
                let sym = self.lookup_symbol(name)?;
                match sym.kind {
                    SymbolKind::State | SymbolKind::Store => Ok(sym.type_.clone()),
                    _ => Err(self.err(format!(
                        "`$` binding requires a `@State` or `@Store` variable, but `{name}` is not"
                    ))),
                }
            }
            Expression::Spread { .. } => {
                Err(self.err("`...` spread is only allowed inside list or object literals"))
            }
            Expression::CallValue(cv) => {
                let callee_type = self.check_expression(&cv.callee)?;
                let callee_type = self.resolve_alias(&callee_type, 0);
                match callee_type {
                    Type::FnSig { params, ret } => {
                        self.check_fn_value_args(&params, ret, &cv.arguments)
                    }
                    Type::Any => {
                        for a in &cv.arguments {
                            self.check_expression(&a.value)?;
                        }
                        Ok(Type::Any)
                    }
                    other => Err(self.err(format!(
                        "expression of type `{}` is not callable",
                        other.name()
                    ))),
                }
            }
        }
    }

    /// Check a literal, recursively validating its contents (list items, object
    /// field values, spread operands) and inferring its type.
    fn check_literal(&mut self, lit: &Literal) -> SResult<Type> {
        match lit {
            Literal::List(items) => {
                if items.is_empty() {
                    return Ok(Type::List(Box::new(Type::Any)));
                }
                let mut element = Type::Any;
                let mut first = true;
                for item in items {
                    let item_type = match item {
                        Expression::Spread { expr: spread, .. } => {
                            let spread_type = self.check_expression(spread)?;
                            match spread_type {
                                Type::List(inner) => *inner,
                                Type::Any => Type::Any,
                                other => {
                                    return Err(self.err(format!(
                                        "spread operand must be a list, got `{}`",
                                        other.name()
                                    )));
                                }
                            }
                        }
                        other => self.check_expression(other)?,
                    };
                    if first {
                        element = item_type;
                        first = false;
                    }
                }
                Ok(Type::List(Box::new(element)))
            }
            Literal::Object(fields) => {
                for field in fields {
                    match field {
                        ObjectField::Field { value, .. } => {
                            self.check_expression(value)?;
                        }
                        ObjectField::Spread { value } => {
                            let spread_type = self.check_expression(value)?;
                            if !matches!(spread_type, Type::Object | Type::Any) {
                                return Err(self.err(format!(
                                    "spread operand must be an object, got `{}`",
                                    spread_type.name()
                                )));
                            }
                        }
                    }
                }
                Ok(Type::Object)
            }
            other => Ok(literal_type(other)),
        }
    }

    /// Run `f` with `no_await_depth` incremented, so `await` inside `if`/
    /// `match` arms is rejected (their arms compile to a plain function).
    fn no_await<T>(&mut self, f: impl FnOnce(&mut Self) -> SResult<T>) -> SResult<T> {
        self.no_await_depth += 1;
        let result = f(self);
        self.no_await_depth -= 1;
        result
    }

    fn check_await(&mut self, operand: &Expression) -> SResult<Type> {
        if self.no_await_depth > 0 {
            return Err(self.err(
                "`await` cannot be used inside an `if`/`match` expression; assign its value first with `let`",
            ));
        }
        if self.async_depth == 0 {
            return Err(self.err("`await` may only be used inside an `async` function"));
        }
        let inner = self.check_expression(operand)?;
        match inner {
            Type::Async(inner) => Ok(*inner),
            Type::Any => Ok(Type::Any),
            other => Err(self.err(format!(
                "cannot await a non-promise value of type `{}`",
                other.name()
            ))),
        }
    }

    fn check_unary(&mut self, un: &crate::ast::UnaryOp) -> SResult<Type> {
        let operand = self.check_expression(&un.operand)?;
        match un.operator {
            crate::ast::UnaryOperator::Not => {
                if self.assignable(&operand, &Type::Boolean) {
                    Ok(Type::Boolean)
                } else {
                    Err(self.err(format!(
                        "unary `!` requires a `boolean` operand, found `{}`",
                        operand.name()
                    )))
                }
            }
            crate::ast::UnaryOperator::Neg => {
                if self.assignable(&operand, &Type::Number) {
                    Ok(Type::Number)
                } else {
                    Err(self.err(format!(
                        "unary `-` requires a `number` operand, found `{}`",
                        operand.name()
                    )))
                }
            }
        }
    }

    fn check_ternary(&mut self, tr: &crate::ast::TernaryExpr) -> SResult<Type> {
        let condition = self.check_expression(&tr.condition)?;
        if !self.assignable(&condition, &Type::Boolean) {
            return Err(self.err(format!(
                "ternary condition must be a `boolean`, found `{}`",
                condition.name()
            )));
        }
        let then_type = self.check_expression(&tr.then_value)?;
        let else_type = self.check_expression(&tr.else_value)?;
        if self.assignable(&then_type, &else_type) {
            Ok(else_type)
        } else if self.assignable(&else_type, &then_type) {
            Ok(then_type)
        } else {
            Ok(Type::Any)
        }
    }

    fn check_member(&mut self, m: &crate::ast::MemberAccess) -> SResult<Type> {
        let object = self.check_expression(&m.object)?;
        let resolved = self.resolve_alias(&object, 0);
        let inner = match resolved {
            Type::Optional(inner) if m.optional => *inner,
            Type::Null if m.optional => return Ok(Type::Any),
            Type::Optional(_) => {
                return Err(self.err(format!(
                    "cannot access member of optional type `{}` without `?.`",
                    object.name()
                )));
            }
            other => other,
        };
        self.member_field_type(&inner, &m.property)
    }

    /// Look up a field type on an object-like type (`ObjectType`, the loose
    /// `object`, or `Any`).
    fn member_field_type(&self, ty: &Type, property: &str) -> SResult<Type> {
        match ty {
            Type::ObjectType(fields) => fields
                .iter()
                .find(|(n, _)| n == property)
                .map(|(_, t)| t.clone())
                .ok_or_else(|| self.err(format!("type has no member `{property}`"))),
            Type::Object | Type::Any | Type::Named(_) => Ok(Type::Any),
            other => Err(self.err(format!(
                "cannot access member of `{}` (type `{}`)",
                property,
                other.name()
            ))),
        }
    }

    fn check_index(&mut self, idx: &crate::ast::IndexExpr) -> SResult<Type> {
        let object = self.check_expression(&idx.object)?;
        self.check_expression(&idx.index)?;
        let resolved = self.resolve_alias(&object, 0);
        match resolved {
            Type::List(inner) => Ok(*inner),
            Type::Object | Type::Any | Type::String | Type::Null => Ok(Type::Any),
            Type::Named(_) => Ok(Type::Any),
            other => Err(self.err(format!("cannot index into `{}`", other.name()))),
        }
    }

    fn check_nullish(&mut self, n: &crate::ast::NullishExpr) -> SResult<Type> {
        let left = self.check_expression(&n.left)?;
        let right = self.check_expression(&n.right)?;
        if let Type::Optional(inner) = left {
            Ok(*inner)
        } else if matches!(left, Type::Null) || matches!(left, Type::Any) {
            Ok(right)
        } else {
            Ok(left)
        }
    }

    fn check_match(&mut self, m: &crate::ast::MatchExpr) -> SResult<Type> {
        let value_type = self.check_expression(&m.value)?;
        let mut arm_types = Vec::new();
        for arm in &m.arms {
            match &arm.pattern {
                crate::ast::MatchPattern::Wildcard => {}
                crate::ast::MatchPattern::Literal(lit) => {
                    let lit_type = literal_type(lit);
                    if !self.assignable(&lit_type, &value_type) {
                        return Err(self.err(format!(
                            "match arm pattern `{}` does not match value of type `{}`",
                            pattern_name(arm),
                            value_type.name()
                        )));
                    }
                }
                crate::ast::MatchPattern::Enum(r) => {
                    let enum_type = self.check_enum_ref(r.enum_name.clone(), &r.variant)?;
                    if !self.assignable(&value_type, &enum_type) {
                        return Err(self.err(format!(
                            "match arm pattern `{}` does not match value of type `{}` (expected `{}`)",
                            pattern_name(arm),
                            value_type.name(),
                            enum_type.name()
                        )));
                    }
                }
                crate::ast::MatchPattern::EnumPayload {
                    enum_name,
                    variant,
                    bindings,
                    span,
                } => {
                    let entry = self
                        .type_entry(enum_name)
                        .ok_or_else(|| self.err(format!("unknown enum `{enum_name}`")))?
                        .clone();
                    let enum_type = Type::Named(enum_name.clone());
                    if !self.assignable(&value_type, &enum_type) {
                        return Err(self.err(format!(
                            "match arm pattern `{}` does not match value of type `{}` (expected `{}`)",
                            pattern_name(arm),
                            value_type.name(),
                            enum_type.name()
                        )));
                    }
                    let v = entry
                        .kind
                        .variants()
                        .and_then(|vs| vs.iter().find(|v| v.name == *variant))
                        .ok_or_else(|| {
                            self.err(format!("enum `{enum_name}` has no member `{variant}`"))
                        })?
                        .clone();
                    match &v.payload {
                        Some(params) => {
                            if params.len() != bindings.len() {
                                return Err(self
                                    .err(format!(
                                        "pattern binds {} values but `{enum_name}::{variant}` carries {}",
                                        bindings.len(),
                                        params.len()
                                    ))
                                    .at(span.clone()));
                            }
                            self.generics.extend(entry.type_params.iter().cloned());
                            self.table.push_scope();
                            for (param, binding) in params.iter().zip(bindings.iter()) {
                                if binding == "_" {
                                    continue;
                                }
                                self.table.declare(Symbol {
                                    name: binding.clone(),
                                    type_: self.resolve_alias(&param.type_, 0),
                                    kind: SymbolKind::Variable,
                                    is_const: true,
                                });
                            }
                            let t = self.check_expression(&arm.value)?;
                            self.table.pop_scope();
                            self.generics.truncate(
                                self.generics.len().saturating_sub(entry.type_params.len()),
                            );
                            arm_types.push(t);
                            continue;
                        }
                        None => {
                            return Err(self.err(format!(
                                "enum member `{enum_name}::{variant}` has no payload to bind"
                            )));
                        }
                    }
                }
            }
            arm_types.push(self.check_expression(&arm.value)?);
        }
        if arm_types.is_empty() {
            return Ok(Type::Any);
        }
        let first = &arm_types[0];
        if arm_types.iter().all(|t| t == first) {
            return Ok(first.clone());
        }
        // Arms must be mutually assignable; the first arm's type is the match
        // type (mirrors `check_if`'s branch typing).
        for t in &arm_types[1..] {
            if !self.assignable(t, first) && !self.assignable(first, t) {
                return Err(self.err(format!(
                    "match arms have incompatible types `{}` and `{}`",
                    first.name(),
                    t.name()
                )));
            }
        }
        Ok(first.clone())
    }

    fn check_binary(&mut self, bin: &crate::ast::BinaryOp) -> SResult<Type> {
        let left = self.check_expression(&bin.left)?;
        let right = self.check_expression(&bin.right)?;
        match bin.operator {
            BinaryOperator::Add => {
                let l = self.resolve_alias(&left, 0);
                let r = self.resolve_alias(&right, 0);
                if matches!(l, Type::Number) && matches!(r, Type::Number) {
                    Ok(Type::Number)
                } else if self.is_stringish(&l) && self.is_stringish(&r) {
                    Ok(Type::String)
                } else if matches!(l, Type::List(_)) && matches!(r, Type::List(_)) {
                    Ok(Type::List(Box::new(self.join_list(&l, &r))))
                } else if matches!(l, Type::Any) || matches!(r, Type::Any) {
                    Ok(Type::Any)
                } else {
                    Err(self
                        .err(format!(
                            "cannot apply `+` to `{}` and `{}`",
                            left.name(),
                            right.name()
                        ))
                        .at(bin.span.clone()))
                }
            }
            BinaryOperator::Sub | BinaryOperator::Mul | BinaryOperator::Div => {
                if self.assignable(&left, &Type::Number) && self.assignable(&right, &Type::Number) {
                    Ok(Type::Number)
                } else {
                    Err(self
                        .err(format!(
                            "cannot apply `{}` to `{}` and `{}`",
                            bin.operator.symbol(),
                            left.name(),
                            right.name()
                        ))
                        .at(bin.span.clone()))
                }
            }
            BinaryOperator::Eq | BinaryOperator::Neq => {
                let comparable = self.assignable(&left, &right) || self.assignable(&right, &left);
                if comparable {
                    Ok(Type::Boolean)
                } else {
                    Err(self
                        .err(format!(
                            "cannot compare `{}` with `{}`",
                            left.name(),
                            right.name()
                        ))
                        .at(bin.span.clone()))
                }
            }
            BinaryOperator::Lt | BinaryOperator::Gt | BinaryOperator::Lte | BinaryOperator::Gte => {
                let comparable = self.assignable(&left, &right) && self.assignable(&right, &left);
                if comparable {
                    Ok(Type::Boolean)
                } else {
                    Err(self
                        .err(format!(
                            "cannot compare `{}` with `{}`",
                            left.name(),
                            right.name()
                        ))
                        .at(bin.span.clone()))
                }
            }
            BinaryOperator::And | BinaryOperator::Or => {
                if self.assignable(&left, &Type::Boolean) && self.assignable(&right, &Type::Boolean)
                {
                    Ok(Type::Boolean)
                } else {
                    Err(self
                        .err(format!(
                            "cannot apply `{}` to `{}` and `{}`",
                            bin.operator.symbol(),
                            left.name(),
                            right.name()
                        ))
                        .at(bin.span.clone()))
                }
            }
        }
    }

    fn is_stringish(&self, ty: &Type) -> bool {
        matches!(ty, Type::String | Type::Literal(_))
    }

    fn join_list(&self, l: &Type, r: &Type) -> Type {
        let (Type::List(a), Type::List(b)) = (l, r) else {
            return Type::Any;
        };
        if self.assignable(a, b) {
            (**b).clone()
        } else if self.assignable(b, a) {
            (**a).clone()
        } else {
            Type::Any
        }
    }

    fn check_call(&mut self, call: &crate::ast::Call) -> SResult<Type> {
        if call.is_enum() {
            let (enum_name, variant) = call.enum_parts().unwrap();
            let args: Vec<&Expression> = call.arguments.iter().map(|a| &a.value).collect();
            return self.check_enum_call(enum_name.to_string(), variant.to_string(), &args);
        }
        if let Some(object) = &call.object {
            // Method call `obj.method(args)`.
            self.check_expression(object)?;
            for arg in &call.arguments {
                self.check_expression(&arg.value)?;
            }
            return Ok(Type::Any);
        }
        // A user-declared `str`/`print` shadows the builtin of the same name
        // (the builtins only apply when no user symbol matches).
        if let Some(sym) = self.table.lookup(&call.callee).cloned() {
            return self.check_call_symbol(call, &sym);
        }
        if call.callee == "print" {
            for arg in &call.arguments {
                self.check_expression(&arg.value)?;
            }
            return Ok(Type::Any);
        }
        if call.callee == "str" {
            if call.arguments.len() != 1 {
                return Err(self
                    .err(format!(
                        "`str` expects exactly one argument, got {}",
                        call.arguments.len()
                    ))
                    .at(call.span.clone()));
            }
            self.check_expression(&call.arguments[0].value)?;
            return Ok(Type::String);
        }
        let Some(sym) = self.table.lookup(&call.callee).cloned() else {
            return Err(self.err(format!("unknown function `{}`", call.callee)));
        };
        self.check_call_symbol(call, &sym)
    }

    /// Shared call validation against a resolved symbol: effect-scope guard,
    /// then per-symbol-kind argument/arity/type checks.
    fn check_call_symbol(&mut self, call: &Call, sym: &Symbol) -> SResult<Type> {
        if self.in_effect
            && self
                .render_locals
                .last()
                .is_some_and(|locals| locals.contains(&call.callee))
        {
            return Err(self.err(format!(
                "`@Effect` closures cannot reference `{}`: it is declared inside the component body and is out of scope when the effect runs",
                call.callee
            )));
        }
        match &sym.kind {
            SymbolKind::Function(type_params, params, return_type) => {
                // An unseeded import (no module graph) is opaque: accept any
                // argument list and return `any`. Local functions never reach
                // here, even when un-annotated (zero params, no return type).
                if self.opaque.contains(&call.callee) {
                    for arg in &call.arguments {
                        self.check_expression(&arg.value)?;
                    }
                    return Ok(Type::Any);
                }
                let named = call
                    .arguments
                    .iter()
                    .filter(|a| a.name.is_some())
                    .collect::<Vec<_>>();
                let positional = call
                    .arguments
                    .iter()
                    .filter(|a| a.name.is_none())
                    .collect::<Vec<_>>();
                let all_named = named.len() == call.arguments.len() && !call.arguments.is_empty();
                let type_params = type_params.clone();
                let params = params.clone();
                let return_type = return_type.clone();
                self.generics.extend(type_params.iter().cloned());
                let result = (|| {
                    if all_named {
                        // Named arguments must cover every parameter (defaults
                        // may be omitted) exactly once.
                        let mut seen = std::collections::HashSet::new();
                        for arg in &named {
                            let name = arg.name.as_ref().unwrap();
                            let Some(param) = params.iter().find(|p| &p.name == name) else {
                                return Err(self.err(format!(
                                    "function `{}` has no parameter `{name}`",
                                    call.callee
                                )));
                            };
                            if !seen.insert(name.clone()) {
                                return Err(self.err(format!(
                                    "argument `{name}` to `{}` is provided twice",
                                    call.callee
                                )));
                            }
                            let arg_type = self.check_expression(&arg.value)?;
                            let expected = param.type_annotation.as_ref().unwrap_or(&Type::Any);
                            if !self.assignable(&arg_type, expected)
                                && !self.literal_matches(&arg.value, expected)
                            {
                                return Err(self.err(format!(
                                    "argument to `{}` must be `{}`, found `{}`",
                                    call.callee,
                                    expected.name(),
                                    arg_type.name()
                                )));
                            }
                        }
                        let required = params
                            .iter()
                            .filter(|p| p.default.is_none() && !param_optional(p))
                            .map(|p| p.name.clone())
                            .collect::<Vec<_>>();
                        let missing = required.iter().find(|name| !seen.contains(*name)).cloned();
                        if let Some(name) = missing {
                            return Err(self.err(format!(
                                "function `{}` is missing required argument `{name}`",
                                call.callee
                            )));
                        }
                        // Call-site inference for generic functions, mirroring
                        // the positional path: bind `T` from the named args
                        // (in parameter order, using defaults when omitted).
                        let mut resolved = return_type;
                        if !type_params.is_empty() && !params.is_empty() {
                            let mut positional_types = Vec::with_capacity(params.len());
                            let mut unbounded = false;
                            for param in &params {
                                let arg = call
                                    .arguments
                                    .iter()
                                    .find(|a| a.name.as_ref() == Some(&param.name));
                                if let Some(arg) = arg {
                                    let arg_type = self.check_expression(&arg.value)?;
                                    positional_types.push(arg_type);
                                } else if let Some(default) = &param.default {
                                    let default_type = self.check_expression(default)?;
                                    positional_types.push(default_type);
                                } else {
                                    // A required parameter is absent; the call
                                    // is already rejected by the arity check
                                    // that ran above, so skip inference.
                                    unbounded = true;
                                    break;
                                }
                            }
                            if !unbounded {
                                let param_types = params
                                    .iter()
                                    .map(|p| p.type_annotation.clone().unwrap_or(Type::Any))
                                    .collect::<Vec<_>>();
                                let bindings = infer_type_bindings(
                                    &type_params,
                                    &param_types,
                                    &positional_types,
                                );
                                resolved = substitute_type(&resolved, &bindings);
                            }
                        }
                        Ok(resolved)
                    } else {
                        if !named.is_empty() {
                            // Codegen emits argument lists in source order for
                            // non-all-named calls, so a positional/named mix
                            // would silently land in the wrong parameter slot.
                            return Err(self.err(format!(
                                "call to `{}` cannot mix positional and named arguments",
                                call.callee
                            )));
                        }
                        let expected = params.len();
                        let required = params
                            .iter()
                            .filter(|p| p.default.is_none() && !param_optional(p))
                            .count();
                        let actual = positional.len();
                        if actual < required || actual > expected {
                            let range = if required == expected {
                                format!("{expected}")
                            } else {
                                format!("{required} to {expected}")
                            };
                            return Err(self.err(format!(
                                "function `{}` expects {range} argument(s), but {actual} were provided",
                                call.callee
                            )));
                        }
                        let mut positional_types = Vec::with_capacity(positional.len());
                        for (arg, param) in positional.iter().zip(params.iter()) {
                            let arg_type = self.check_expression(&arg.value)?;
                            positional_types.push(arg_type.clone());
                            let expected = param.type_annotation.as_ref().unwrap_or(&Type::Any);
                            if !self.assignable(&arg_type, expected)
                                && !self.literal_matches(&arg.value, expected)
                            {
                                return Err(self.err(format!(
                                    "argument to `{}` must be `{}`, found `{}`",
                                    call.callee,
                                    expected.name(),
                                    arg_type.name()
                                )));
                            }
                        }
                        // Call-site inference for generic functions
                        // (`first([1, 2])` binds `T = number`).
                        let mut resolved = return_type;
                        if !type_params.is_empty() {
                            let param_types = params
                                .iter()
                                .map(|p| p.type_annotation.clone().unwrap_or(Type::Any))
                                .collect::<Vec<_>>();
                            let bindings =
                                infer_type_bindings(&type_params, &param_types, &positional_types);
                            resolved = substitute_type(&resolved, &bindings);
                        }
                        Ok(resolved)
                    }
                })();
                self.generics
                    .truncate(self.generics.len().saturating_sub(type_params.len()));
                result
            }
            SymbolKind::Variable => {
                // A function value: `let f = fn() {...}; f(x)`. The type may be
                // a `type Handler = fn(...)` alias, so resolve it first.
                let resolved = self.resolve_alias(&sym.type_, 0);
                let sig = match &resolved {
                    Type::FnSig { params, ret } => Some((params.clone(), ret.clone())),
                    _ => None,
                };
                let Some((params, ret)) = sig else {
                    return Err(self.err(format!("`{}` is not a function", call.callee)));
                };
                self.check_fn_value_args(&params, ret, &call.arguments)
            }
            SymbolKind::State | SymbolKind::Store => {
                Err(self.err(format!("`{}` is not a function", call.callee)))
            }
        }
    }

    /// Validate a call against a function-value signature (`Type::FnSig`):
    /// positional arguments only, exact arity, per-parameter type checks.
    fn check_fn_value_args(
        &mut self,
        params: &[Type],
        ret: Option<Box<Type>>,
        arguments: &[CallArg],
    ) -> SResult<Type> {
        if arguments.iter().any(|a| a.name.is_some()) {
            return Err(self.err("named arguments are not supported when calling a function value"));
        }
        let expected = params.len();
        let actual = arguments.len();
        if actual != expected {
            return Err(self.err(format!(
                "function values expect exactly {expected} argument(s), but {actual} were provided",
            )));
        }
        for (arg, param) in arguments.iter().zip(params.iter()) {
            let arg_type = self.check_expression(&arg.value)?;
            if !self.assignable(&arg_type, param) {
                return Err(self.err(format!(
                    "argument to function value must be `{}`, found `{}`",
                    param.name(),
                    arg_type.name()
                )));
            }
        }
        Ok(ret.map(|r| *r).unwrap_or(Type::Any))
    }

    fn check_enum_ref(&self, enum_name: String, variant: &str) -> SResult<Type> {
        let Some(entry) = self.type_entry(&enum_name) else {
            return Err(self.err(format!("unknown enum `{enum_name}`")));
        };
        match &entry.kind {
            TypeEntryKind::Enum(variants) => {
                if !variants.iter().any(|v| v.name == variant) {
                    return Err(self.err(format!("enum `{enum_name}` has no member `{variant}`")));
                }
                Ok(Type::Named(enum_name))
            }
            TypeEntryKind::Alias(_) => Err(self.err(format!("`{enum_name}` is not an enum"))),
        }
    }

    fn check_enum_call(
        &mut self,
        enum_name: String,
        variant: String,
        arguments: &[&Expression],
    ) -> SResult<Type> {
        let Some(entry) = self.type_entry(&enum_name) else {
            return Err(self.err(format!("unknown enum `{enum_name}`")));
        };
        let type_params = entry.type_params.clone();
        let TypeEntryKind::Enum(variants) = &entry.kind else {
            return Err(self.err(format!("`{enum_name}` is not an enum")));
        };
        let Some(v) = variants.iter().find(|v| v.name == variant) else {
            return Err(self.err(format!("enum `{enum_name}` has no member `{variant}`")));
        };
        let payload = v.payload.clone();
        self.generics.extend(type_params.clone());
        let result = match payload {
            Some(params) => {
                if arguments.len() != params.len() {
                    Err(self.err(format!(
                        "enum member `{enum_name}::{variant}` expects {} argument(s), got {}",
                        params.len(),
                        arguments.len()
                    )))
                } else {
                    let mut errs = Vec::new();
                    for (arg, param) in arguments.iter().zip(params.iter()) {
                        match self.check_expression(arg) {
                            Ok(arg_type) => {
                                if !self.assignable(&arg_type, &param.type_) {
                                    errs.push(self.err(format!(
                                        "argument to `{enum_name}::{variant}` must be `{}`, found `{}`",
                                        param.type_.name(),
                                        arg_type.name()
                                    )));
                                }
                            }
                            Err(e) => errs.push(e),
                        }
                    }
                    if let Some(e) = errs.into_iter().next() {
                        Err(e)
                    } else {
                        Ok(Type::Named(enum_name))
                    }
                }
            }
            None => {
                if !arguments.is_empty() {
                    Err(self.err(format!(
                        "enum member `{enum_name}::{variant}` takes no payload"
                    )))
                } else {
                    Ok(Type::Named(enum_name))
                }
            }
        };
        match result {
            Ok(t) => {
                self.generics
                    .truncate(self.generics.len().saturating_sub(type_params.len()));
                Ok(t)
            }
            Err(e) => {
                self.generics
                    .truncate(self.generics.len().saturating_sub(type_params.len()));
                Err(e)
            }
        }
    }

    fn check_if(&mut self, if_expr: &IfExpr) -> SResult<Type> {
        let condition = self.check_expression(&if_expr.condition)?;
        if !self.assignable(&condition, &Type::Boolean) {
            return Err(self.err(format!(
                "if condition must be a `boolean`, found `{}`",
                condition.name()
            )));
        }
        let then_type = self.check_block_tail(&if_expr.then_branch)?;
        let else_type = match &if_expr.else_branch {
            Some(branch) => self.check_block_tail(branch)?,
            None => None,
        };
        // An `if` in value position takes the then-branch tail type; when both
        // branches have values they must be compatible (Rust/Swift style).
        match (then_type, else_type) {
            (Some(then_t), Some(else_t))
                if !self.assignable(&else_t, &then_t) && !self.assignable(&then_t, &else_t) =>
            {
                Err(self.err(format!(
                    "if branches have incompatible types `{}` and `{}`",
                    then_t.name(),
                    else_t.name()
                )))
            }
            (Some(then_t), _) => Ok(then_t),
            _ => Ok(Type::Any),
        }
    }

    /// Validate that every named type referenced by `ty` is defined.
    fn check_type(&self, ty: &Type) -> SResult<()> {
        match ty {
            Type::Named(name) => {
                if name == "Component"
                    || self.generics.contains(name)
                    || self.type_table.contains_key(name)
                    || self.imported_types.contains_key(name)
                {
                    Ok(())
                } else {
                    Err(self.err(format!("unknown type `{name}`")))
                }
            }
            Type::List(inner) | Type::Optional(inner) => self.check_type(inner),
            Type::Union(parts) | Type::Intersection(parts) => {
                for p in parts {
                    self.check_type(p)?;
                }
                Ok(())
            }
            Type::ObjectType(fields) => {
                for (_, t) in fields {
                    self.check_type(t)?;
                }
                Ok(())
            }
            Type::Async(inner) => self.check_type(inner),
            Type::FnSig { params, ret } => {
                for p in params {
                    self.check_type(p)?;
                }
                if let Some(r) = ret {
                    self.check_type(r)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Resolve a `Named` type to its underlying type. Generic type parameters
    /// erase to `Any`; enums are returned unchanged. A depth cap guards against
    /// self-referential aliases.
    fn resolve_alias(&self, ty: &Type, depth: usize) -> Type {
        if depth > 32 {
            return ty.clone();
        }
        match ty {
            Type::Named(name) => {
                if self.generics.contains(name) {
                    Type::Any
                } else {
                    match self.type_entry(name).map(|e| &e.kind) {
                        Some(TypeEntryKind::Alias(inner)) => self.resolve_alias(inner, depth + 1),
                        _ => ty.clone(),
                    }
                }
            }
            _ => ty.clone(),
        }
    }

    /// Look up a type by name, consulting both locally-declared types and
    /// types imported from other modules.
    fn type_entry(&self, name: &str) -> Option<&TypeEntry> {
        self.type_table
            .get(name)
            .or_else(|| self.imported_types.get(name))
    }

    /// Is a value of type `from` acceptable where `to` is expected?
    fn assignable(&self, from: &Type, to: &Type) -> bool {
        let from = self.resolve_alias(from, 0);
        let to = self.resolve_alias(to, 0);
        if matches!(from, Type::Any) || matches!(to, Type::Any) {
            return true;
        }
        if from == to {
            return true;
        }
        match (&from, &to) {
            (Type::Null, Type::Optional(_)) => true,
            (inner, Type::Optional(expected)) => {
                self.assignable(inner, expected) || matches!(inner, Type::Null)
            }
            // Loose typing (docs §10): an optional cascades to its inner type,
            // mirroring how `T?` is a shorthand the checker treats permissively.
            (Type::Optional(inner), expected) => self.assignable(inner, expected),
            (Type::Union(parts), expected) => parts.iter().all(|p| self.assignable(p, expected)),
            (actual, Type::Union(parts)) => parts.iter().any(|p| self.assignable(actual, p)),
            (Type::Intersection(parts), expected) => {
                parts.iter().all(|p| self.assignable(p, expected))
            }
            (actual, Type::Intersection(parts)) => parts.iter().all(|p| self.assignable(actual, p)),
            (Type::List(a), Type::List(b)) => self.assignable(a, b),
            (Type::Object, Type::ObjectType(_)) => true,
            (Type::ObjectType(_), Type::Object) => true,
            (Type::ObjectType(from_fields), Type::ObjectType(to_fields)) => {
                // Structural typing: every field the target requires must be
                // present in the source with a compatible type. Extra source
                // fields are allowed (`{ a, b }` is assignable to `{ a }`).
                to_fields.iter().all(|(to_name, to_ty)| {
                    from_fields
                        .iter()
                        .find(|(from_name, _)| from_name == to_name)
                        .is_some_and(|(_, from_ty)| self.assignable(from_ty, to_ty))
                })
            }
            (Type::Literal(_), Type::String) => true,
            (Type::Async(a), Type::Async(b)) => self.assignable(a, b),
            (Type::Async(a), expected) => {
                matches!(a.as_ref(), Type::Any) || self.assignable(a, expected)
            }
            (actual, Type::Async(b)) => {
                matches!(b.as_ref(), Type::Any) || self.assignable(actual, b)
            }
            (
                Type::FnSig {
                    params: ap,
                    ret: ar,
                },
                Type::FnSig {
                    params: bp,
                    ret: br,
                },
            ) => {
                ap.len() == bp.len()
                    && ap.iter().zip(bp).all(|(a, b)| self.assignable(a, b))
                    && match (ar, br) {
                        (None, None) => true,
                        (Some(a), Some(b)) => self.assignable(a, b),
                        (Some(_), None) => true,
                        (None, Some(_)) => false,
                    }
            }
            _ => false,
        }
    }

    /// Value-aware check for string-literal types: does a direct literal
    /// expression belong to the set of string-literal types in `annotation`?
    fn literal_matches(&self, value: &Expression, annotation: &Type) -> bool {
        match annotation {
            Type::Optional(inner) => match value {
                Expression::Literal {
                    value: Literal::Null,
                    ..
                } => true,
                _ => self.literal_matches(value, inner),
            },
            Type::Union(parts) => parts.iter().any(|p| self.literal_matches(value, p)),
            Type::Named(_) => {
                let resolved = self.resolve_alias(annotation, 0);
                if resolved != *annotation {
                    self.literal_matches(value, &resolved)
                } else {
                    false
                }
            }
            Type::Literal(expected) => match value {
                Expression::Literal {
                    value: Literal::String(s),
                    ..
                } => s == expected,
                _ => false,
            },
            _ => false,
        }
    }
}

/// True when a parameter is declared optional (`name: T?`), meaning callers may
/// omit its argument (docs §6 / §15).
fn param_optional(param: &crate::ast::Param) -> bool {
    matches!(param.type_annotation, Some(Type::Optional(_)))
}

/// Infer a function's type-parameter bindings by unifying each declared
/// parameter type against its (positionally aligned) argument type, e.g.
/// `first([1, 2])` binds `T = number` when the parameter is `list<T>`.
fn infer_type_bindings(
    type_params: &[String],
    param_types: &[Type],
    arg_types: &[Type],
) -> std::collections::HashMap<String, Type> {
    let mut bindings = std::collections::HashMap::new();
    for (param_ty, arg_ty) in param_types.iter().zip(arg_types.iter()) {
        unify_param(param_ty, arg_ty, type_params, &mut bindings);
    }
    bindings
}

fn unify_param(
    expected: &Type,
    actual: &Type,
    type_params: &[String],
    bindings: &mut std::collections::HashMap<String, Type>,
) {
    match expected {
        Type::Named(name) if type_params.contains(name) => {
            bindings
                .entry(name.clone())
                .or_insert_with(|| actual.clone());
        }
        Type::List(inner) => {
            if let Type::List(element) = actual {
                unify_param(inner, element, type_params, bindings);
            }
        }
        Type::Optional(inner) => match actual {
            Type::Optional(a) => unify_param(inner, a, type_params, bindings),
            Type::Null => {}
            other => unify_param(inner, other, type_params, bindings),
        },
        Type::Union(parts) => {
            for p in parts {
                unify_param(p, actual, type_params, bindings);
            }
        }
        Type::Intersection(parts) => {
            for p in parts {
                unify_param(p, actual, type_params, bindings);
            }
        }
        Type::Async(inner) => {
            if let Type::Async(a) = actual {
                unify_param(inner, a, type_params, bindings);
            }
        }
        Type::FnSig { params, ret } => {
            if let Type::FnSig {
                params: actual_params,
                ret: actual_ret,
            } = actual
            {
                for (p, a) in params.iter().zip(actual_params.iter()) {
                    unify_param(p, a, type_params, bindings);
                }
                if let (Some(pe), Some(ae)) = (ret, actual_ret) {
                    unify_param(pe, ae, type_params, bindings);
                }
            }
        }
        _ => {}
    }
}

/// Substitute inferred type-parameter bindings throughout a type.
fn substitute_type(ty: &Type, bindings: &std::collections::HashMap<String, Type>) -> Type {
    match ty {
        Type::Named(name) => {
            if let Some(bound) = bindings.get(name) {
                bound.clone()
            } else {
                ty.clone()
            }
        }
        Type::List(inner) => Type::List(Box::new(substitute_type(inner, bindings))),
        Type::Optional(inner) => Type::Optional(Box::new(substitute_type(inner, bindings))),
        Type::Union(parts) => {
            Type::Union(parts.iter().map(|p| substitute_type(p, bindings)).collect())
        }
        Type::Intersection(parts) => {
            Type::Intersection(parts.iter().map(|p| substitute_type(p, bindings)).collect())
        }
        Type::Async(inner) => Type::Async(Box::new(substitute_type(inner, bindings))),
        Type::FnSig { params, ret } => Type::FnSig {
            params: params
                .iter()
                .map(|p| substitute_type(p, bindings))
                .collect(),
            ret: ret.as_ref().map(|r| Box::new(substitute_type(r, bindings))),
        },
        other => other.clone(),
    }
}

fn literal_type(lit: &Literal) -> Type {
    match lit {
        Literal::String(_) => Type::String,
        Literal::Number(_) => Type::Number,
        Literal::Boolean(_) => Type::Boolean,
        Literal::Null => Type::Null,
        Literal::List(items) => {
            let element = items
                .iter()
                .find(|e| !matches!(e, Expression::Spread { .. }))
                .map(expr_type_hint)
                .unwrap_or(Type::Any);
            Type::List(Box::new(element))
        }
        Literal::Object(_) => Type::Object,
    }
}

/// Best-effort type of an expression without semantic checks (used to infer
/// list element types from literal syntax).
fn expr_type_hint(expr: &Expression) -> Type {
    match expr {
        Expression::Literal { value: lit, .. } => literal_type(lit),
        _ => Type::Any,
    }
}

/// Render a match arm pattern for error messages.
fn pattern_name(arm: &crate::ast::MatchArm) -> String {
    match &arm.pattern {
        crate::ast::MatchPattern::Literal(Literal::String(s)) => format!("\"{s}\""),
        crate::ast::MatchPattern::Literal(Literal::Number(n)) => n.to_string(),
        crate::ast::MatchPattern::Literal(Literal::Boolean(b)) => b.to_string(),
        crate::ast::MatchPattern::Literal(Literal::Null) => "null".into(),
        crate::ast::MatchPattern::Literal(Literal::List(_)) => "[...]".into(),
        crate::ast::MatchPattern::Literal(Literal::Object(_)) => "{...}".into(),
        crate::ast::MatchPattern::Enum(r) => format!("{}::{}", r.enum_name, r.variant),
        crate::ast::MatchPattern::EnumPayload {
            enum_name, variant, ..
        } => format!("{enum_name}::{variant}"),
        crate::ast::MatchPattern::Wildcard => "_".into(),
    }
}
