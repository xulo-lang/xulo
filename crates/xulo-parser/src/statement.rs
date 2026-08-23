use xulo_core::ast::{
    AssignStmt, AssignTarget, BindingPattern, Block, ComponentStmt, EffectStmt, EnumDef,
    EnumPayloadParam, EnumVariant, EnvStmt, ExportItem, ExportStmt, ExprStmt, Expression, FnBound,
    FnDef, ForStmt, ImplDecl, ImportSpec, ImportStmt, LetBinding, Param, ReturnStmt, StateStmt,
    Statement, StoreStmt, TraitDecl, TraitMethod, TryStmt, TypeAlias, UiElement, WhileStmt,
};
use xulo_lexer::token::{LexedToken, Token};

use super::expression::{call_args, decode_string, expression, fn_expr, if_expr};
use super::types::type_expr;
use super::{
    In, PErr, Pr, at_eof, consumed_span, enter_nest, ident_name, ident_name_spanned, opt_tk,
    peek_is, tk, verified_tk,
};
use winnow::error::ErrMode;

/// A statement: `fn`/`let`/`const`/`type`/`enum` definitions, `return`, `for`,
/// `while`, assignment, `if`, block, or an expression statement — dispatched on
/// the leading token.
pub fn statement(input: &mut In<'_>) -> Pr<Statement> {
    let _guard = enter_nest(input)?;
    match input.first().map(|t| t.kind) {
        Some(Token::Fn) => {
            // `fn name(...)` is a definition; `fn(...)` in statement position is
            // an anonymous function expression (e.g. a trailing implicit return).
            if matches!(input.get(1).map(|t| t.kind), Some(Token::LParen)) {
                let original = *input;
                let e = expression(input)?;
                let span = consumed_span(original, input, 0);
                Ok(Statement::Expr(ExprStmt {
                    expr: e,
                    has_semicolon: false,
                    span,
                }))
            } else {
                fn_def(input).map(Statement::Fn)
            }
        }
        Some(Token::Let) => let_binding(input, false).map(Statement::Let),
        Some(Token::Const) => let_binding(input, true).map(Statement::Let),
        Some(Token::Type) => type_alias(input).map(Statement::TypeAlias),
        Some(Token::Enum) => enum_def(input).map(Statement::Enum),
        Some(Token::Trait) => trait_def(input).map(Statement::Trait),
        Some(Token::Impl) => impl_def(input).map(Statement::Impl),
        Some(Token::Return) => return_stmt(input).map(Statement::Return),
        Some(Token::For) => for_stmt(input).map(Statement::For),
        Some(Token::While) => while_stmt(input).map(Statement::While),
        Some(Token::Break) => break_stmt(input),
        Some(Token::Continue) => continue_stmt(input),
        Some(Token::If) => if_stmt(input),
        Some(Token::LBrace) => block(input).map(Statement::Block),
        Some(Token::Try) => try_stmt(input).map(Statement::Try),
        Some(Token::Throw) => throw_stmt(input).map(Statement::Throw),
        Some(Token::Import) => import_stmt(input).map(Statement::Import),
        Some(Token::Pub) => pub_stmt(input),
        Some(Token::Reserved) if is_removed_export(input.first()) => {
            let original = *input;
            *input = &input[1..];
            let span = consumed_span(original, input, 0);
            Err(ErrMode::Cut(PErr {
                span,
                message: "the `export` keyword was removed: use `pub fn`/`pub let`/`pub const`/`pub type`/`pub enum`/`pub trait`, or `pub use { a, b }` to re-export names"
                    .into(),
            }))
        }
        Some(Token::Reserved) if is_removed_default(input.first()) => {
            let original = *input;
            *input = &input[1..];
            let span = consumed_span(original, input, 0);
            Err(ErrMode::Cut(PErr {
                span,
                message: "the `default` keyword was removed: Xulo has no default exports — export with `pub <decl>` / `pub use { a, b }` and import by name"
                    .into(),
            }))
        }
        Some(Token::At) => decorator_stmt(input),
        Some(Token::Ident) if is_component(input) => {
            component_stmt(input).map(Statement::Component)
        }
        _ => expr_or_assign(input),
    }
}

