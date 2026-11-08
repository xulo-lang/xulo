//! Compile-time macro expansion (spec `builtins/execution.md`,
//! `type-system/checking-rules.md`).
//!
//! The expansion engine collects `#[macro]` definitions from the written AST,
//! checks their metadata, and runs a fixed-point loop that expands derive,
//! attribute, and function macros before global name resolution. Macro bodies
//! execute through the ordinary interpreter; the sandbox is enforced by
//! refusing the compile-time entry points for I/O and by running without a
//! host environment.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use xulo_core::ast::{
    Attribute, AttributeValue, Block, EnumDef, Expression, FnDef, ImplDecl, LetBinding,
    MacroCallExpr, ParamMode, Program, QuotePiece, Statement, StructDef, Type, TypeAlias,
};
use xulo_core::error::{ErrorKind, XuloError};

use crate::interpreter::{Interpreter, RunError};
use crate::value::{AstNodeRepr, AstNodeValue, SpanValue, Value};

/// The boxed callable a native `ast` member installs.
type NativeImpl = dyn Fn(&Interpreter, &[Value]) -> Result<Value, RunError>;

/// The three macro kinds (spec `#[macro]` definitions).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MacroKind {
    Derive,
    Attribute,
    Function,
}

impl MacroKind {
    fn name(self) -> &'static str {
        match self {
            MacroKind::Derive => "derive",
            MacroKind::Attribute => "attribute",
            MacroKind::Function => "function",
        }
    }
}

/// One collected `#[macro]` definition.
#[derive(Clone)]
pub struct MacroDef {
    pub kind: MacroKind,
    /// The macro name (`target` for derive, `name` otherwise).
    pub name: String,
    /// The implementation function's identifier.
    pub fn_name: String,
    pub span: Range<usize>,
    pub max_steps: Option<u64>,
    pub max_memory: Option<u64>,
    pub max_depth: Option<u64>,
    /// The defining module's program, for a macro imported from another
    /// module: its implementation and helpers are registered from here while
    /// context queries read the invoking module (`None` = same module).
    pub exec_base: Option<Rc<Program>>,
}

/// The macro namespace of one module: key -> definition. Keys are the written
/// macro names; qualified names are accepted too.
#[derive(Clone, Default)]
pub struct MacroRegistry {
    pub defs: HashMap<String, MacroDef>,
    /// Qualified paths that name a macro which exists but is not exported.
    /// They are not callable; expansion reports `E0602` instead of `E1005`.
    pub private: std::collections::HashSet<String>,
}

impl MacroRegistry {
    pub fn get(&self, name: &str) -> Option<&MacroDef> {
        self.defs.get(name).or_else(|| {
            name.rsplit("::")
                .next()
                .and_then(|short| self.defs.get(short))
        })
    }
}

/// Collect and validate the `#[macro]` definitions written in `program`.
pub fn collect_macros(program: &Program) -> Result<MacroRegistry, XuloError> {
    let mut registry = MacroRegistry::default();
    for statement in &program.statements {
        let f = match statement {
            Statement::Fn(f) => Some(f),
            Statement::Export(export) => match &export.item {
                xulo_core::ast::ExportItem::Fn(f) => Some(f),
                _ => None,
            },
            _ => None,
        };
        let Some(f) = f else { continue };
        let Some(attr) = f.attributes.iter().find(|a| a.name == "macro") else {
            continue;
        };
        let def = validate_macro_def(f, attr)?;
        if let Some(prev) = registry.defs.get(&def.name) {
            return Err(error(
                "E1010",
                format!("macro `{}` is already defined", def.name),
                attr.span.clone(),
            )
            .with_note(format!(
                "the previous definition is at bytes {}..{}",
                prev.span.start, prev.span.end
            )));
        }
        registry.defs.insert(def.name.clone(), def);
    }
    Ok(registry)
}

fn error(code: &'static str, message: impl Into<String>, span: Range<usize>) -> XuloError {
    XuloError::new(ErrorKind::Semantic, message)
        .with_code(code)
        .at(span)
}

fn invalid(message: impl Into<String>, span: Range<usize>) -> XuloError {
    error(
        "E1010",
        format!("invalid macro definition: {}", message.into()),
        span,
    )
}

fn validate_macro_def(f: &FnDef, attr: &Attribute) -> Result<MacroDef, XuloError> {
    let mut kind = None;
    let mut target = None;
    let mut name = None;
    let mut max_steps = None;
    let mut max_memory = None;
    let mut max_depth = None;
    for arg in &attr.args {
        let Some(key) = arg.name.as_deref() else {
            return Err(invalid(
                "`#[macro]` takes `name = value` keys only",
                arg.span.clone(),
            ));
        };
        match key {
            "kind" => {
                let AttributeValue::String(text) = &arg.value else {
                    return Err(invalid(
                        "`kind` must be a string (\"derive\", \"attribute\", or \"function\")",
                        arg.span.clone(),
                    ));
                };
                kind = Some(match text.as_str() {
                    "derive" => MacroKind::Derive,
                    "attribute" => MacroKind::Attribute,
                    "function" => MacroKind::Function,
                    other => {
                        return Err(invalid(
                            format!("unknown macro kind `{other}`"),
                            arg.span.clone(),
                        ));
                    }
                });
            }
            "target" => {
                let AttributeValue::Identifier(text) = &arg.value else {
                    return Err(invalid("`target` must be an identifier", arg.span.clone()));
                };
                target = Some(text.clone());
            }
            "name" => {
                let AttributeValue::String(text) = &arg.value else {
                    return Err(invalid("`name` must be a string", arg.span.clone()));
                };
                name = Some(text.clone());
            }
            "max_steps" => max_steps = Some(quota_value(arg, key)?),
            "max_memory" => max_memory = Some(quota_value(arg, key)?),
            "max_depth" => max_depth = Some(quota_value(arg, key)?),
            other => {
                return Err(invalid(
                    format!("unknown `#[macro]` key `{other}`"),
                    arg.span.clone(),
                ));
            }
        }
    }
    let kind = kind.ok_or_else(|| {
        invalid("`#[macro]` requires `kind`", attr.span.clone())
            .with_note("write `#[macro(kind = \"derive\", target = Model)]`")
    })?;
    let name = match kind {
        MacroKind::Derive => {
            if name.is_some() {
                return Err(invalid(
                    "a derive macro takes `target`, not `name`",
                    attr.span.clone(),
                ));
            }
            target.clone().ok_or_else(|| {
                invalid("derive macro requires `target`", attr.span.clone())
                    .with_note("write `#[macro(kind = \"derive\", target = Model)]`")
            })?
        }
        MacroKind::Attribute | MacroKind::Function => {
            if target.is_some() {
                return Err(invalid(
                    "an attribute or function macro takes `name`, not `target`",
                    attr.span.clone(),
                ));
            }
            name.ok_or_else(|| {
                invalid(
                    "attribute and function macros require `name`",
                    attr.span.clone(),
                )
            })?
        }
    };
    if !is_legal_macro_name(&name) {
        return Err(invalid(
            format!("`{name}` is not a legal macro name"),
            attr.span.clone(),
        ));
    }
    if matches!(name.as_str(), "derive" | "test" | "macro") {
        return Err(invalid(
            format!("`{name}` is a built-in attribute name and cannot be a macro"),
            attr.name_span.clone(),
        ));
    }
    if kind == MacroKind::Derive && matches!(name.as_str(), "Debug" | "Display" | "ToString") {
        return Err(invalid(
            format!("`{name}` is a reserved derive target"),
            attr.name_span.clone(),
        ));
    }
    // Function shape (spec type-system/checking-rules.md, `#[macro]`
    // definitions).
    if f.is_async || f.abi.is_some() {
        return Err(invalid(
            "a macro implementation must not be `async` or `extern`",
            f.name_span.clone(),
        ));
    }
    if !f.generic_params.is_empty() || !f.bounds.is_empty() {
        return Err(invalid(
            "a macro implementation must not declare generic parameters or bounds",
            f.name_span.clone(),
        ));
    }
    if f.params.len() != 1 || f.params[0].mode.is_some() || f.params[0].default.is_some() {
        return Err(invalid(
            "a macro implementation takes exactly one plain parameter",
            f.name_span.clone(),
        ));
    }
    let (expect_param, expect_output) = match kind {
        MacroKind::Derive => ("ast::DeriveInput", "ast::DeriveOutput"),
        MacroKind::Attribute => ("ast::AttributeInput", "ast::AttributeOutput"),
        MacroKind::Function => ("ast::FunctionInput", "ast::FunctionOutput"),
    };
    let Some(param_ty) = &f.params[0].type_annotation else {
        return Err(invalid(
            "the macro parameter needs a type annotation",
            f.params[0].span.clone(),
        ));
    };
    if !type_is(param_ty, expect_param) {
        return Err(invalid(
            format!("the macro parameter must be `{expect_param}`"),
            f.params[0].span.clone(),
        ));
    }
    let ok_return = matches!(
        &f.return_type,
        Some(Type::Named(name, args))
            if name == "Result"
                && args.len() == 2
                && type_is(&args[0], expect_output)
                && type_is(&args[1], "ast::MacroFailure")
    );
    if !ok_return {
        return Err(invalid(
            format!("the macro must return `Result<{expect_output}, ast::MacroFailure>`"),
            f.return_type
                .as_ref()
                .map(|_| f.span.clone())
                .unwrap_or(f.name_span.clone()),
        ));
    }
    if has_user_attribute_macro(&f.attributes, attr) {
        return Err(invalid(
            "a `#[macro]` function must not carry a user attribute macro",
            attr.span.clone(),
        ));
    }
    if body_contains_macro_call(&f.body) {
        return Err(invalid(
            "a macro body must not contain a direct user macro invocation; use `ast::call_*`",
            f.body
                .statements
                .first()
                .map(statement_span)
                .unwrap_or_else(|| f.span.clone()),
        ));
    }
    if let Some((resource, span)) = forbidden_resource(&f.body) {
        return Err(error(
            "E1007",
            format!("macro implementation cannot access {resource}"),
            span,
        )
        .with_note(
            "compile-time macros may not access the filesystem, network, environment, processes, concurrency, wall-clock time, or randomness",
        ));
    }
    Ok(MacroDef {
        kind,
        name,
        fn_name: f.name.clone(),
        span: f.span.clone(),
        max_steps,
        max_memory,
        max_depth,
        exec_base: None,
    })
}

fn quota_value(arg: &xulo_core::ast::AttributeArg, key: &str) -> Result<u64, XuloError> {
    match arg.value {
        AttributeValue::Integer(n) if n >= 0 => Ok(n as u64),
        _ => Err(invalid(
            format!("`{key}` must be a non-negative integer literal"),
            arg.span.clone(),
        )),
    }
}

fn is_legal_macro_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn type_is(ty: &Type, expected: &str) -> bool {
    matches!(ty, Type::Named(name, args) if name == expected && args.is_empty())
}

fn has_user_attribute_macro(attrs: &[Attribute], macro_attr: &Attribute) -> bool {
    attrs.iter().any(|a| {
        !std::ptr::eq(a, macro_attr) && !matches!(a.name.as_str(), "test" | "derive" | "macro")
    })
}