/// True when an uppercase-leading identifier followed by `(` or `{` begins a
/// UI component statement (`VStack { ... }`, `Text("hi")`).
fn is_component(input: &In<'_>) -> bool {
    match (input.first(), input.get(1)) {
        (Some(t), Some(n)) => {
            t.kind == Token::Ident
                && t.text
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_uppercase())
                && matches!(n.kind, Token::LParen | Token::LBrace)
        }
        _ => false,
    }
}

fn fn_def(input: &mut In<'_>) -> Pr<FnDef> {
    fn_def_opt(input, false)
}

/// Shared `fn` definition parser; `allow_self` is set inside `impl` bodies so a
/// leading `self` receiver parameter is accepted.
fn fn_def_opt(input: &mut In<'_>, allow_self: bool) -> Pr<FnDef> {
    let original = *input;
    tk(input, Token::Fn)?;
    let name_original = *input;
    let name = ident_name(input)?;
    let name_span = consumed_span(name_original, input, 0);
    let (type_params, mut bounds) = opt_type_params(input)?;
    let params = params_list_opt(input, allow_self)?;
    let (return_type, is_async) = if opt_tk(input, Token::Colon) {
        if peek_is(input, Token::Async) {
            tk(input, Token::Async)?;
            let inner = if matches!(input.first().map(|t| t.kind), Some(Token::LBrace)) {
                xulo_core::ast::Type::Any
            } else {
                type_expr(input)?
            };
            (Some(xulo_core::ast::Type::Async(Box::new(inner))), true)
        } else {
            (Some(type_expr(input)?), false)
        }
    } else {
        (None, false)
    };
    // A trailing `where T: Trait` clause follows the return type.
    let where_bounds = where_clause(input)?;
    bounds.extend(where_bounds);
    let body = block(input)?;
    let span = consumed_span(original, input, 0);
    Ok(FnDef {
        name,
        name_span,
        params,
        return_type,
        type_params,
        bounds,
        is_async,
        body,
        span,
    })
}

pub(super) fn params_list(input: &mut In<'_>) -> Pr<Vec<Param>> {
    params_list_opt(input, false)
}

/// `params_list`, but when `allow_self` a leading `self` parameter is accepted
/// as the receiver of a trait `impl` method. `self` is only legal as the
/// *first* parameter; anywhere else it is a reserved keyword and rejected,
/// since dispatch binds the receiver positionally before the listed params.
fn params_list_opt(input: &mut In<'_>, allow_self: bool) -> Pr<Vec<Param>> {
    tk(input, Token::LParen)?;
    let mut params = Vec::new();
    if !matches!(input.first().map(|t| t.kind), Some(Token::RParen)) {
        loop {
            if !params.is_empty() && is_self_tk(input.first()) {
                let original = *input;
                let span = consumed_span(original, input, 0);
                return Err(ErrMode::Cut(PErr {
                    span,
                    message: "`self` must be the first parameter: it binds the receiver before the remaining arguments".into(),
                }));
            }
            params.push(param(input, allow_self)?);
            if !opt_tk(input, Token::Comma) {
                break;
            }
        }
    }
    tk(input, Token::RParen)?;
    Ok(params)
}

fn param(input: &mut In<'_>, allow_self: bool) -> Pr<Param> {
    let original = *input;
    let name = if allow_self && is_self_tk(input.first()) {
        *input = &input[1..];
        "self".to_string()
    } else {
        ident_name(input)?
    };
    let type_annotation = if opt_tk(input, Token::Colon) {
        Some(type_expr(input)?)
    } else {
        None
    };
    let default = if opt_tk(input, Token::Assign) {
        Some(expression(input)?)
    } else {
        None
    };
    let span = consumed_span(original, input, 0);
    Ok(Param {
        name,
        type_annotation,
        default,
        span,
    })
}

/// True when the token is the `self` receiver keyword. `self` stays a reserved
/// word; it is only accepted as a parameter name inside `impl` bodies.
fn is_self_tk(tok: Option<&LexedToken>) -> bool {
    matches!(tok, Some(t) if t.kind == Token::Reserved && t.text == "self")
}

/// The removed `export` keyword, now a reserved word. A statement starting with
/// it gets a targeted error pointing at the `pub`-based replacements.
fn is_removed_export(tok: Option<&LexedToken>) -> bool {
    matches!(tok, Some(t) if t.kind == Token::Reserved && t.text == "export")
}