/// Runtime context shared by every interpreter that executes a macro in one
/// expansion tree.
struct MacroCtx {
    registry: Rc<MacroRegistry>,
    base: Rc<Program>,
    source: String,
    file: String,
    chain: Rc<RefCell<Vec<String>>>,
    current_span: RefCell<SpanValue>,
    /// Budget shared by one top-level expansion and its `ast::call_*` tree
    /// (spec `builtins/quota.md`); empty between invocation sites.
    quota: Rc<RefCell<Option<Rc<QuotaState>>>>,
}

/// The counters of one top-level expansion. Nested macro calls share these
/// cells; the root invocation seeds them from its `#[macro]` overrides.
struct QuotaState {
    step_limit: Rc<Cell<Option<u64>>>,
    step_used: Rc<Cell<u64>>,
    mem_limit: Rc<Cell<Option<u64>>>,
    mem_used: Rc<Cell<u64>>,
    mem_seen: Rc<RefCell<std::collections::HashSet<usize>>>,
    depth_limit: Rc<Cell<Option<usize>>>,
}

impl QuotaState {
    fn for_def(def: &MacroDef) -> Self {
        Self {
            step_limit: Rc::new(Cell::new(Some(def.max_steps.unwrap_or(1_000_000)))),
            step_used: Rc::new(Cell::new(0)),
            mem_limit: Rc::new(Cell::new(Some(def.max_memory.unwrap_or(16_000_000)))),
            mem_used: Rc::new(Cell::new(0)),
            mem_seen: Rc::new(RefCell::new(std::collections::HashSet::new())),
            depth_limit: Rc::new(Cell::new(Some(
                def.max_depth.unwrap_or(100).min(usize::MAX as u64) as usize,
            ))),
        }
    }
}

impl MacroCtx {
    fn span(&self, span: &Range<usize>) -> SpanValue {
        let (line, column) = line_col(&self.source, span.start);
        SpanValue {
            file: self.file.clone(),
            start: span.start,
            end: span.end,
            line,
            column,
        }
    }
}

/// Expand every macro invocation in `program` in place (spec
/// `type-system/checking-rules.md`, "Expansion pass").
pub fn expand_program(program: &mut Program, source: &str, file: &str) -> Result<(), XuloError> {
    let registry = collect_macros(program)?;
    expand_with_registry(program, source, file, registry)
}

/// Expand a module that imports macro names from already-loaded modules
/// (spec `modules/import-export.md`): `imports` maps each local macro name to
/// its definition in the defining module.
pub fn expand_program_with_imports(
    program: &mut Program,
    source: &str,
    file: &str,
    imports: &[(String, MacroDef)],
    qualified: &[(String, MacroDef)],
    private_qualified: &std::collections::HashSet<String>,
) -> Result<(), XuloError> {
    let mut registry = collect_macros(program)?;
    registry.private.extend(private_qualified.iter().cloned());
    for (local, def) in imports {
        if registry.defs.contains_key(local) {
            return Err(error(
                "E0104",
                format!("macro `{local}` is already in scope"),
                def.span.clone(),
            ));
        }
        registry.defs.insert(local.clone(), def.clone());
    }
    for (path, def) in qualified {
        registry
            .defs
            .entry(path.clone())
            .or_insert_with(|| def.clone());
    }
    expand_with_registry(program, source, file, registry)
}

fn expand_with_registry(
    program: &mut Program,
    source: &str,
    file: &str,
    registry: MacroRegistry,
) -> Result<(), XuloError> {
    if registry.defs.is_empty() && !program_has_macro_usage(program) {
        return Ok(());
    }
    let written_macro_spans = macro_definition_spans(program);
    let run_program = prepare_exec_program(program);
    let ctx = MacroCtx {
        registry: Rc::new(registry),
        base: Rc::new(run_program),
        source: source.to_string(),
        file: file.to_string(),
        chain: Rc::new(RefCell::new(Vec::new())),
        current_span: RefCell::new(SpanValue {
            file: file.to_string(),
            ..SpanValue::empty()
        }),
        quota: Rc::new(RefCell::new(None)),
    };
    for _round in 0..100 {
        let mut changed = false;
        let mut next = Vec::new();
        for statement in std::mem::take(&mut program.statements) {
            let mut expanded = expand_declaration(statement, &ctx, &mut changed)?;
            for stmt in &mut expanded {
                map_statement(stmt, &mut |expr| {
                    expand_function_expr(expr, &ctx, &mut changed)
                })?;
            }
            next.extend(expanded);
        }
        program.statements = next;
        if !changed {
            // A `#[macro]` produced by an expansion is not registered
            // (spec type-system/checking-rules.md, `#[macro]` definitions).
            for span in macro_definition_spans(program) {
                if !written_macro_spans.contains(&span) {
                    return Err(error(
                        "E1010",
                        "a macro definition produced by expansion is not registered",
                        span.0..span.1,
                    )
                    .with_note(
                        "`#[macro]` definitions are collected from the written AST before expansion",
                    ));
                }
            }
            return Ok(());
        }
    }
    Err(error(
        "E1004",
        "macro expansion exceeded maximum rounds (100)",
        0..0,
    )
    .with_note("expansion may be stuck in an infinite loop"))
}

/// The program an expansion's interpreter registers: declarations only, with
/// the macro-surface types and without `main`/tests (spec
/// `type-system/checking-rules.md`, steps 2–3).
fn prepare_exec_program(program: &Program) -> Program {
    let mut run_program = program.clone();
    run_program.statements.retain(|statement| {
        if matches!(statement, Statement::Test(_)) {
            return false;
        }
        !matches!(inner_decl(statement), InnerDecl::Fn(f) if f.name == "main")
    });
    add_ast_type_decls(&mut run_program);
    run_program
}

/// The spans of every `#[macro]` attribute in `program`'s top-level
/// declarations; used to detect definitions produced by expansion.
fn macro_definition_spans(program: &Program) -> std::collections::HashSet<(usize, usize)> {
    let mut spans = std::collections::HashSet::new();
    for statement in &program.statements {
        if let Some(attrs) = statement_attrs(statement)
            && let Some(attr) = attrs.iter().find(|a| a.name == "macro")
        {
            spans.insert((attr.span.start, attr.span.end));
        }
    }
    spans
}

fn program_has_macro_usage(program: &Program) -> bool {
    let mut found = false;
    let mut probe = program.clone();
    let _ = map_program(&mut probe, &mut |expr| {
        if matches!(expr, Expression::MacroCall(_)) {
            found = true;
        }
        Ok(())
    });
    for stmt in &program.statements {
        if let Some(attrs) = statement_attrs(stmt) {
            for a in attrs {
                if a.name == "derive" && a.identifier_args().iter().any(|(n, _)| *n != "Debug") {
                    found = true;
                }
                if !matches!(a.name.as_str(), "test" | "derive" | "macro") {
                    found = true;
                }
            }
        }
    }
    found
}

fn expand_declaration(
    mut statement: Statement,
    ctx: &MacroCtx,
    changed: &mut bool,
) -> Result<Vec<Statement>, XuloError> {
    let mut items = expand_derives(&mut statement, ctx, changed)?;
    let extras = expand_attributes(&mut statement, ctx, changed)?;
    let mut out = vec![statement];
    out.extend(extras);
    out.append(&mut items);
    Ok(out)
}

fn expand_derives(
    statement: &mut Statement,
    ctx: &MacroCtx,
    changed: &mut bool,
) -> Result<Vec<Statement>, XuloError> {
    let Some(attrs) = statement_attrs_mut(statement) else {
        return Ok(Vec::new());
    };
    let Some(pos) = attrs.iter().position(|a| a.name == "derive") else {
        return Ok(Vec::new());
    };
    let attr = attrs[pos].clone();
    let mut keep = Vec::new();
    let mut generated = Vec::new();
    let mut applied = false;
    for arg in attr.args.clone() {
        let AttributeValue::Identifier(name) = &arg.value else {
            keep.push(arg);
            continue;
        };
        if name == "Debug" || name == "Display" || name == "ToString" {
            keep.push(arg);
            continue;
        }
        let Some(def) = ctx.registry.get(name) else {
            return Err(error(
                "E1005",
                format!("cannot find macro `{name}`"),
                arg.span.clone(),
            )
            .with_note(format!("no derive macro `{name}` is in scope")));
        };
        if def.kind != MacroKind::Derive {
            return Err(error(
                "E1006",
                format!("`{name}` is not a derive macro"),
                arg.span.clone(),
            ));
        }
        let input = derive_input(statement, ctx);
        let output = invoke_macro(ctx, def, input)?;
        generated.extend(derive_output(output, &def.name)?);
        applied = true;
    }
    if applied {
        *changed = true;
        let attrs = statement_attrs_mut(statement).expect("declaration attrs");
        let pos = attrs
            .iter()
            .position(|a| a.name == "derive")
            .expect("derive attr");
        if keep.is_empty() {
            attrs.remove(pos);
        } else {
            attrs[pos].args = keep;
        }
    }
    Ok(generated)
}

fn expand_attributes(
    statement: &mut Statement,
    ctx: &MacroCtx,
    changed: &mut bool,
) -> Result<Vec<Statement>, XuloError> {
    let mut extras = Vec::new();
    let mut applied = 0usize;
    while let Some(attrs) = statement_attrs(statement) {
        applied += 1;
        if applied > 1000 {
            return Err(error(
                "E1004",
                "attribute macro expansion did not terminate",
                statement_span(statement),
            )
            .with_note("the macro keeps re-attaching its own attribute"));
        }
        let Some(pos) = attrs
            .iter()
            .position(|a| !matches!(a.name.as_str(), "test" | "derive" | "macro"))
        else {
            break;
        };
        let name = attrs[pos].name.clone();
        let Some(def) = ctx.registry.get(&name) else {
            // Unknown attributes are the ordinary `E0108`, reported by the
            // checker after expansion.
            break;
        };
        if def.kind != MacroKind::Attribute {
            return Err(error(
                "E1006",
                format!(
                    "`{name}` is a {} macro, not an attribute macro",
                    def.kind.name()
                ),
                attrs[pos].name_span.clone(),
            ));
        }
        let host = host_kind(statement).expect("attribute host");
        let attr = {
            let attrs = statement_attrs_mut(statement).expect("declaration attrs");
            attrs.remove(pos)
        };
        let input = attribute_input(statement, &attr, ctx);
        let def = def.clone();
        let output = invoke_macro(ctx, &def, input)?;
        let (item, mut out_extras) = attribute_output(output, &def.name)?;
        if host_kind(&item).as_deref() != Some(host.as_str()) {
            return Err(error(
                "E1006",
                format!(
                    "attribute macro `{}` returned a {} declaration for a {host}",
                    def.name,
                    host_kind(&item).unwrap_or_else(|| "different".to_string())
                ),
                attr.span.clone(),
            ));
        }
        *statement = item;
        extras.append(&mut out_extras);
        *changed = true;
    }
    Ok(extras)
}

fn expand_function_expr(
    expr: &mut Expression,
    ctx: &MacroCtx,
    changed: &mut bool,
) -> Result<(), XuloError> {
    let Expression::MacroCall(call) = expr else {
        return Ok(());
    };
    let path = call.path.join("::");
    let Some(def) = ctx.registry.get(&path) else {
        if ctx.registry.private.contains(&path) {
            return Err(error(
                "E0602",
                format!("macro `{path}` is not exported by its module"),
                call.span.clone(),
            ));
        }
        return Err(error(
            "E1005",
            format!("cannot find macro `{path}`"),
            call.span.clone(),
        ));
    };
    if def.kind != MacroKind::Function {
        return Err(error(
            "E1006",
            format!("`{path}` is not a function macro"),
            call.span.clone(),
        ));
    }
    // Every name mentioned by an argument must resolve in the pre-expansion
    // scope (spec expressions/calls.md, E1009). The scanner over-approximates
    // the scope (every binding written anywhere in the module), so it rejects
    // only names that are genuinely unbound.
    let known = known_names(&ctx.base);
    for arg in &call.args {
        if let Some((name, span)) = unbound_name(arg, &known) {
            return Err(error(
                "E1009",
                format!("cannot resolve `{name}` in macro arguments"),
                span,
            )
            .with_note(
                "macro arguments are resolved before expansion, so a macro cannot reference a name produced by another macro",
            ));
        }
    }
    let def = def.clone();
    let input = function_input(call, ctx);
    let output = invoke_macro(ctx, &def, input)?;
    let expr_node = function_output(output, &def.name)?;
    *expr = expr_node;
    *changed = true;
    Ok(())
}