/// The removed `default` keyword, now a reserved word (Xulo has no default
/// exports). A statement starting with it gets a targeted error.
fn is_removed_default(tok: Option<&LexedToken>) -> bool {
    matches!(tok, Some(t) if t.kind == Token::Reserved && t.text == "default")
}

fn let_binding(input: &mut In<'_>, is_const: bool) -> Pr<LetBinding> {
    if is_const {
        tk(input, Token::Const)?;
    } else {
        tk(input, Token::Let)?;
    }
    let is_mutable = if !is_const && peek_is(input, Token::Mut) {
        tk(input, Token::Mut)?;
        true
    } else {
        false
    };
    let_binding_body(input, is_const, is_mutable)
}

/// Parse a `let`/`const` binding after the keyword has been consumed.
fn let_binding_body(input: &mut In<'_>, is_const: bool, is_mutable: bool) -> Pr<LetBinding> {
    let name_original = *input;

    // Tuple destructuring: `let (a, b, c) = expr`
    if peek_is(input, Token::LParen) {
        let mut names = Vec::new();
        tk(input, Token::LParen)?;
        if !peek_is(input, Token::RParen) {
            loop {
                let n = ident_name(input)?;
                names.push(n);
                if !opt_tk(input, Token::Comma) || peek_is(input, Token::RParen) {
                    break;
                }
            }
        }
        tk(input, Token::RParen)?;
        let name_span = consumed_span(name_original, input, 0);
        let value = if opt_tk(input, Token::Assign) {
            Some(expression(input)?)
        } else {
            None
        };
        if is_const && value.is_none() {
            return Err(ErrMode::Cut(PErr::unexpected(input)));
        }
        return Ok(LetBinding {
            name: String::new(),
            name_span,
            type_annotation: None,
            value,
            is_const,
            is_mutable,
            memo: false,
            memo_deps: None,
            tuple_names: Some(names),
        });
    }

    let name = ident_name(input)?;
    let name_span = consumed_span(name_original, input, 0);
    let type_annotation = if opt_tk(input, Token::Colon) {
        Some(type_expr(input)?)
    } else {
        None
    };
    // `let x := expr` is sugar for `let mut x = expr`.
    let (value, mutability) = if opt_tk(input, Token::ColonAssign) {
        let v = Some(expression(input)?);
        (v, true)
    } else if opt_tk(input, Token::Assign) {
        let v = Some(expression(input)?);
        (v, is_mutable)
    } else {
        (None, is_mutable)
    };
    if is_const && value.is_none() {
        return Err(ErrMode::Cut(PErr::unexpected(input)));
    }
    Ok(LetBinding {
        name,
        name_span,
        type_annotation,
        value,
        is_const,
        is_mutable: mutability,
        memo: false,
        memo_deps: None,
        tuple_names: None,
    })
}

/// An expression statement, or an assignment when the expression is followed by
/// `=`. Only identifiers, member accesses, and indexes are valid targets.
fn expr_or_assign(input: &mut In<'_>) -> Pr<Statement> {
    let original = *input;
    let expr = expression(input)?;
    if matches!(input.first().map(|t| t.kind), Some(Token::Assign)) {
        let target = match expr {
            Expression::Identifier { name, .. } => AssignTarget::Name(name),
            Expression::Member(m) if !m.optional => {
                AssignTarget::Member(Box::new(m.object), m.property)
            }
            Expression::Index(i) => AssignTarget::Index(i.object, i.index),
            _ => {
                return Err(ErrMode::Cut(PErr::unexpected(input)));
            }
        };
        tk(input, Token::Assign)?;
        let value = expression(input)?;
        let span = consumed_span(original, input, 0);
        Ok(Statement::Assign(AssignStmt {
            target,
            value,
            span,
        }))
    } else {
        let span = consumed_span(original, input, 0);
        Ok(Statement::Expr(ExprStmt {
            expr,
            has_semicolon: false,
            span,
        }))
    }
}

fn type_alias(input: &mut In<'_>) -> Pr<TypeAlias> {
    tk(input, Token::Type)?;
    let name_original = *input;
    let name = ident_name(input)?;
    let name_span = consumed_span(name_original, input, 0);
    let (type_params, _) = opt_type_params(input)?;
    tk(input, Token::Assign)?;
    let type_ = type_expr(input)?;
    Ok(TypeAlias {
        name,
        name_span,
        type_params,
        type_,
    })
}

fn enum_def(input: &mut In<'_>) -> Pr<EnumDef> {
    tk(input, Token::Enum)?;
    let name_original = *input;
    let name = ident_name(input)?;
    let name_span = consumed_span(name_original, input, 0);
    let (type_params, _) = opt_type_params(input)?;
    tk(input, Token::LBrace)?;
    let mut variants = Vec::new();
    while !matches!(input.first().map(|t| t.kind), Some(Token::RBrace)) {
        if at_eof(input) {
            return Err(ErrMode::Backtrack(PErr::unexpected(input)));
        }
        let vname_original = *input;
        let vname = ident_name(input)?;
        let vname_span = consumed_span(vname_original, input, 0);
        let payload = if opt_tk(input, Token::LParen) {
            let mut params = Vec::new();
            loop {
                let (name, type_) = if input.first().map(|t| t.kind) == Some(Token::Ident)
                    && input.get(1).map(|t| t.kind) == Some(Token::Colon)
                {
                    let field = ident_name(input)?;
                    tk(input, Token::Colon)?;
                    let ty = type_expr(input)?;
                    (Some(field), ty)
                } else {
                    let ty = type_expr(input)?;
                    (None, ty)
                };
                params.push(EnumPayloadParam { name, type_ });
                if !opt_tk(input, Token::Comma)
                    || matches!(input.first().map(|t| t.kind), Some(Token::RParen))
                {
                    break;
                }
            }
            tk(input, Token::RParen)?;
            Some(params)
        } else {
            None
        };
        variants.push(EnumVariant {
            name: vname,
            name_span: vname_span,
            payload,
        });
        opt_tk(input, Token::Comma);
    }
    tk(input, Token::RBrace)?;
    Ok(EnumDef {
        name,
        name_span,
        type_params,
        variants,
    })
}

/// Optional `<T, U>` type parameters with optional inline bounds (`<T: Area>`).
/// Returns the parameters and any bounds written inline. Parsed, then erased at
/// codegen time.
fn opt_type_params(input: &mut In<'_>) -> Pr<(Vec<String>, Vec<FnBound>)> {
    if !peek_is(input, Token::Lt) {
        return Ok((Vec::new(), Vec::new()));
    }
    tk(input, Token::Lt)?;
    let mut params = Vec::new();
    let mut bounds = Vec::new();
    loop {
        let param = ident_name(input)?;
        if opt_tk(input, Token::Colon) {
            let traits = trait_refs(input)?;
            bounds.push(FnBound {
                param: param.clone(),
                traits,
            });
        }
        params.push(param);
        if !opt_tk(input, Token::Comma) {
            break;
        }
    }
    tk(input, Token::Gt)?;
    Ok((params, bounds))
}

/// `Trait & Trait` bound list, as identifiers (resolved by the semantic phase).
fn trait_refs(input: &mut In<'_>) -> Pr<Vec<String>> {
    let mut traits = vec![ident_name(input)?];
    while opt_tk(input, Token::Amp) {
        traits.push(ident_name(input)?);
    }
    Ok(traits)
}

/// Optional trailing `where T: Area, U: Comparable` clause on a function.
fn where_clause(input: &mut In<'_>) -> Pr<Vec<FnBound>> {
    let mut bounds = Vec::new();
    while opt_tk(input, Token::Where) {
        loop {
            let param = ident_name(input)?;
            tk(input, Token::Colon)?;
            let traits = trait_refs(input)?;
            bounds.push(FnBound {
                param: param.clone(),
                traits,
            });
            if !opt_tk(input, Token::Comma) {
                break;
            }
        }
    }
    Ok(bounds)
}

/// `trait Name { fn method(self, ...): ReturnType; ... }` — a set of method
/// signatures forming a named structural contract.
fn trait_def(input: &mut In<'_>) -> Pr<TraitDecl> {
    let original = *input;
    tk(input, Token::Trait)?;
    let name_original = *input;
    let name = ident_name(input)?;
    let name_span = consumed_span(name_original, input, 0);
    let (type_params, _) = opt_type_params(input)?;
    tk(input, Token::LBrace)?;
    let mut methods = Vec::new();
    while !matches!(input.first().map(|t| t.kind), Some(Token::RBrace)) {
        if at_eof(input) {
            return Err(ErrMode::Backtrack(PErr::unexpected(input)));
        }
        methods.push(trait_method(input)?);
    }
    tk(input, Token::RBrace)?;
    let span = consumed_span(original, input, 0);
    Ok(TraitDecl {
        name,
        name_span,
        type_params,
        methods,
        span,
    })
}