/// Names visible in the pre-expansion module scope. The set deliberately
/// over-approximates lexical scopes: it exists to reject a name that is bound
/// nowhere, which is the `E1009` case, without false positives.
fn known_names(program: &Program) -> std::collections::HashSet<String> {
    use std::collections::HashSet;
    let mut set: HashSet<String> = [
        "self", "print", "println", "str", "assert", "Math", "Time", "Task", "ast",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for stmt in &program.statements {
        collect_stmt_names(stmt, &mut set);
    }
    let mut probe = program.clone();
    let _ = map_program(&mut probe, &mut |expr| {
        match expr {
            Expression::Match(m) => {
                for arm in &m.arms {
                    collect_pattern_names(&arm.pattern, &mut set);
                }
            }
            Expression::FnExpr(f) => {
                for p in &f.params {
                    set.insert(p.name.clone());
                }
            }
            Expression::Arrow(a) => {
                for p in &a.params {
                    set.insert(p.name.clone());
                }
            }
            _ => {}
        }
        Ok(())
    });
    set
}

fn collect_stmt_names(stmt: &Statement, set: &mut std::collections::HashSet<String>) {
    match stmt {
        Statement::Fn(f) => {
            set.insert(f.name.clone());
            for p in &f.params {
                set.insert(p.name.clone());
            }
            collect_block_names(&f.body, set);
        }
        Statement::Test(f) => collect_block_names(&f.body, set),
        Statement::Let(b) => {
            if !b.name.is_empty() {
                set.insert(b.name.clone());
            }
            if let Some(names) = &b.tuple_names {
                set.extend(names.iter().cloned());
            }
            if let Some(destructure) = &b.object_destructuring {
                for (field, alias) in destructure {
                    set.insert(alias.clone().unwrap_or_else(|| field.clone()));
                }
            }
        }
        Statement::For(f) => {
            set.insert(f.iter_var.clone());
            collect_block_names(&f.body, set);
        }
        Statement::While(w) => collect_block_names(&w.body, set),
        Statement::Block(b) => collect_block_names(b, set),
        Statement::Try(t) => {
            set.insert(t.catch_var.clone());
            collect_block_names(&t.try_block, set);
            collect_block_names(&t.catch_block, set);
        }
        Statement::Impl(i) => {
            for m in &i.methods {
                for p in &m.params {
                    set.insert(p.name.clone());
                }
                collect_block_names(&m.body, set);
            }
        }
        Statement::Export(export) => match &export.item {
            xulo_core::ast::ExportItem::Fn(f) => {
                set.insert(f.name.clone());
                for p in &f.params {
                    set.insert(p.name.clone());
                }
                collect_block_names(&f.body, set);
            }
            xulo_core::ast::ExportItem::Let(b) => {
                set.insert(b.name.clone());
            }
            xulo_core::ast::ExportItem::Struct(s) => {
                set.insert(s.name.clone());
            }
            xulo_core::ast::ExportItem::Enum(e) => {
                set.insert(e.name.clone());
            }
            xulo_core::ast::ExportItem::Trait(t) => {
                set.insert(t.name.clone());
            }
            xulo_core::ast::ExportItem::Type(t) => {
                set.insert(t.name.clone());
            }
            xulo_core::ast::ExportItem::Names(names) => set.extend(names.iter().cloned()),
        },
        Statement::Struct(s) => {
            set.insert(s.name.clone());
        }
        Statement::Enum(e) => {
            set.insert(e.name.clone());
        }
        Statement::Trait(t) => {
            set.insert(t.name.clone());
        }
        Statement::TypeAlias(t) => {
            set.insert(t.name.clone());
        }
        Statement::Use(u) => collect_use_names(&u.target, set),
        Statement::State(s) if !s.binding.name.is_empty() => {
            set.insert(s.binding.name.clone());
        }
        Statement::Store(s) => match &s.pattern {
            xulo_core::ast::BindingPattern::Ident(name) => {
                set.insert(name.clone());
            }
            xulo_core::ast::BindingPattern::Destructure(fields) => {
                for (field, alias) in fields {
                    set.insert(alias.clone().unwrap_or_else(|| field.clone()));
                }
            }
        },
        Statement::Effect(e) => {
            for p in &e.closure.params {
                set.insert(p.name.clone());
            }
        }
        Statement::Component(c) => {
            for arg in &c.args {
                if let Some(name) = &arg.name {
                    set.insert(name.clone());
                }
            }
        }
        Statement::Mod(m) => {
            set.insert(m.name.clone());
            if let Some(body) = &m.body {
                for stmt in body {
                    collect_stmt_names(stmt, set);
                }
            }
        }
        _ => {}
    }
}

fn collect_block_names(block: &Block, set: &mut std::collections::HashSet<String>) {
    for stmt in &block.statements {
        collect_stmt_names(stmt, set);
    }
}

fn collect_pattern_names(
    pattern: &xulo_core::ast::MatchPattern,
    set: &mut std::collections::HashSet<String>,
) {
    use xulo_core::ast::MatchPattern;
    match pattern {
        MatchPattern::Binding { name, .. } => {
            set.insert(name.clone());
        }
        MatchPattern::TypePattern {
            binder: xulo_core::ast::PatternBinder::Binding(name),
            ..
        } => {
            set.insert(name.clone());
        }
        MatchPattern::EnumPayload { bindings, .. } => set.extend(bindings.iter().cloned()),
        MatchPattern::Struct { bindings, .. }
        | MatchPattern::Or {
            patterns: bindings, ..
        } => {
            for b in bindings {
                collect_pattern_names(b, set);
            }
        }
        _ => {}
    }
}

fn collect_use_names(
    target: &xulo_core::ast::UseTarget,
    set: &mut std::collections::HashSet<String>,
) {
    use xulo_core::ast::UseTarget;
    match target {
        UseTarget::Path {
            segments, alias, ..
        } => {
            if let Some(alias) = alias {
                set.insert(alias.clone());
            } else if let Some(last) = segments.last() {
                set.insert(last.clone());
            }
        }
        UseTarget::Group { entries, .. } => {
            for entry in entries {
                if let Some(alias) = &entry.alias {
                    set.insert(alias.clone());
                } else if let Some(last) = entry.segments.last() {
                    set.insert(last.clone());
                }
            }
        }
        UseTarget::Glob { .. } => {}
        UseTarget::Local(names) => set.extend(names.iter().cloned()),
    }
}

fn unbound_name(
    expr: &Expression,
    known: &std::collections::HashSet<String>,
) -> Option<(String, Range<usize>)> {
    let mut probe = expr.clone();
    let mut found = None;
    let _ = map_expr(&mut probe, &mut |expr| {
        if let Expression::Identifier { name, span } = expr
            && !known.contains(name)
        {
            found = Some((name.clone(), span.clone()));
        }
        Ok(())
    });
    found
}

fn invoke_macro(ctx: &MacroCtx, def: &MacroDef, input: Value) -> Result<Value, XuloError> {
    if ctx.chain.borrow().iter().any(|n| n == &def.name) {
        let mut chain = ctx.chain.borrow().clone();
        chain.push(def.name.clone());
        return Err(error(
            "E1003",
            format!("recursive macro call: {}", chain.join(" → ")),
            def.span.clone(),
        ));
    }
    let existing = ctx.quota.borrow().clone();
    let fresh = existing.is_none();
    let quota = match existing {
        Some(quota) => quota,
        None => {
            let quota = Rc::new(QuotaState::for_def(def));
            *ctx.quota.borrow_mut() = Some(quota.clone());
            quota
        }
    };
    ctx.chain.borrow_mut().push(def.name.clone());
    let result = run_macro(ctx, def, input, &quota);
    ctx.chain.borrow_mut().pop();
    if fresh {
        *ctx.quota.borrow_mut() = None;
    }
    result
}

fn run_macro(
    ctx: &MacroCtx,
    def: &MacroDef,
    input: Value,
    quota: &QuotaState,
) -> Result<Value, XuloError> {
    let mut interp = Interpreter::new();
    install_ast(&interp, ctx);
    // Registration evaluates declarations only, not the macro body; the
    // budget starts at the invocation (spec builtins/quota.md, "Counting").
    let imported_base;
    let exec_base: &Program = match &def.exec_base {
        Some(base) => {
            imported_base = prepare_exec_program(base);
            &imported_base
        }
        None => &ctx.base,
    };
    interp.run(exec_base).map_err(|e| {
        error(
            "E1012",
            format!("macro `{}` trapped: {}", def.name, e.message),
            def.span.clone(),
        )
    })?;
    interp.share_quota(
        quota.step_limit.clone(),
        quota.step_used.clone(),
        quota.mem_limit.clone(),
        quota.mem_used.clone(),
        quota.mem_seen.clone(),
        quota.depth_limit.clone(),
    );
    let callee = interp
        .root_env()
        .borrow()
        .get(&def.fn_name)
        .ok_or_else(|| {
            error(
                "E1010",
                format!(
                    "macro `{}` has no implementation function `{}`",
                    def.name, def.fn_name
                ),
                def.span.clone(),
            )
        })?;
    let value = interp
        .call_value_with_values(&callee, &[input])
        .map_err(|e| match e {
            // A macro-call failure from `ast::call_*` keeps its own macro
            // diagnostic (E1002/E1003/E1005/E1006); anything else is a
            // compile-time trap (E1012).
            RunError::Err(e) if e.code().starts_with("E10") && e.code() != "E1012" => e,
            RunError::Err(e) => error(
                "E1012",
                format!("macro `{}` trapped: {}", def.name, e.message),
                def.span.clone(),
            ),
            other => error(
                "E1012",
                format!(
                    "macro `{}` trapped: {}",
                    def.name,
                    run_error_message(&other)
                ),
                def.span.clone(),
            ),
        })?;
    Ok(value)
}

fn run_error_message(e: &RunError) -> String {
    match e {
        RunError::Err(e) => e.message.clone(),
        RunError::Throw(v) | RunError::Propagate(v) => v.format(),
    }
}

// ---------------------------------------------------------------------------
// Input records
// ---------------------------------------------------------------------------

fn object(fields: Vec<(String, Value)>) -> Value {
    Value::Object(Rc::new(RefCell::new(fields)))
}

fn value_list(items: Vec<Value>) -> Value {
    Value::List(Rc::new(RefCell::new(items)))
}

fn span_value_json(ctx: &MacroCtx, span: &Range<usize>) -> Value {
    let s = ctx.span(span);
    object(vec![
        ("file".to_string(), Value::String(s.file.into())),
        ("start".to_string(), Value::Number(s.start as f64)),
        ("end".to_string(), Value::Number(s.end as f64)),
        ("line".to_string(), Value::Number(s.line as f64)),
        ("column".to_string(), Value::Number(s.column as f64)),
    ])
}

fn ast_value(value: &AttributeValue) -> Value {
    match value {
        AttributeValue::Identifier(s) | AttributeValue::String(s) => {
            Value::String(s.clone().into())
        }
        AttributeValue::Integer(n) => Value::Number(*n as f64),
        AttributeValue::Float(n) => Value::Number(*n),
        AttributeValue::Boolean(b) => Value::Boolean(*b),
    }
}

fn attribute_info(attr: &Attribute, ctx: &MacroCtx) -> Value {
    let args = attr
        .args
        .iter()
        .map(|arg| {
            object(vec![
                (
                    "name".to_string(),
                    arg.name
                        .as_ref()
                        .map(|n| Value::String(n.clone().into()))
                        .unwrap_or(Value::Null),
                ),
                ("value".to_string(), ast_value(&arg.value)),
            ])
        })
        .collect();
    object(vec![
        ("name".to_string(), Value::String(attr.name.clone().into())),
        ("args".to_string(), value_list(args)),
        ("span".to_string(), span_value_json(ctx, &attr.span)),
    ])
}

fn attributes_info(attrs: &[Attribute], ctx: &MacroCtx) -> Value {
    value_list(attrs.iter().map(|a| attribute_info(a, ctx)).collect())
}

fn declared_type(ty: &Type) -> String {
    ty.display()
}

fn field_info(
    name: &str,
    ty: &Type,
    attrs: &[Attribute],
    span: &Range<usize>,
    ctx: &MacroCtx,
) -> Value {
    object(vec![
        ("name".to_string(), Value::String(name.into())),
        ("ty".to_string(), Value::String(declared_type(ty).into())),
        ("attrs".to_string(), attributes_info(attrs, ctx)),
        ("span".to_string(), span_value_json(ctx, span)),
    ])
}

fn type_info(statement: &Statement, ctx: &MacroCtx) -> Value {
    let (kind, name, fields, variants, generics, attrs, span) = match inner_decl(statement) {
        InnerDecl::Struct(s) => (
            "struct",
            s.name.clone(),
            s.fields
                .iter()
                .map(|f| field_info(&f.name, &f.type_, &[], &f.name_span, ctx))
                .collect(),
            Vec::new(),
            s.generic_params.clone(),
            s.attributes.clone(),
            s.name_span.clone(),
        ),
        InnerDecl::Enum(e) => (
            "enum",
            e.name.clone(),
            Vec::new(),
            e.variants
                .iter()
                .map(|v| {
                    let fields: Vec<Value> = v
                        .payload
                        .iter()
                        .flatten()
                        .map(|p| {
                            field_info(
                                p.name.as_deref().unwrap_or("_"),
                                &p.type_,
                                &[],
                                &v.name_span,
                                ctx,
                            )
                        })
                        .collect();
                    object(vec![
                        ("name".to_string(), Value::String(v.name.clone().into())),
                        ("fields".to_string(), value_list(fields)),
                        ("attrs".to_string(), Value::Null),
                        ("span".to_string(), span_value_json(ctx, &v.name_span)),
                    ])
                })
                .collect(),
            e.generic_params.clone(),
            e.attributes.clone(),
            e.name_span.clone(),
        ),
        _ => {
            return object(vec![
                ("kind".to_string(), Value::String("unresolved".into())),
                ("name".to_string(), Value::String(String::new().into())),
                ("fields".to_string(), value_list(Vec::new())),
                ("variants".to_string(), value_list(Vec::new())),
                ("generics".to_string(), value_list(Vec::new())),
                ("attrs".to_string(), value_list(Vec::new())),
                ("span".to_string(), span_value_json(ctx, &(0..0))),
            ]);
        }
    };
    let generic_values = generics
        .iter()
        .map(|g| {
            object(vec![
                (
                    "kind".to_string(),
                    Value::String(
                        match g.kind {
                            xulo_core::ast::GenericParamKind::Type => "type",
                            xulo_core::ast::GenericParamKind::Const => "const",
                        }
                        .into(),
                    ),
                ),
                ("name".to_string(), Value::String(g.name.clone().into())),
                ("bounds".to_string(), value_list(Vec::new())),
                (
                    "ty".to_string(),
                    g.const_type
                        .as_ref()
                        .map(|t| Value::String(declared_type(t).into()))
                        .unwrap_or(Value::Null),
                ),
                ("span".to_string(), span_value_json(ctx, &(0..0))),
            ])
        })
        .collect();
    object(vec![
        ("kind".to_string(), Value::String(kind.into())),
        ("name".to_string(), Value::String(name.into())),
        ("fields".to_string(), value_list(fields)),
        ("variants".to_string(), value_list(variants)),
        ("generics".to_string(), value_list(generic_values)),
        ("attrs".to_string(), attributes_info(&attrs, ctx)),
        ("span".to_string(), span_value_json(ctx, &span)),
    ])
}

fn derive_input(statement: &Statement, ctx: &MacroCtx) -> Value {
    object(vec![("target".to_string(), type_info(statement, ctx))])
}

fn node_of(statement: &Statement, ctx: &MacroCtx, span: &Range<usize>) -> Value {
    let text = ctx.source.get(span.start..span.end).map(|s| s.to_string());
    Value::AstNode(Rc::new(AstNodeValue {
        kind: statement_kind(statement).to_string(),
        span: ctx.span(span),
        text,
        fields: Vec::new(),
        repr: AstNodeRepr::Statement(Box::new(statement.clone())),
    }))
}

fn declaration_info(statement: &Statement, ctx: &MacroCtx) -> Value {
    let (kind, name, params, return_type, attrs, trait_ref, self_type, span) =
        match inner_decl(statement) {
            InnerDecl::Fn(f) => (
                "fn",
                f.name.clone(),
                f.params
                    .iter()
                    .map(|p| {
                        object(vec![
                            ("name".to_string(), Value::String(p.name.clone().into())),
                            (
                                "ty".to_string(),
                                Value::String(
                                    p.type_annotation
                                        .as_ref()
                                        .map(declared_type)
                                        .unwrap_or_default()
                                        .into(),
                                ),
                            ),
                            (
                                "mode".to_string(),
                                Value::String(
                                    match p.mode {
                                        None => "value",
                                        Some(ParamMode::Mut) => "mut",
                                        Some(ParamMode::Shared) => "shared",
                                    }
                                    .into(),
                                ),
                            ),
                            (
                                "has_default".to_string(),
                                Value::Boolean(p.default.is_some()),
                            ),
                            ("span".to_string(), span_value_json(ctx, &p.span)),
                        ])
                    })
                    .collect(),
                f.return_type
                    .as_ref()
                    .map(|t| Value::String(declared_type(t).into())),
                f.attributes.clone(),
                None,
                None,
                f.span.clone(),
            ),
            InnerDecl::Struct(s) => (
                "struct",
                s.name.clone(),
                Vec::new(),
                None,
                s.attributes.clone(),
                None,
                None,
                s.name_span.clone(),
            ),
            InnerDecl::Enum(e) => (
                "enum",
                e.name.clone(),
                Vec::new(),
                None,
                e.attributes.clone(),
                None,
                None,
                e.name_span.clone(),
            ),
            InnerDecl::Trait(t) => (
                "trait",
                t.name.clone(),
                Vec::new(),
                None,
                t.attributes.clone(),
                None,
                None,
                t.name_span.clone(),
            ),
            InnerDecl::Impl(i) => (
                "impl",
                String::new(),
                Vec::new(),
                None,
                i.attributes.clone(),
                Some(i.trait_name.clone()),
                Some(i.type_name.clone()),
                i.span.clone(),
            ),
            InnerDecl::Type(t) => (
                "type",
                t.name.clone(),
                Vec::new(),
                None,
                t.attributes.clone(),
                None,
                None,
                t.name_span.clone(),
            ),
            InnerDecl::Let(b) => (
                if b.is_const { "const" } else { "let" },
                b.name.clone(),
                Vec::new(),
                None,
                b.attributes.clone(),
                None,
                None,
                b.name_span.clone(),
            ),
            InnerDecl::Other => (
                "fn",
                String::new(),
                Vec::new(),
                None,
                Vec::new(),
                None,
                None,
                0..0,
            ),
        };
    object(vec![
        ("kind".to_string(), Value::String(kind.into())),
        ("name".to_string(), Value::String(name.into())),
        ("node".to_string(), node_of(statement, ctx, &span)),
        ("params".to_string(), value_list(params)),
        (
            "return_type".to_string(),
            return_type.unwrap_or(Value::Null),
        ),
        ("attrs".to_string(), attributes_info(&attrs, ctx)),
        (
            "trait_ref".to_string(),
            trait_ref
                .map(|s| Value::String(s.into()))
                .unwrap_or(Value::Null),
        ),
        (
            "self_type".to_string(),
            self_type
                .map(|s| Value::String(s.into()))
                .unwrap_or(Value::Null),
        ),
        ("span".to_string(), span_value_json(ctx, &span)),
    ])
}

fn attribute_input(statement: &Statement, attr: &Attribute, ctx: &MacroCtx) -> Value {
    let mut positional = Vec::new();
    let mut named = Vec::new();
    for arg in &attr.args {
        match &arg.name {
            Some(name) => named.push((name.clone(), ast_value(&arg.value))),
            None => positional.push(ast_value(&arg.value)),
        }
    }
    object(vec![
        ("args".to_string(), value_list(positional)),
        ("named_args".to_string(), object(named)),
        ("target".to_string(), declaration_info(statement, ctx)),
    ])
}

fn function_input(call: &MacroCallExpr, ctx: &MacroCtx) -> Value {
    let args = call
        .args
        .iter()
        .map(|arg| scalar_or_node(arg, ctx))
        .collect();
    let text = ctx
        .source
        .get(call.span.start..call.span.end)
        .unwrap_or_default()
        .to_string();
    let target = object(vec![
        ("kind".to_string(), Value::String("macro_call".into())),
        ("text".to_string(), Value::String(text.into())),
        (
            "node".to_string(),
            Value::AstNode(Rc::new(AstNodeValue {
                kind: "MacroCallExpr".to_string(),
                span: ctx.span(&call.span),
                text: None,
                fields: Vec::new(),
                repr: AstNodeRepr::Expression(Box::new(Expression::MacroCall(Box::new(
                    call.clone(),
                )))),
            })),
        ),
        ("span".to_string(), span_value_json(ctx, &call.span)),
    ]);
    object(vec![
        ("args".to_string(), value_list(args)),
        ("target".to_string(), target),
    ])
}

fn conversion_error(kind: MacroKind, found: &str) -> RunError {
    let expected = match kind {
        MacroKind::Derive => "a struct or enum declaration",
        MacroKind::Attribute => "a declaration",
        MacroKind::Function => "an expression",
    };
    RunError::Err(
        error(
            "E1002",
            format!("cannot convert `{found}` to {}", kind.name()),
            0..0,
        )
        .with_note(format!("expected {expected}"))
        .with_note(format!(
            "`ast::call_{}` uses the node as its input record",
            kind.name()
        )),
    )
}

fn function_input_from_expr(expr: &Expression, ctx: &MacroCtx) -> Value {
    let span = expr.span().clone();
    let text = ctx
        .source
        .get(span.start..span.end)
        .unwrap_or_default()
        .to_string();
    let target = object(vec![
        ("kind".to_string(), Value::String("expression".into())),
        ("text".to_string(), Value::String(text.into())),
        ("node".to_string(), node_expr(expr, ctx)),
        ("span".to_string(), span_value_json(ctx, &span)),
    ]);
    object(vec![
        ("args".to_string(), value_list(Vec::new())),
        ("target".to_string(), target),
    ])
}

fn scalar_or_node(expr: &Expression, ctx: &MacroCtx) -> Value {
    match expr {
        Expression::Literal { value, .. } => match value {
            xulo_core::ast::Literal::String(s) => Value::String(s.clone().into()),
            xulo_core::ast::Literal::Number(n) => Value::Number(*n),
            xulo_core::ast::Literal::Boolean(b) => Value::Boolean(*b),
            xulo_core::ast::Literal::Null => Value::Enum {
                enum_name: "ast::Value".to_string(),
                tag: "Unit".to_string(),
                payload: None,
            },
            _ => node_expr(expr, ctx),
        },
        _ => node_expr(expr, ctx),
    }
}

fn node_expr(expr: &Expression, ctx: &MacroCtx) -> Value {
    let span = expr.span().clone();
    let text = ctx.source.get(span.start..span.end).map(|s| s.to_string());
    Value::AstNode(Rc::new(AstNodeValue {
        kind: "Expression".to_string(),
        span: ctx.span(&span),
        text,
        fields: Vec::new(),
        repr: AstNodeRepr::Expression(Box::new(expr.clone())),
    }))
}

// ---------------------------------------------------------------------------
// Output records
// ---------------------------------------------------------------------------

fn result_ok(value: &Value) -> Option<&Value> {
    match value {
        Value::Enum {
            enum_name,
            tag,
            payload,
        } if enum_name == "Result" && tag == "Ok" => payload.as_deref(),
        _ => None,
    }
}

fn macro_failure(value: &Value, name: &str) -> Option<XuloError> {
    let Value::Enum {
        enum_name,
        tag,
        payload,
    } = value
    else {
        return None;
    };
    if enum_name != "Result" || tag != "Err" {
        return None;
    }
    let payload = payload.as_deref()?;
    let fields = value_fields(payload)?;
    let message = get_str(&fields, "message").unwrap_or_default();
    let span = get_field(&fields, "span").and_then(|v| span_from_value(&v));
    let mut err = error(
        "E1008",
        format!("macro `{name}` reported an error"),
        span.clone().unwrap_or(0..0),
    )
    .with_note(message);
    if let Some(span) = span {
        err = err.at(span);
    }
    Some(err)
}

fn derive_output(value: Value, name: &str) -> Result<Vec<Statement>, XuloError> {
    let payload = result_ok(&value).ok_or_else(|| {
        macro_failure(&value, name).unwrap_or_else(|| {
            error(
                "E1006",
                format!("macro `{name}` did not return `Result::Ok(ast::DeriveOutput)`"),
                0..0,
            )
        })
    })?;
    let fields = value_fields(payload).ok_or_else(|| {
        error(
            "E1006",
            format!("macro `{name}` returned a malformed output record"),
            0..0,
        )
    })?;
    let mut out = Vec::new();
    if let Some(items) = get_field(&fields, "items") {
        for item in list_items(&items) {
            out.push(statement_from_node(&item, name)?);
        }
    }
    Ok(out)
}

fn attribute_output(value: Value, name: &str) -> Result<(Statement, Vec<Statement>), XuloError> {
    let payload = result_ok(&value).ok_or_else(|| {
        macro_failure(&value, name).unwrap_or_else(|| {
            error(
                "E1006",
                format!("macro `{name}` did not return `Result::Ok(ast::AttributeOutput)`"),
                0..0,
            )
        })
    })?;
    let fields = value_fields(payload).ok_or_else(|| {
        error(
            "E1006",
            format!("macro `{name}` returned a malformed output record"),
            0..0,
        )
    })?;
    let item = get_field(&fields, "item")
        .ok_or_else(|| error("E1006", format!("macro `{name}` returned no `item`"), 0..0))?;
    let item = statement_from_node(&item, name)?;
    let mut extras = Vec::new();
    if let Some(list) = get_field(&fields, "extras") {
        for extra in list_items(&list) {
            extras.push(statement_from_node(&extra, name)?);
        }
    }
    Ok((item, extras))
}

fn function_output(value: Value, name: &str) -> Result<Expression, XuloError> {
    let payload = result_ok(&value).ok_or_else(|| {
        macro_failure(&value, name).unwrap_or_else(|| {
            error(
                "E1006",
                format!("macro `{name}` did not return `Result::Ok(ast::FunctionOutput)`"),
                0..0,
            )
        })
    })?;
    let fields = value_fields(payload).ok_or_else(|| {
        error(
            "E1006",
            format!("macro `{name}` returned a malformed output record"),
            0..0,
        )
    })?;
    let expr = get_field(&fields, "expr")
        .ok_or_else(|| error("E1006", format!("macro `{name}` returned no `expr`"), 0..0))?;
    let Value::AstNode(node) = expr else {
        return Err(error(
            "E1006",
            format!("macro `{name}` returned a non-node expression"),
            0..0,
        ));
    };
    match &node.repr {
        AstNodeRepr::Expression(e) => Ok((**e).clone()),
        AstNodeRepr::Constructed => constructed_expression(&node).ok_or_else(|| {
            error(
                "E1006",
                format!(
                    "macro `{name}` output node `{}` is not an expression",
                    node.kind
                ),
                0..0,
            )
        }),
        _ => Err(error(
            "E1006",
            format!("macro `{name}` output is not an expression"),
            0..0,
        )),
    }
}

/// Unwrap an `ast::Value` enum produced by macro code to its plain payload.
fn unwrap_ast_value(value: &Value) -> Value {
    if let Value::Enum {
        enum_name,
        tag,
        payload,
    } = value
        && enum_name == "ast::Value"
    {
        return match tag.as_str() {
            "Unit" => Value::Null,
            "Bool" | "Int" | "Float" | "String" | "AstNode" | "List" | "Map" => {
                payload.as_deref().cloned().unwrap_or(Value::Null)
            }
            _ => Value::Null,
        };
    }
    value.clone()
}

/// Convert a node built by `ast::node` into an expression, for the kinds the
/// macro surface constructs directly (spec memory-and-runtime.md, "AST
/// values"; expressions/calls.md shows `StringLiteral`).
fn constructed_expression(node: &AstNodeValue) -> Option<Expression> {
    let field = |name: &str| get_field(&node.fields, name).map(|v| unwrap_ast_value(&v));
    let text = |name: &str| match field(name) {
        Some(Value::String(s)) => Some(s.to_string()),
        Some(Value::Number(n)) => Some(crate::value::format_number(n)),
        _ => None,
    };
    let span = node.span.start..node.span.end;
    let literal = |value| {
        Some(Expression::Literal {
            value,
            span: span.clone(),
        })
    };
    match node.kind.as_str() {
        "StringLiteral" | "String" => literal(xulo_core::ast::Literal::String(text("text")?)),
        "NumberLiteral" | "IntLiteral" | "FloatLiteral" => {
            literal(xulo_core::ast::Literal::Number(text("text")?.parse().ok()?))
        }
        "BooleanLiteral" | "Bool" => literal(xulo_core::ast::Literal::Boolean(matches!(
            field("text"),
            Some(Value::Boolean(true))
        ))),
        "NullLiteral" | "Null" => literal(xulo_core::ast::Literal::Null),
        "Identifier" | "IdentifierExpr" => Some(Expression::Identifier {
            name: text("name")?,
            span: span.clone(),
        }),
        "Expression" => {
            let source = text("text")?;
            match xulo_parser::parse_quote_fragment(&source).ok()? {
                xulo_parser::QuoteFragment::Expression(e) => Some(e),
                _ => None,
            }
        }
        _ => None,
    }
}

fn statement_from_node(value: &Value, name: &str) -> Result<Statement, XuloError> {
    let Value::AstNode(node) = value else {
        return Err(error(
            "E1006",
            format!("macro `{name}` returned a non-node item"),
            0..0,
        ));
    };
    match &node.repr {
        AstNodeRepr::Statement(s) => Ok((**s).clone()),
        _ => Err(error(
            "E1006",
            format!("macro `{name}` output is not a declaration"),
            0..0,
        )),
    }
}

fn get_field(fields: &[(String, Value)], name: &str) -> Option<Value> {
    fields
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.clone())
}