/// One method signature inside a `trait` block.
fn trait_method(input: &mut In<'_>) -> Pr<TraitMethod> {
    let original = *input;
    tk(input, Token::Fn)?;
    let name_original = *input;
    let name = ident_name(input)?;
    let name_span = consumed_span(name_original, input, 0);
    let params = params_list_opt(input, true)?;
    let has_self = params.first().is_some_and(|p| p.name == "self");
    let rest = params.into_iter().filter(|p| p.name != "self").collect();
    let (return_type, is_async) = if opt_tk(input, Token::Colon) {
        if peek_is(input, Token::Async) {
            tk(input, Token::Async)?;
            let inner = if matches!(input.first().map(|t| t.kind), Some(Token::LBrace)) {
                xulo_core::ast::Type::Any
            } else {
                type_expr(input)?
            };
            (Some(xulo_core::ast::Type::Async(Box::new(inner))), true)
        } else {
            (Some(type_expr(input)?), false)
        }
    } else {
        (None, false)
    };
    opt_tk(input, Token::Semicolon);
    let span = consumed_span(original, input, 0);
    Ok(TraitMethod {
        name,
        name_span,
        has_self,
        params: rest,
        return_type,
        is_async,
        span,
    })
}

/// `impl Area for Rectangle { fn area(self): number { ... } }` — bodies for the
/// named trait's methods on a concrete type.
fn impl_def(input: &mut In<'_>) -> Pr<ImplDecl> {
    let original = *input;
    tk(input, Token::Impl)?;
    let trait_name = ident_name(input)?;
    tk(input, Token::For)?;
    let type_name = ident_name(input)?;
    tk(input, Token::LBrace)?;
    let mut methods = Vec::new();
    while !matches!(input.first().map(|t| t.kind), Some(Token::RBrace)) {
        if at_eof(input) {
            return Err(ErrMode::Backtrack(PErr::unexpected(input)));
        }
        methods.push(fn_def_opt(input, true)?);
    }
    tk(input, Token::RBrace)?;
    let span = consumed_span(original, input, 0);
    Ok(ImplDecl {
        trait_name,
        type_name,
        methods,
        span,
    })
}

fn return_stmt(input: &mut In<'_>) -> Pr<ReturnStmt> {
    let original = *input;
    tk(input, Token::Return)?;
    // A bare `return` (no value) is allowed (docs EBNF §7).
    let value = if matches!(
        input.first().map(|t| t.kind),
        Some(Token::RBrace | Token::Semicolon | Token::EOF)
    ) {
        None
    } else {
        Some(expression(input)?)
    };
    let span = consumed_span(original, input, 0);
    Ok(ReturnStmt { value, span })
}

fn break_stmt(input: &mut In<'_>) -> Pr<Statement> {
    let original = *input;
    tk(input, Token::Break)?;
    let _span = consumed_span(original, input, 0);
    Ok(Statement::Break)
}

fn continue_stmt(input: &mut In<'_>) -> Pr<Statement> {
    let original = *input;
    tk(input, Token::Continue)?;
    let _span = consumed_span(original, input, 0);
    Ok(Statement::Continue)
}

fn for_stmt(input: &mut In<'_>) -> Pr<ForStmt> {
    tk(input, Token::For)?;
    let original = *input;
    let iter_var = ident_name(input)?;
    let iter_var_span = consumed_span(original, input, 0);
    tk(input, Token::In)?;
    let iterable = expression(input)?;
    let body = block(input)?;
    Ok(ForStmt {
        iter_var,
        iter_var_span,
        iterable,
        body,
    })
}

fn while_stmt(input: &mut In<'_>) -> Pr<WhileStmt> {
    tk(input, Token::While)?;
    let condition = expression(input)?;
    let body = block(input)?;
    Ok(WhileStmt { condition, body })
}

fn if_stmt(input: &mut In<'_>) -> Pr<Statement> {
    let e = if_expr(input)?;
    let span = e.span.clone();
    Ok(Statement::Expr(ExprStmt {
        expr: Expression::If(Box::new(e)),
        has_semicolon: false,
        span,
    }))
}

/// `{ statement; statement; ... }`.
pub fn block(input: &mut In<'_>) -> Pr<Block> {
    tk(input, Token::LBrace)?;
    let mut statements = Vec::new();
    while !matches!(input.first().map(|t| t.kind), Some(Token::RBrace)) {
        if at_eof(input) {
            return Err(ErrMode::Backtrack(PErr::unexpected(input)));
        }
        statements.push(super::terminated_statement(input)?);
    }
    tk(input, Token::RBrace)?;
    Ok(Block { statements })
}

fn try_stmt(input: &mut In<'_>) -> Pr<TryStmt> {
    tk(input, Token::Try)?;
    let try_block = block(input)?;
    tk(input, Token::Catch)?;
    tk(input, Token::LParen)?;
    let original = *input;
    let catch_var = ident_name(input)?;
    let catch_var_span = consumed_span(original, input, 0);
    tk(input, Token::RParen)?;
    let catch_block = block(input)?;
    Ok(TryStmt {
        try_block,
        catch_var,
        catch_var_span,
        catch_block,
    })
}

fn throw_stmt(input: &mut In<'_>) -> Pr<Expression> {
    tk(input, Token::Throw)?;
    expression(input)
}

fn import_stmt(input: &mut In<'_>) -> Pr<ImportStmt> {
    tk(input, Token::Import)?;
    let type_only = opt_tk(input, Token::Type);
    if matches!(input.first().map(|t| t.kind), Some(Token::String)) {
        let t = verified_tk(input, Token::String)?;
        return Ok(ImportStmt {
            source: super::expression::decode_string(&t.text),
            spec: ImportSpec::Bare,
            type_only,
        });
    }
    let spec = if opt_tk(input, Token::LBrace) {
        let mut names = Vec::new();
        if !matches!(input.first().map(|t| t.kind), Some(Token::RBrace)) {
            loop {
                let name = ident_name(input)?;
                let alias = if opt_tk(input, Token::As) {
                    Some(ident_name(input)?)
                } else {
                    None
                };
                names.push((name, alias));
                if !opt_tk(input, Token::Comma) {
                    break;
                }
            }
        }
        tk(input, Token::RBrace)?;
        ImportSpec::Named(names)
    } else if opt_tk(input, Token::Star) {
        tk(input, Token::As)?;
        let ns = ident_name(input)?;
        ImportSpec::Namespace(ns)
    } else {
        let name = ident_name(input)?;
        tk(input, Token::As)?;
        let alias = ident_name(input)?;
        ImportSpec::Named(vec![(name, Some(alias))])
    };
    tk(input, Token::From)?;
    let t = verified_tk(input, Token::String)?;
    Ok(ImportStmt {
        source: super::expression::decode_string(&t.text),
        spec,
        type_only,
    })
}

/// `pub fn/let/const/type/enum/trait` — public-visibility modifier that lowers
/// to the `Statement::Export` mechanism, so the declaration is visible to other
/// modules. Also `pub use { a, b }` (re-export already-declared names).
fn pub_stmt(input: &mut In<'_>) -> Pr<Statement> {
    tk(input, Token::Pub)?;
    use xulo_core::ast::ExportItem;
    let item = if matches!(input.first().map(|t| t.kind), Some(Token::Use)) {
        tk(input, Token::Use)?;
        let mut names = Vec::new();
        tk(input, Token::LBrace)?;
        if !matches!(input.first().map(|t| t.kind), Some(Token::RBrace)) {
            loop {
                names.push(ident_name(input)?);
                if !opt_tk(input, Token::Comma) {
                    break;
                }
            }
        }
        tk(input, Token::RBrace)?;
        ExportItem::Names(names)
    } else {
        export_decl(input)?
    };
    Ok(Statement::Export(ExportStmt { item }))
}