fn get_str(fields: &[(String, Value)], name: &str) -> Option<String> {
    match get_field(fields, name)? {
        Value::String(s) => Some(s.to_string()),
        _ => None,
    }
}

fn value_fields(value: &Value) -> Option<Vec<(String, Value)>> {
    match value {
        Value::Object(fields) => Some(fields.borrow().clone()),
        Value::AstNode(node) => Some(node.fields.clone()),
        _ => None,
    }
}

fn list_items(value: &Value) -> Vec<Value> {
    match value {
        Value::List(items) => items.borrow().clone(),
        _ => Vec::new(),
    }
}

fn span_from_value(value: &Value) -> Option<Range<usize>> {
    let fields = value_fields(value)?;
    let start = match get_field(&fields, "start")? {
        Value::Number(n) => n as usize,
        _ => return None,
    };
    let end = match get_field(&fields, "end")? {
        Value::Number(n) => n as usize,
        _ => return None,
    };
    Some(start..end)
}

// ---------------------------------------------------------------------------
// The `ast` namespace at compile-time execution
// ---------------------------------------------------------------------------

/// Install the `ast` namespace and record types for ordinary (non-macro)
/// execution: a `#[test]` may construct an input record and call the macro
/// implementation directly (spec testing.md, "Implementation tests"). The
/// `ast::call_*` entry points have no registry at runtime and trap.
pub fn install_runtime_ast(interp: &Interpreter) {
    const RECORDS: &[(&str, &[&str])] = &[
        ("ast::Span", &["file", "start", "end", "line", "column"]),
        ("ast::AstNode", &["kind", "span"]),
        ("ast::MacroFailure", &["message", "span"]),
        ("ast::AttributeArgInfo", &["name", "value"]),
        ("ast::AttributeInfo", &["name", "args", "span"]),
        ("ast::FieldInfo", &["name", "ty", "attrs", "span"]),
        ("ast::VariantInfo", &["name", "fields", "attrs", "span"]),
        (
            "ast::GenericParam",
            &["kind", "name", "bounds", "ty", "span"],
        ),
        (
            "ast::TypeInfo",
            &[
                "kind", "name", "fields", "variants", "generics", "attrs", "span",
            ],
        ),
        (
            "ast::ParamInfo",
            &["name", "ty", "mode", "has_default", "span"],
        ),
        (
            "ast::DeclarationInfo",
            &[
                "kind",
                "name",
                "node",
                "params",
                "return_type",
                "attrs",
                "trait_ref",
                "self_type",
                "span",
            ],
        ),
        ("ast::ExpressionInfo", &["kind", "text", "node", "span"]),
        ("ast::DeriveInput", &["target"]),
        ("ast::AttributeInput", &["args", "named_args", "target"]),
        ("ast::FunctionInput", &["args", "target"]),
        ("ast::DeriveOutput", &["items"]),
        ("ast::AttributeOutput", &["item", "extras"]),
        ("ast::FunctionOutput", &["expr"]),
    ];
    for (name, fields) in RECORDS {
        interp.register_macro_struct(name, fields);
    }
    interp.register_macro_enum("ast::Value");

    let native = |f: Box<NativeImpl>| Value::Native(Rc::from(f));
    let members: Vec<(String, Value)> = vec![
        (
            "node".to_string(),
            native(Box::new(|_i, args| {
                let kind = match args.first() {
                    Some(Value::String(s)) => s.to_string(),
                    _ => String::new(),
                };
                let fields = args.get(1).and_then(value_fields).unwrap_or_default();
                Ok(Value::AstNode(Rc::new(AstNodeValue {
                    kind,
                    span: SpanValue::empty(),
                    text: None,
                    fields,
                    repr: AstNodeRepr::Constructed,
                })))
            })),
        ),
        (
            "field".to_string(),
            native(Box::new(|_i, args| ast_field(args))),
        ),
        (
            "span_of".to_string(),
            native(Box::new(|_i, args| ast_span_of(args))),
        ),
        (
            "current_span".to_string(),
            native(Box::new(|_i, _args| Ok(span_object(&SpanValue::empty())))),
        ),
        (
            "compile_date".to_string(),
            native(Box::new(|_i, _args| {
                Ok(Value::String(compile_date().into()))
            })),
        ),
        (
            "lookup_type".to_string(),
            native(Box::new(|_i, _args| Ok(Value::Null))),
        ),
        (
            "lookup_function".to_string(),
            native(Box::new(|_i, _args| Ok(Value::Null))),
        ),
        (
            "lookup_variable".to_string(),
            native(Box::new(|_i, _args| Ok(Value::Null))),
        ),
        (
            "fields_of".to_string(),
            native(Box::new(|_i, _args| Ok(value_list(Vec::new())))),
        ),
        (
            "call_derive".to_string(),
            native(Box::new(|_i, _args| runtime_call_panic("call_derive"))),
        ),
        (
            "call_attribute".to_string(),
            native(Box::new(|_i, _args| runtime_call_panic("call_attribute"))),
        ),
        (
            "call_function".to_string(),
            native(Box::new(|_i, _args| runtime_call_panic("call_function"))),
        ),
    ];
    interp.root_env().borrow_mut().define(
        "ast",
        Value::Object(Rc::new(RefCell::new(members))),
        false,
    );
}