fn export_decl(input: &mut In<'_>) -> Pr<ExportItem> {
    use xulo_core::ast::ExportItem;
    match input.first().map(|t| t.kind) {
        Some(Token::Fn) => fn_def(input).map(ExportItem::Fn),
        Some(Token::Let) => let_binding(input, false).map(ExportItem::Let),
        Some(Token::Const) => let_binding(input, true).map(ExportItem::Let),
        Some(Token::Type) => type_alias(input).map(ExportItem::Type),
        Some(Token::Enum) => enum_def(input).map(ExportItem::Enum),
        Some(Token::Trait) => trait_def(input).map(ExportItem::Trait),
        _ => Err(ErrMode::Cut(PErr::unexpected(input))),
    }
}

/// `@State` / `@Store` / `@Effect` / `@Environment` / `@Memo` declarations.
fn decorator_stmt(input: &mut In<'_>) -> Pr<Statement> {
    tk(input, Token::At)?;
    let kind = ident_name(input)?;
    match kind.as_str() {
        "State" => {
            let is_const = if opt_tk(input, Token::Const) {
                true
            } else {
                tk(input, Token::Let)?;
                false
            };
            let binding = let_binding_body(input, is_const, false)?;
            Ok(Statement::State(StateStmt { binding }))
        }
        "Store" => {
            opt_tk(input, Token::Const);
            let pattern = binding_pattern(input)?;
            tk(input, Token::Assign)?;
            let value = expression(input)?;
            Ok(Statement::Store(StoreStmt { pattern, value }))
        }
        "Effect" => {
            // Leading dependency array: `@Effect([count]) fn() { ... }` (canonical).
            let mut deps = opt_leading_deps(input)?;
            let closure = fn_expr(input)?;
            let Expression::FnExpr(closure) = closure else {
                return Err(ErrMode::Cut(PErr::unexpected(input)));
            };
            // Legacy trailing dependency array: `@Effect fn() { ... }, [count]`.
            if opt_tk(input, Token::Comma) {
                if deps.is_some() {
                    return Err(ErrMode::Cut(PErr::unexpected(input)));
                }
                tk(input, Token::LBracket)?;
                let mut trailing = Vec::new();
                if !peek_is(input, Token::RBracket) {
                    loop {
                        trailing.push(expression(input)?);
                        if !opt_tk(input, Token::Comma) {
                            break;
                        }
                    }
                }
                tk(input, Token::RBracket)?;
                deps = Some(trailing);
            }
            Ok(Statement::Effect(EffectStmt {
                closure: *closure,
                deps,
            }))
        }
        "Environment" => {
            tk(input, Token::Let)?;
            let name_original = *input;
            let name = ident_name(input)?;
            let name_span = consumed_span(name_original, input, 0);
            tk(input, Token::Colon)?;
            let type_ = type_expr(input)?;
            Ok(Statement::Environment(EnvStmt {
                name,
                name_span,
                type_,
            }))
        }
        "Memo" => {
            // `@Memo([deps]) let x = expr` (empty or bare `@Memo` = cache forever).
            let deps = opt_leading_deps(input)?.unwrap_or_default();
            let is_const = if opt_tk(input, Token::Const) {
                true
            } else {
                tk(input, Token::Let)?;
                false
            };
            let mut binding = let_binding_body(input, is_const, false)?;
            binding.memo = true;
            binding.memo_deps = Some(deps);
            Ok(Statement::Let(binding))
        }
        _ => Err(ErrMode::Cut(PErr::unexpected(input))),
    }
}

/// An optional leading dependency array: `([a, b])` right after the decorator
/// keyword. Returns `None` when the next token is not `(`.
fn opt_leading_deps(input: &mut In<'_>) -> Pr<Option<Vec<Expression>>> {
    if !peek_is(input, Token::LParen) {
        return Ok(None);
    }
    tk(input, Token::LParen)?;
    tk(input, Token::LBracket)?;
    let mut deps = Vec::new();
    if !peek_is(input, Token::RBracket) {
        loop {
            deps.push(expression(input)?);
            if !opt_tk(input, Token::Comma) {
                break;
            }
        }
    }
    tk(input, Token::RBracket)?;
    tk(input, Token::RParen)?;
    Ok(Some(deps))
}