fn runtime_call_panic(name: &str) -> Result<Value, RunError> {
    Err(RunError::Err(
        XuloError::new(
            ErrorKind::Runtime,
            format!("`ast::{name}` is only available during compile-time macro expansion"),
        )
        .at(0..0),
    ))
}

fn install_ast(interp: &Interpreter, ctx: &MacroCtx) {
    let ctx = Rc::new(MacroCtxView {
        registry: ctx.registry.clone(),
        base: ctx.base.clone(),
        source: ctx.source.clone(),
        file: ctx.file.clone(),
        span: ctx.current_span.borrow().clone(),
        chain: ctx.chain.clone(),
        quota: ctx.quota.clone(),
    });
    let mut members: Vec<(String, Value)> = Vec::new();
    let native = |f: Box<NativeImpl>| Value::Native(Rc::from(f));
    {
        let c = ctx.clone();
        members.push((
            "call_derive".to_string(),
            native(Box::new(move |_i, args| {
                c.call_entry(MacroKind::Derive, args)
            })),
        ));
    }
    {
        let c = ctx.clone();
        members.push((
            "call_attribute".to_string(),
            native(Box::new(move |_i, args| {
                c.call_entry(MacroKind::Attribute, args)
            })),
        ));
    }
    {
        let c = ctx.clone();
        members.push((
            "call_function".to_string(),
            native(Box::new(move |_i, args| {
                c.call_entry(MacroKind::Function, args)
            })),
        ));
    }
    {
        let c = ctx.clone();
        members.push((
            "node".to_string(),
            native(Box::new(move |_i, args| c.node(args))),
        ));
    }
    members.push((
        "field".to_string(),
        native(Box::new(|_i, args| ast_field(args))),
    ));
    {
        let c = ctx.clone();
        members.push((
            "current_span".to_string(),
            native(Box::new(move |_i, _args| Ok(span_object(&c.span)))),
        ));
    }
    members.push((
        "span_of".to_string(),
        native(Box::new(|_i, args| ast_span_of(args))),
    ));
    members.push((
        "compile_date".to_string(),
        native(Box::new(|_i, _args| {
            Ok(Value::String(compile_date().into()))
        })),
    ));
    {
        let c = ctx.clone();
        members.push((
            "lookup_type".to_string(),
            native(Box::new(move |_i, args| c.lookup_type(args))),
        ));
    }
    {
        let c = ctx.clone();
        members.push((
            "lookup_function".to_string(),
            native(Box::new(move |_i, args| c.lookup_function(args))),
        ));
    }
    {
        let c = ctx.clone();
        members.push((
            "lookup_variable".to_string(),
            native(Box::new(move |_i, args| c.lookup_variable(args))),
        ));
    }
    {
        let c = ctx.clone();
        members.push((
            "fields_of".to_string(),
            native(Box::new(move |_i, args| c.fields_of(args))),
        ));
    }
    interp.root_env().borrow_mut().define(
        "ast",
        Value::Object(Rc::new(RefCell::new(members))),
        false,
    );
}

/// A snapshot of the expansion context that native functions can capture.
struct MacroCtxView {
    registry: Rc<MacroRegistry>,
    base: Rc<Program>,
    source: String,
    file: String,
    span: SpanValue,
    chain: Rc<RefCell<Vec<String>>>,
    quota: Rc<RefCell<Option<Rc<QuotaState>>>>,
}

impl MacroCtxView {
    fn call_entry(&self, kind: MacroKind, args: &[Value]) -> Result<Value, RunError> {
        if args.len() != 2 {
            return Err(RunError::Err(error(
                "E0207",
                format!("`ast::call_{}` expects 2 arguments", kind.name()),
                0..0,
            )));
        }
        let name = match &args[0] {
            Value::String(s) => s.to_string(),
            other => {
                return Err(RunError::Err(error(
                    "E1002",
                    format!("macro name must be a string, found {}", other.kind_name()),
                    0..0,
                )));
            }
        };
        let Some(def) = self.registry.get(&name) else {
            return Err(RunError::Err(error(
                "E1005",
                format!("cannot find macro `{name}`"),
                0..0,
            )));
        };
        if def.kind != kind {
            return Err(RunError::Err(error(
                "E1006",
                format!(
                    "`{name}` is a {} macro, not a {} macro",
                    def.kind.name(),
                    kind.name()
                ),
                0..0,
            )));
        }
        let ctx = MacroCtx {
            registry: self.registry.clone(),
            base: self.base.clone(),
            source: self.source.clone(),
            file: self.file.clone(),
            chain: self.chain.clone(),
            current_span: RefCell::new(self.span.clone()),
            quota: self.quota.clone(),
        };
        let def = def.clone();
        let input = match &args[1] {
            Value::AstNode(node) => self.convert_input(kind, node)?,
            other => other.clone(),
        };
        let value = invoke_macro(&ctx, &def, input).map_err(RunError::Err)?;
        Ok(value)
    }

    /// Convert an `ast::AstNode` input to the entry point's typed input
    /// record (spec expressions/calls.md, the conversion table). A node with
    /// no original declaration or expression is `E1002`.
    fn convert_input(&self, kind: MacroKind, node: &AstNodeValue) -> Result<Value, RunError> {
        let ctx = self.as_ctx();
        match kind {
            MacroKind::Derive => match &node.repr {
                AstNodeRepr::Statement(statement)
                    if matches!(
                        inner_decl(statement),
                        InnerDecl::Struct(_) | InnerDecl::Enum(_)
                    ) =>
                {
                    Ok(derive_input(statement, &ctx))
                }
                _ => Err(conversion_error(kind, &node.kind)),
            },
            MacroKind::Attribute => match &node.repr {
                AstNodeRepr::Statement(statement) => {
                    let empty = Attribute {
                        name: String::new(),
                        name_span: 0..0,
                        args: Vec::new(),
                        span: 0..0,
                        has_parens: false,
                    };
                    Ok(attribute_input(statement, &empty, &ctx))
                }
                _ => Err(conversion_error(kind, &node.kind)),
            },
            MacroKind::Function => match &node.repr {
                AstNodeRepr::Expression(expr) => Ok(function_input_from_expr(expr, &ctx)),
                _ => Err(conversion_error(kind, &node.kind)),
            },
        }
    }

    fn node(&self, args: &[Value]) -> Result<Value, RunError> {
        let kind = match args.first() {
            Some(Value::String(s)) => s.to_string(),
            _ => {
                return Err(RunError::Err(error(
                    "E0207",
                    "`ast::node` expects a kind string",
                    0..0,
                )));
            }
        };
        let fields = args.get(1).and_then(value_fields).unwrap_or_default();
        Ok(Value::AstNode(Rc::new(AstNodeValue {
            kind,
            span: self.span.clone(),
            text: None,
            fields,
            repr: AstNodeRepr::Constructed,
        })))
    }

    fn lookup_type(&self, args: &[Value]) -> Result<Value, RunError> {
        let name = match args.first() {
            Some(Value::String(s)) => s.to_string(),
            _ => return Ok(Value::Null),
        };
        for stmt in &self.base.statements {
            if let InnerDecl::Struct(s) = inner_decl(stmt)
                && s.name == name
            {
                let ctx = self.as_ctx();
                return Ok(type_info(stmt, &ctx));
            }
            if let InnerDecl::Enum(e) = inner_decl(stmt)
                && e.name == name
            {
                let ctx = self.as_ctx();
                return Ok(type_info(stmt, &ctx));
            }
        }
        Ok(Value::Null)
    }

    fn lookup_function(&self, args: &[Value]) -> Result<Value, RunError> {
        let name = match args.first() {
            Some(Value::String(s)) => s.to_string(),
            _ => return Ok(Value::Null),
        };
        for stmt in &self.base.statements {
            if let InnerDecl::Fn(f) = inner_decl(stmt)
                && f.name == name
            {
                return Ok(Value::String(function_signature(f).into()));
            }
        }
        Ok(Value::Null)
    }

    fn lookup_variable(&self, args: &[Value]) -> Result<Value, RunError> {
        let name = match args.first() {
            Some(Value::String(s)) => s.to_string(),
            _ => return Ok(Value::Null),
        };
        for stmt in &self.base.statements {
            if let InnerDecl::Let(b) = inner_decl(stmt)
                && b.name == name
            {
                return Ok(Value::String(
                    b.type_annotation
                        .as_ref()
                        .map(declared_type)
                        .unwrap_or_default()
                        .into(),
                ));
            }
        }
        Ok(Value::Null)
    }

    fn fields_of(&self, args: &[Value]) -> Result<Value, RunError> {
        let name = match args.first() {
            Some(Value::String(s)) => s.to_string(),
            _ => return Ok(value_list(Vec::new())),
        };
        let ctx = self.as_ctx();
        for stmt in &self.base.statements {
            if let InnerDecl::Struct(s) = inner_decl(stmt)
                && s.name == name
            {
                return Ok(value_list(
                    s.fields
                        .iter()
                        .map(|f| field_info(&f.name, &f.type_, &[], &f.name_span, &ctx))
                        .collect(),
                ));
            }
        }
        Ok(value_list(Vec::new()))
    }