/// A `@Store` binding pattern: `name` or `{ a, b: c }`.
fn binding_pattern(input: &mut In<'_>) -> Pr<BindingPattern> {
    if opt_tk(input, Token::LBrace) {
        let mut fields = Vec::new();
        if !peek_is(input, Token::RBrace) {
            loop {
                let name = ident_name(input)?;
                let alias = if opt_tk(input, Token::Colon) {
                    Some(ident_name(input)?)
                } else {
                    None
                };
                fields.push((name, alias));
                if !opt_tk(input, Token::Comma) {
                    break;
                }
            }
        }
        tk(input, Token::RBrace)?;
        Ok(BindingPattern::Destructure(fields))
    } else {
        ident_name(input).map(BindingPattern::Ident)
    }
}

/// A UI component statement: `ComponentName(args)? { children }?`.
fn component_stmt(input: &mut In<'_>) -> Pr<ComponentStmt> {
    let (name, name_span) = ident_name_spanned(input)?;
    let args = if peek_is(input, Token::LParen) {
        call_args(input)?
    } else {
        Vec::new()
    };
    let children = if opt_tk(input, Token::LBrace) {
        let elements = ui_elements(input)?;
        tk(input, Token::RBrace)?;
        elements
    } else {
        Vec::new()
    };
    Ok(ComponentStmt {
        name,
        name_span,
        args,
        children,
    })
}

/// Zero or more UI elements, terminated by a closing `}`.
///
/// Every UI nesting path (`component_stmt` children, groups, `if`/`else`/`for`
/// bodies, `else if` chains) recurses through here, so the depth guard placed
/// here protects the whole UI subtree the same way `statement`/`expression`
/// guard theirs — without it, hostile `VStack { VStack { ... } }` nesting
/// bypasses `MAX_NEST_DEPTH` and overflows the stack.
fn ui_elements(input: &mut In<'_>) -> Pr<Vec<UiElement>> {
    let _guard = enter_nest(input)?;
    let mut elements = Vec::new();
    while !peek_is(input, Token::RBrace) {
        if at_eof(input) {
            return Err(ErrMode::Backtrack(PErr::unexpected(input)));
        }
        elements.push(ui_element(input)?);
    }
    Ok(elements)
}

/// A single UI element: component, naked string, expression, `if`, `for`, or
/// grouping.
fn ui_element(input: &mut In<'_>) -> Pr<UiElement> {
    match input.first().map(|t| t.kind) {
        Some(Token::LBrace) => {
            tk(input, Token::LBrace)?;
            let group = ui_elements(input)?;
            tk(input, Token::RBrace)?;
            Ok(UiElement::Group(group))
        }
        Some(Token::If) => ui_if(input),
        Some(Token::For) => ui_for(input),
        Some(Token::String) => {
            let t = verified_tk(input, Token::String)?;
            Ok(UiElement::Text(decode_string(&t.text)))
        }
        Some(Token::Ident) if is_component(input) => {
            component_stmt(input).map(UiElement::Component)
        }
        _ => expression(input)
            .map(UiElement::Expr)
            .map_err(|_| ErrMode::Backtrack(PErr::unexpected(input))),
    }
}

/// A UI `if` / `else` (conditional rendering).
fn ui_if(input: &mut In<'_>) -> Pr<UiElement> {
    tk(input, Token::If)?;
    let condition = expression(input)?;
    tk(input, Token::LBrace)?;
    let then_branch = ui_elements(input)?;
    tk(input, Token::RBrace)?;
    let else_branch = if opt_tk(input, Token::Else) {
        if peek_is(input, Token::If) {
            Some(vec![ui_if(input)?])
        } else {
            tk(input, Token::LBrace)?;
            let els = ui_elements(input)?;
            tk(input, Token::RBrace)?;
            Some(els)
        }
    } else {
        None
    };
    Ok(UiElement::If {
        condition,
        then_branch,
        else_branch,
    })
}

/// A UI `for` (list rendering).
fn ui_for(input: &mut In<'_>) -> Pr<UiElement> {
    tk(input, Token::For)?;
    let original = *input;
    let iter_var = ident_name(input)?;
    let iter_var_span = consumed_span(original, input, 0);
    tk(input, Token::In)?;
    let iterable = expression(input)?;
    tk(input, Token::LBrace)?;
    let body = ui_elements(input)?;
    tk(input, Token::RBrace)?;
    Ok(UiElement::For {
        iter_var,
        iter_var_span,
        iterable,
        body,
    })
}