    fn as_ctx(&self) -> MacroCtx {
        MacroCtx {
            registry: self.registry.clone(),
            base: self.base.clone(),
            source: self.source.clone(),
            file: self.file.clone(),
            chain: self.chain.clone(),
            current_span: RefCell::new(self.span.clone()),
            quota: self.quota.clone(),
        }
    }
}

fn span_object(span: &SpanValue) -> Value {
    object(vec![
        ("file".to_string(), Value::String(span.file.clone().into())),
        ("start".to_string(), Value::Number(span.start as f64)),
        ("end".to_string(), Value::Number(span.end as f64)),
        ("line".to_string(), Value::Number(span.line as f64)),
        ("column".to_string(), Value::Number(span.column as f64)),
    ])
}

fn ast_field(args: &[Value]) -> Result<Value, RunError> {
    let Some(Value::AstNode(node)) = args.first() else {
        return Ok(Value::Null);
    };
    let name = match args.get(1) {
        Some(Value::String(s)) => s.to_string(),
        _ => return Ok(Value::Null),
    };
    Ok(get_field(&node.fields, &name).unwrap_or(Value::Null))
}

fn ast_span_of(args: &[Value]) -> Result<Value, RunError> {
    let Some(Value::AstNode(node)) = args.first() else {
        return Ok(span_object(&SpanValue::empty()));
    };
    Ok(span_object(&node.span))
}

fn function_signature(f: &FnDef) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            format!(
                "{}: {}",
                p.name,
                p.type_annotation
                    .as_ref()
                    .map(declared_type)
                    .unwrap_or_else(|| "unknown".to_string())
            )
        })
        .collect();
    let ret = f
        .return_type
        .as_ref()
        .map(|t| format!(": {}", declared_type(t)))
        .unwrap_or_default();
    format!("fn {}({}){}", f.name, params.join(", "), ret)
}

fn compile_date() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

// ---------------------------------------------------------------------------
// Declaration helpers and AST traversal
// ---------------------------------------------------------------------------

fn add_ast_type_decls(program: &mut Program) {
    let empty_span = 0..0;
    let mut decls = Vec::new();
    let records: &[(&str, &[&str])] = &[
        ("ast::Span", &["file", "start", "end", "line", "column"]),
        ("ast::AstNode", &["kind", "span"]),
        ("ast::MacroFailure", &["message", "span"]),
        ("ast::AttributeArgInfo", &["name", "value"]),
        ("ast::AttributeInfo", &["name", "args", "span"]),
        ("ast::FieldInfo", &["name", "ty", "attrs", "span"]),
        ("ast::VariantInfo", &["name", "fields", "attrs", "span"]),
        (
            "ast::GenericParam",
            &["kind", "name", "bounds", "ty", "span"],
        ),
        (
            "ast::TypeInfo",
            &[
                "kind", "name", "fields", "variants", "generics", "attrs", "span",
            ],
        ),
        (
            "ast::ParamInfo",
            &["name", "ty", "mode", "has_default", "span"],
        ),
        (
            "ast::DeclarationInfo",
            &[
                "kind",
                "name",
                "node",
                "params",
                "return_type",
                "attrs",
                "trait_ref",
                "self_type",
                "span",
            ],
        ),
        ("ast::ExpressionInfo", &["kind", "text", "node", "span"]),
        ("ast::DeriveInput", &["target"]),
        ("ast::AttributeInput", &["args", "named_args", "target"]),
        ("ast::FunctionInput", &["args", "target"]),
        ("ast::DeriveOutput", &["items"]),
        ("ast::AttributeOutput", &["item", "extras"]),
        ("ast::FunctionOutput", &["expr"]),
    ];
    for (name, fields) in records {
        decls.push(Statement::Struct(StructDef {
            name: name.to_string(),
            name_span: empty_span.clone(),
            type_params: Vec::new(),
            generic_params: Vec::new(),
            type_bounds: Vec::new(),
            fields: fields
                .iter()
                .map(|f| xulo_core::ast::StructField {
                    name: f.to_string(),
                    name_span: empty_span.clone(),
                    type_: Type::Any,
                    default: None,
                })
                .collect(),
            span: empty_span.clone(),
            attributes: Vec::new(),
        }));
    }
    // `ast::Value` is an enum the macro surface constructs and matches.
    decls.push(Statement::Enum(EnumDef {
        name: "ast::Value".to_string(),
        name_span: empty_span.clone(),
        type_params: Vec::new(),
        generic_params: Vec::new(),
        type_bounds: Vec::new(),
        variants: [
            "Unit", "Bool", "Int", "Float", "String", "List", "Map", "AstNode",
        ]
        .iter()
        .map(|n| xulo_core::ast::EnumVariant {
            name: n.to_string(),
            name_span: empty_span.clone(),
            payload: None,
        })
        .collect(),
        attributes: Vec::new(),
    }));
    program.statements.extend(decls);
}

enum InnerDecl<'a> {
    Fn(&'a FnDef),
    Struct(&'a StructDef),
    Enum(&'a EnumDef),
    Trait(&'a xulo_core::ast::TraitDecl),
    Impl(&'a ImplDecl),
    Type(&'a TypeAlias),
    Let(&'a LetBinding),
    Other,
}

fn inner_decl(statement: &Statement) -> InnerDecl<'_> {
    match statement {
        Statement::Fn(f) => InnerDecl::Fn(f),
        Statement::Struct(s) => InnerDecl::Struct(s),
        Statement::Enum(e) => InnerDecl::Enum(e),
        Statement::Trait(t) => InnerDecl::Trait(t),
        Statement::Impl(i) => InnerDecl::Impl(i),
        Statement::TypeAlias(t) => InnerDecl::Type(t),
        Statement::Let(b) => InnerDecl::Let(b),
        Statement::Export(export) => match &export.item {
            xulo_core::ast::ExportItem::Fn(f) => InnerDecl::Fn(f),
            xulo_core::ast::ExportItem::Struct(s) => InnerDecl::Struct(s),
            xulo_core::ast::ExportItem::Enum(e) => InnerDecl::Enum(e),
            xulo_core::ast::ExportItem::Trait(t) => InnerDecl::Trait(t),
            xulo_core::ast::ExportItem::Type(t) => InnerDecl::Type(t),
            xulo_core::ast::ExportItem::Let(b) => InnerDecl::Let(b),
            _ => InnerDecl::Other,
        },
        _ => InnerDecl::Other,
    }
}

fn host_kind(statement: &Statement) -> Option<String> {
    Some(
        match inner_decl(statement) {
            InnerDecl::Fn(_) => "fn",
            InnerDecl::Struct(_) => "struct",
            InnerDecl::Enum(_) => "enum",
            InnerDecl::Trait(_) => "trait",
            InnerDecl::Impl(_) => "impl",
            InnerDecl::Type(_) => "type",
            InnerDecl::Let(b) if b.is_const => "const",
            InnerDecl::Let(_) => "let",
            InnerDecl::Other => return None,
        }
        .to_string(),
    )
}

fn statement_kind(statement: &Statement) -> &'static str {
    match statement {
        Statement::Fn(_) => "FnDeclaration",
        Statement::Struct(_) => "StructDeclaration",
        Statement::Enum(_) => "EnumDeclaration",
        Statement::Trait(_) => "TraitDeclaration",
        Statement::Impl(_) => "ImplDeclaration",
        Statement::TypeAlias(_) => "TypeAlias",
        Statement::Let(b) if b.is_const => "ConstDeclaration",
        Statement::Let(_) => "LetDeclaration",
        _ => "Declaration",
    }
}

fn statement_attrs(statement: &Statement) -> Option<&Vec<Attribute>> {
    match statement {
        Statement::Fn(f) => Some(&f.attributes),
        Statement::Struct(s) => Some(&s.attributes),
        Statement::Enum(e) => Some(&e.attributes),
        Statement::Trait(t) => Some(&t.attributes),
        Statement::Impl(i) => Some(&i.attributes),
        Statement::TypeAlias(t) => Some(&t.attributes),
        Statement::Let(b) => Some(&b.attributes),
        Statement::Export(export) => match &export.item {
            xulo_core::ast::ExportItem::Fn(f) => Some(&f.attributes),
            xulo_core::ast::ExportItem::Struct(s) => Some(&s.attributes),
            xulo_core::ast::ExportItem::Enum(e) => Some(&e.attributes),
            xulo_core::ast::ExportItem::Trait(t) => Some(&t.attributes),
            xulo_core::ast::ExportItem::Type(t) => Some(&t.attributes),
            xulo_core::ast::ExportItem::Let(b) => Some(&b.attributes),
            _ => None,
        },
        _ => None,
    }
}

fn statement_attrs_mut(statement: &mut Statement) -> Option<&mut Vec<Attribute>> {
    match statement {
        Statement::Fn(f) => Some(&mut f.attributes),
        Statement::Struct(s) => Some(&mut s.attributes),
        Statement::Enum(e) => Some(&mut e.attributes),
        Statement::Trait(t) => Some(&mut t.attributes),
        Statement::Impl(i) => Some(&mut i.attributes),
        Statement::TypeAlias(t) => Some(&mut t.attributes),
        Statement::Let(b) => Some(&mut b.attributes),
        Statement::Export(export) => match &mut export.item {
            xulo_core::ast::ExportItem::Fn(f) => Some(&mut f.attributes),
            xulo_core::ast::ExportItem::Struct(s) => Some(&mut s.attributes),
            xulo_core::ast::ExportItem::Enum(e) => Some(&mut e.attributes),
            xulo_core::ast::ExportItem::Trait(t) => Some(&mut t.attributes),
            xulo_core::ast::ExportItem::Type(t) => Some(&mut t.attributes),
            xulo_core::ast::ExportItem::Let(b) => Some(&mut b.attributes),
            _ => None,
        },
        _ => None,
    }
}

fn statement_span(statement: &Statement) -> Range<usize> {
    match statement {
        Statement::Fn(f) => f.span.clone(),
        Statement::Struct(s) => s.span.clone(),
        Statement::Enum(e) => e.name_span.clone(),
        Statement::Trait(t) => t.span.clone(),
        Statement::Impl(i) => i.span.clone(),
        Statement::TypeAlias(t) => t.name_span.clone(),
        Statement::Let(b) => b.name_span.clone(),
        _ => 0..0,
    }
}

fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for ch in source.get(..offset).unwrap_or_default().chars() {
        if ch == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

// ---------------------------------------------------------------------------
// Traversal
// ---------------------------------------------------------------------------

fn map_program(
    program: &mut Program,
    f: &mut dyn FnMut(&mut Expression) -> Result<(), XuloError>,
) -> Result<(), XuloError> {
    for stmt in &mut program.statements {
        map_statement(stmt, f)?;
    }
    Ok(())
}

/// The first sandbox-forbidden operation in a macro body (spec
/// `builtins/sandbox.md`), with the resource it names.
fn forbidden_resource(body: &Block) -> Option<(String, Range<usize>)> {
    let mut forbidden = None;
    let mut probe = Block {
        statements: body.statements.clone(),
    };
    let _ = map_block(&mut probe, &mut |expr| {
        if forbidden.is_some() {
            return Ok(());
        }
        match expr {
            Expression::Spawn(_) => {
                forbidden = Some(("concurrency".to_string(), expr.span().clone()));
            }
            Expression::Lock(_) => {
                forbidden = Some(("concurrency".to_string(), expr.span().clone()));
            }
            Expression::Call(call) => {
                let callee = call.callee.as_str();
                let resource = if callee.starts_with("std::fs") {
                    Some("the filesystem")
                } else if callee.starts_with("std::net") || callee.starts_with("std::http") {
                    Some("the network")
                } else if callee.starts_with("std::os") {
                    Some("the environment")
                } else if callee.starts_with("std::process") {
                    Some("processes")
                } else if callee.starts_with("std::time")
                    || callee.starts_with("Time::")
                    || callee.starts_with("Task::")
                {
                    Some("wall-clock time or concurrency")
                } else if matches!(callee, "Math::random" | "Math::random_int") {
                    Some("randomness")
                } else {
                    None
                };
                if let Some(resource) = resource {
                    forbidden = Some((resource.to_string(), call.span.clone()));
                }
            }
            _ => {}
        }
        Ok(())
    });
    forbidden
}

fn body_contains_macro_call(body: &Block) -> bool {
    let mut found = false;
    let mut probe = Block {
        statements: body.statements.clone(),
    };
    let _ = map_block(&mut probe, &mut |expr| {
        if matches!(expr, Expression::MacroCall(_)) {
            found = true;
        }
        Ok(())
    });
    found
}

fn map_block(
    block: &mut Block,
    f: &mut dyn FnMut(&mut Expression) -> Result<(), XuloError>,
) -> Result<(), XuloError> {
    for stmt in &mut block.statements {
        map_statement(stmt, f)?;
    }
    Ok(())
}

fn map_statement(
    stmt: &mut Statement,
    f: &mut dyn FnMut(&mut Expression) -> Result<(), XuloError>,
) -> Result<(), XuloError> {
    match stmt {
        Statement::Fn(fd) => map_block(&mut fd.body, f)?,
        Statement::Test(fd) => map_block(&mut fd.body, f)?,
        Statement::Let(b) => {
            if let Some(v) = &mut b.value {
                map_expr(v, f)?;
            }
            if let Some(deps) = &mut b.memo_deps {
                for d in deps {
                    map_expr(d, f)?;
                }
            }
        }
        Statement::Return(r) => {
            if let Some(v) = &mut r.value {
                map_expr(v, f)?;
            }
        }
        Statement::For(loop_) => {
            map_expr(&mut loop_.iterable, f)?;
            map_block(&mut loop_.body, f)?;
        }
        Statement::While(loop_) => {
            map_expr(&mut loop_.condition, f)?;
            map_block(&mut loop_.body, f)?;
        }
        Statement::Assign(a) => {
            if let xulo_core::ast::AssignValue::Expr(e) = &mut a.value {
                map_expr(e, f)?;
            } else if let xulo_core::ast::AssignValue::Assign(inner) = &mut a.value {
                map_assign(inner, f)?;
            }
            match &mut a.target {
                xulo_core::ast::AssignTarget::Member(obj, _) => map_expr(obj, f)?,
                xulo_core::ast::AssignTarget::Index(obj, idx) => {
                    map_expr(obj, f)?;
                    map_expr(idx, f)?;
                }
                xulo_core::ast::AssignTarget::Name(_) => {}
            }
        }
        Statement::TypeAlias(_) => {}
        Statement::Enum(_) | Statement::Struct(_) | Statement::Trait(_) => {}
        Statement::Impl(i) => {
            for m in &mut i.methods {
                map_block(&mut m.body, f)?;
            }
        }
        Statement::Expr(e) => map_expr(&mut e.expr, f)?,
        Statement::Block(b) => map_block(b, f)?,
        Statement::Try(t) => {
            map_block(&mut t.try_block, f)?;
            map_block(&mut t.catch_block, f)?;
        }
        Statement::Throw(e) => map_expr(e, f)?,
        Statement::Export(export) => match &mut export.item {
            xulo_core::ast::ExportItem::Fn(fd) => map_block(&mut fd.body, f)?,
            xulo_core::ast::ExportItem::Let(b) => {
                if let Some(v) = &mut b.value {
                    map_expr(v, f)?;
                }
            }
            _ => {}
        },
        Statement::State(s) => {
            if let Some(v) = &mut s.binding.value {
                map_expr(v, f)?;
            }
        }
        Statement::Store(s) => map_expr(&mut s.value, f)?,
        Statement::Effect(e) => {
            map_block(&mut e.closure.body, f)?;
            if let Some(deps) = &mut e.deps {
                for d in deps {
                    map_expr(d, f)?;
                }
            }
        }
        Statement::Environment(_) => {}
        Statement::Component(c) => {
            for arg in &mut c.args {
                map_expr(&mut arg.value, f)?;
            }
            for child in &mut c.children {
                map_ui_element(child, f)?;
            }
        }
        Statement::Mod(m) => {
            if let Some(body) = &mut m.body {
                for stmt in body {
                    map_statement(stmt, f)?;
                }
            }
        }
        Statement::Use(_) | Statement::ExternBlock(_) | Statement::Break | Statement::Continue => {}
        Statement::Defer(d) => match &mut d.body {
            xulo_core::ast::DeferBody::Block(b) => map_block(b, f)?,
            xulo_core::ast::DeferBody::Expr(e) => map_expr(e, f)?,
        },
    }
    Ok(())
}

fn map_assign(
    assign: &mut xulo_core::ast::AssignStmt,
    f: &mut dyn FnMut(&mut Expression) -> Result<(), XuloError>,
) -> Result<(), XuloError> {
    match &mut assign.value {
        xulo_core::ast::AssignValue::Expr(e) => map_expr(e, f),
        xulo_core::ast::AssignValue::Assign(inner) => map_assign(inner, f),
    }
}

fn map_ui_element(
    element: &mut xulo_core::ast::UiElement,
    f: &mut dyn FnMut(&mut Expression) -> Result<(), XuloError>,
) -> Result<(), XuloError> {
    match element {
        xulo_core::ast::UiElement::Expr(e) => map_expr(e, f),
        xulo_core::ast::UiElement::If {
            condition,
            then_branch,
            else_branch,
        } => {
            map_expr(condition, f)?;
            for child in then_branch {
                map_ui_element(child, f)?;
            }
            if let Some(else_branch) = else_branch {
                for child in else_branch {
                    map_ui_element(child, f)?;
                }
            }
            Ok(())
        }
        xulo_core::ast::UiElement::For { iterable, body, .. } => {
            map_expr(iterable, f)?;
            for child in body {
                map_ui_element(child, f)?;
            }
            Ok(())
        }
        xulo_core::ast::UiElement::Group(children) => {
            for child in children {
                map_ui_element(child, f)?;
            }
            Ok(())
        }
        xulo_core::ast::UiElement::Component(c) => {
            for arg in &mut c.args {
                map_expr(&mut arg.value, f)?;
            }
            for child in &mut c.children {
                map_ui_element(child, f)?;
            }
            Ok(())
        }
        xulo_core::ast::UiElement::Text(_) => Ok(()),
    }
}

fn map_expr(
    expr: &mut Expression,
    f: &mut dyn FnMut(&mut Expression) -> Result<(), XuloError>,
) -> Result<(), XuloError> {
    match expr {
        Expression::Literal { value, .. } => {
            if let xulo_core::ast::Literal::List(items) = value {
                for item in items {
                    map_expr(item, f)?;
                }
            } else if let xulo_core::ast::Literal::Object(fields) = value {
                for field in fields {
                    match field {
                        xulo_core::ast::ObjectField::Field { value, .. } => map_expr(value, f)?,
                        xulo_core::ast::ObjectField::Spread { value } => map_expr(value, f)?,
                    }
                }
            }
        }
        Expression::BinaryOp(b) => {
            map_expr(&mut b.left, f)?;
            map_expr(&mut b.right, f)?;
        }
        Expression::Unary(u) => map_expr(&mut u.operand, f)?,
        Expression::Call(c) => {
            if let Some(object) = &mut c.object {
                map_expr(object, f)?;
            }
            for arg in &mut c.arguments {
                map_expr(&mut arg.value, f)?;
            }
        }
        Expression::If(e) => {
            map_expr(&mut e.condition, f)?;
            map_block(&mut e.then_branch, f)?;
            if let Some(else_branch) = &mut e.else_branch {
                map_block(else_branch, f)?;
            }
        }
        Expression::Ternary(t) => {
            map_expr(&mut t.condition, f)?;
            map_expr(&mut t.then_value, f)?;
            map_expr(&mut t.else_value, f)?;
        }
        Expression::Match(m) => {
            map_expr(&mut m.value, f)?;
            for arm in &mut m.arms {
                if let Some(guard) = &mut arm.guard {
                    map_expr(guard, f)?;
                }
                match &mut arm.value {
                    xulo_core::ast::MatchBody::Expr(e) => map_expr(e, f)?,
                    xulo_core::ast::MatchBody::Block(b) => map_block(b, f)?,
                }
            }
        }
        Expression::Member(m) => map_expr(&mut m.object, f)?,
        Expression::Index(i) => {
            map_expr(&mut i.object, f)?;
            map_expr(&mut i.index, f)?;
        }
        Expression::Nullish(n) => {
            map_expr(&mut n.left, f)?;
            map_expr(&mut n.right, f)?;
        }
        Expression::Range(r) => {
            map_expr(&mut r.start, f)?;
            map_expr(&mut r.end, f)?;
        }
        Expression::Await { expr: inner, .. } => map_expr(inner, f)?,
        Expression::FnExpr(fd) => map_block(&mut fd.body, f)?,
        Expression::Spread { expr: inner, .. } => map_expr(inner, f)?,
        Expression::CallValue(c) => {
            map_expr(&mut c.callee, f)?;
            for arg in &mut c.arguments {
                map_expr(&mut arg.value, f)?;
            }
        }
        Expression::Component(c) => {
            for arg in &mut c.args {
                map_expr(&mut arg.value, f)?;
            }
            for child in &mut c.children {
                map_ui_element(child, f)?;
            }
        }
        Expression::Tuple { items, .. } => {
            for item in items {
                map_expr(item, f)?;
            }
        }
        Expression::TypedMap(tm) => {
            for entry in &mut tm.entries {
                map_expr(&mut entry.key, f)?;
                map_expr(&mut entry.value, f)?;
            }
        }
        Expression::Is(is) => map_expr(&mut is.expr, f)?,
        Expression::Panic { expr: inner, .. } => map_expr(inner, f)?,
        Expression::Propagate { expr: inner, .. } => map_expr(inner, f)?,
        Expression::Arrow(arrow) => match &mut arrow.body {
            xulo_core::ast::ArrowBody::Expr(e) => map_expr(e, f)?,
            xulo_core::ast::ArrowBody::Block(b) => map_block(b, f)?,
        },
        Expression::Spawn(spawn) => match &mut spawn.body {
            xulo_core::ast::SpawnBody::Async(b) => map_block(b, f)?,
            xulo_core::ast::SpawnBody::Service { fields, .. } => {
                for field in fields {
                    map_expr(&mut field.value, f)?;
                }
            }
        },
        Expression::Lock(lock) => map_block(&mut lock.body, f)?,
        Expression::MacroCall(call) => {
            for arg in &mut call.args {
                map_expr(arg, f)?;
            }
            f(expr)?;
            return Ok(());
        }
        Expression::Quote(quote) => {
            for piece in &mut quote.pieces {
                if let QuotePiece::Hole { expr, .. } = piece {
                    map_expr(expr, f)?;
                }
            }
        }
        Expression::Identifier { .. } | Expression::EnumRef(_) | Expression::Binding { .. } => {}
    }
    f(expr)
}
