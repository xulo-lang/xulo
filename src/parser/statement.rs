use crate::ast::{
    AssignStmt, BindingPattern, Block, ComponentStmt, EffectStmt, EnumDef, EnumVariant, EnvStmt,
    ExportItem, ExportStmt, Expression, FnDef, ForStmt, ImportSpec, ImportStmt, LetBinding, Param,
    ReturnStmt, StateStmt, Statement, StoreStmt, TryStmt, TypeAlias, UiElement, WhileStmt,
};
use crate::lexer::token::Token;

use super::expression::{call_args, decode_string, expression, fn_expr, if_expr};
use super::types::type_expr;
use super::{at_eof, ident_name, opt_tk, peek_is, tk, verified_tk, In, PErr, Pr};
use winnow::error::ErrMode;

/// A statement: `fn`/`let`/`const`/`type`/`enum` definitions, `return`, `for`,
/// `while`, assignment, `if`, block, or an expression statement — dispatched on
/// the leading token.
pub fn statement(input: &mut In<'_>) -> Pr<Statement> {
    match input.first().map(|t| t.kind) {
        Some(Token::Fn) => {
            // `fn name(...)` is a definition; `fn(...)` in statement position is
            // an anonymous function expression (e.g. a trailing implicit return).
            if matches!(input.get(1).map(|t| t.kind), Some(Token::LParen)) {
                expression(input).map(Statement::Expr)
            } else {
                fn_def(input).map(Statement::Fn)
            }
        }
        Some(Token::Let) => let_binding(input, false).map(Statement::Let),
        Some(Token::Const) => let_binding(input, true).map(Statement::Let),
        Some(Token::Type) => type_alias(input).map(Statement::TypeAlias),
        Some(Token::Enum) => enum_def(input).map(Statement::Enum),
        Some(Token::Return) => return_stmt(input).map(Statement::Return),
        Some(Token::For) => for_stmt(input).map(Statement::For),
        Some(Token::While) => while_stmt(input).map(Statement::While),
        Some(Token::If) => if_stmt(input),
        Some(Token::LBrace) => block(input).map(Statement::Block),
        Some(Token::Try) => try_stmt(input).map(Statement::Try),
        Some(Token::Throw) => throw_stmt(input).map(Statement::Throw),
        Some(Token::Import) => import_stmt(input).map(Statement::Import),
        Some(Token::Export) => export_stmt(input).map(Statement::Export),
        Some(Token::At) => decorator_stmt(input),
        Some(Token::Ident) if is_component(input) => {
            component_stmt(input).map(Statement::Component)
        }
        Some(Token::Ident) if is_assignment(input) => assign_stmt(input).map(Statement::Assign),
        _ => expression(input).map(Statement::Expr),
    }
}

/// True when the statement at this position is `ident = expr`.
fn is_assignment(input: &mut In<'_>) -> bool {
    matches!(input.get(1).map(|t| t.kind), Some(Token::Assign))
}

/// True when an uppercase-leading identifier followed by `(` or `{` begins a
/// UI component statement (`VStack { ... }`, `Text("hi")`).
fn is_component(input: &In<'_>) -> bool {
    match (input.first(), input.get(1)) {
        (Some(t), Some(n)) => {
            t.kind == Token::Ident
                && t.text.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                && matches!(n.kind, Token::LParen | Token::LBrace)
        }
        _ => false,
    }
}

fn fn_def(input: &mut In<'_>) -> Pr<FnDef> {
    tk(input, Token::Fn)?;
    let name = ident_name(input)?;
    let type_params = opt_type_params(input)?;
    let params = params_list(input)?;
    let (return_type, is_async) = if opt_tk(input, Token::Colon) {
        if peek_is(input, Token::Async) {
            tk(input, Token::Async)?;
            let inner = if matches!(input.first().map(|t| t.kind), Some(Token::LBrace)) {
                crate::ast::Type::Any
            } else {
                type_expr(input)?
            };
            (Some(crate::ast::Type::Async(Box::new(inner))), true)
        } else {
            (Some(type_expr(input)?), false)
        }
    } else {
        (None, false)
    };
    let body = block(input)?;
    Ok(FnDef {
        name,
        type_params,
        params,
        return_type,
        body,
        is_async,
    })
}

pub(super) fn params_list(input: &mut In<'_>) -> Pr<Vec<Param>> {
    tk(input, Token::LParen)?;
    let mut params = Vec::new();
    if !matches!(input.first().map(|t| t.kind), Some(Token::RParen)) {
        loop {
            params.push(param(input)?);
            if !opt_tk(input, Token::Comma) {
                break;
            }
        }
    }
    tk(input, Token::RParen)?;
    Ok(params)
}

fn param(input: &mut In<'_>) -> Pr<Param> {
    let name = ident_name(input)?;
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
    Ok(Param {
        name,
        type_annotation,
        default,
    })
}

fn let_binding(input: &mut In<'_>, is_const: bool) -> Pr<LetBinding> {
    if is_const {
        tk(input, Token::Const)?;
    } else {
        tk(input, Token::Let)?;
    }
    let_binding_body(input, is_const)
}

/// Parse a `let`/`const` binding after the keyword has been consumed.
fn let_binding_body(input: &mut In<'_>, is_const: bool) -> Pr<LetBinding> {
    let name = ident_name(input)?;
    let type_annotation = if opt_tk(input, Token::Colon) {
        Some(type_expr(input)?)
    } else {
        None
    };
    let value = if opt_tk(input, Token::Assign) {
        Some(expression(input)?)
    } else {
        None
    };
    if is_const && value.is_none() {
        return Err(ErrMode::Cut(PErr::unexpected(input)));
    }
    Ok(LetBinding {
        name,
        type_annotation,
        value,
        is_const,
    })
}

fn assign_stmt(input: &mut In<'_>) -> Pr<AssignStmt> {
    let name = ident_name(input)?;
    tk(input, Token::Assign)?;
    let value = expression(input)?;
    Ok(AssignStmt { name, value })
}

fn type_alias(input: &mut In<'_>) -> Pr<TypeAlias> {
    tk(input, Token::Type)?;
    let name = ident_name(input)?;
    let type_params = opt_type_params(input)?;
    tk(input, Token::Assign)?;
    let type_ = type_expr(input)?;
    Ok(TypeAlias {
        name,
        type_params,
        type_,
    })
}

fn enum_def(input: &mut In<'_>) -> Pr<EnumDef> {
    tk(input, Token::Enum)?;
    let name = ident_name(input)?;
    let type_params = opt_type_params(input)?;
    tk(input, Token::LBrace)?;
    let mut variants = Vec::new();
    while !matches!(input.first().map(|t| t.kind), Some(Token::RBrace)) {
        if at_eof(input) {
            return Err(ErrMode::Backtrack(PErr::unexpected(input)));
        }
        let vname = ident_name(input)?;
        let (payload, payload_name) = if opt_tk(input, Token::LParen) {
            let (ty, name) = if input.first().map(|t| t.kind) == Some(Token::Ident)
                && input.get(1).map(|t| t.kind) == Some(Token::Colon)
            {
                let field = ident_name(input)?;
                tk(input, Token::Colon)?;
                let ty = type_expr(input)?;
                (ty, Some(field))
            } else {
                let ty = type_expr(input)?;
                (ty, None)
            };
            tk(input, Token::RParen)?;
            (Some(ty), name)
        } else {
            (None, None)
        };
        variants.push(EnumVariant {
            name: vname,
            payload,
            payload_name,
        });
        opt_tk(input, Token::Comma);
    }
    tk(input, Token::RBrace)?;
    Ok(EnumDef {
        name,
        type_params,
        variants,
    })
}

/// Optional `<T, U>` type parameters (parsed, then erased at codegen time).
fn opt_type_params(input: &mut In<'_>) -> Pr<Vec<String>> {
    if !peek_is(input, Token::Lt) {
        return Ok(Vec::new());
    }
    tk(input, Token::Lt)?;
    let mut params = Vec::new();
    loop {
        params.push(ident_name(input)?);
        if !opt_tk(input, Token::Comma) {
            break;
        }
    }
    tk(input, Token::Gt)?;
    Ok(params)
}

fn return_stmt(input: &mut In<'_>) -> Pr<ReturnStmt> {
    tk(input, Token::Return)?;
    let value = expression(input)?;
    Ok(ReturnStmt { value })
}

fn for_stmt(input: &mut In<'_>) -> Pr<ForStmt> {
    tk(input, Token::For)?;
    let iter_var = ident_name(input)?;
    tk(input, Token::In)?;
    let iterable = expression(input)?;
    let body = block(input)?;
    Ok(ForStmt {
        iter_var,
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
    if_expr(input).map(|e| Statement::Expr(Expression::If(Box::new(e))))
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
    let catch_var = ident_name(input)?;
    tk(input, Token::RParen)?;
    let catch_block = block(input)?;
    Ok(TryStmt {
        try_block,
        catch_var,
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
        if opt_tk(input, Token::As) {
            let alias = ident_name(input)?;
            ImportSpec::Named(vec![(name, Some(alias))])
        } else {
            ImportSpec::Default(name)
        }
    };
    tk(input, Token::From)?;
    let t = verified_tk(input, Token::String)?;
    Ok(ImportStmt {
        source: super::expression::decode_string(&t.text),
        spec,
        type_only,
    })
}

fn export_stmt(input: &mut In<'_>) -> Pr<ExportStmt> {
    tk(input, Token::Export)?;
    use crate::ast::ExportItem;
    let item = if opt_tk(input, Token::LBrace) {
        let mut names = Vec::new();
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
    } else if opt_tk(input, Token::Default) {
        ExportItem::Default(Box::new(export_decl(input)?))
    } else {
        export_decl(input)?
    };
    Ok(ExportStmt { item })
}

fn export_decl(input: &mut In<'_>) -> Pr<ExportItem> {
    use crate::ast::ExportItem;
    match input.first().map(|t| t.kind) {
        Some(Token::Fn) => fn_def(input).map(ExportItem::Fn),
        Some(Token::Let) => let_binding(input, false).map(ExportItem::Let),
        Some(Token::Const) => let_binding(input, true).map(ExportItem::Let),
        Some(Token::Type) => type_alias(input).map(ExportItem::Type),
        Some(Token::Enum) => enum_def(input).map(ExportItem::Enum),
        _ => Err(ErrMode::Cut(PErr::unexpected(input))),
    }
}

/// `@State` / `@Store` / `@Effect` / `@Environment` declarations.
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
            let binding = let_binding_body(input, is_const)?;
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
            let closure = fn_expr(input)?;
            let Expression::FnExpr(closure) = closure else {
                return Err(ErrMode::Cut(PErr::unexpected(input)));
            };
            let deps = if opt_tk(input, Token::Comma) {
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
                Some(deps)
            } else {
                None
            };
            Ok(Statement::Effect(EffectStmt {
                closure: *closure,
                deps,
            }))
        }
        "Environment" => {
            tk(input, Token::Let)?;
            let name = ident_name(input)?;
            tk(input, Token::Colon)?;
            let type_ = type_expr(input)?;
            Ok(Statement::Environment(EnvStmt { name, type_ }))
        }
        _ => Err(ErrMode::Cut(PErr::unexpected(input))),
    }
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
    let name = ident_name(input)?;
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
        args,
        children,
    })
}

/// Zero or more UI elements, terminated by a closing `}`.
fn ui_elements(input: &mut In<'_>) -> Pr<Vec<UiElement>> {
    let mut elements = Vec::new();
    while !peek_is(input, Token::RBrace) {
        if at_eof(input) {
            return Err(ErrMode::Backtrack(PErr::unexpected(input)));
        }
        elements.push(ui_element(input)?);
    }
    Ok(elements)
}

/// A single UI element: component, naked string, `if`, `for`, or grouping.
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
        Some(Token::Ident) if is_component(input) => component_stmt(input).map(UiElement::Component),
        _ => Err(ErrMode::Backtrack(PErr::unexpected(input))),
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
    let iter_var = ident_name(input)?;
    tk(input, Token::In)?;
    let iterable = expression(input)?;
    tk(input, Token::LBrace)?;
    let body = ui_elements(input)?;
    tk(input, Token::RBrace)?;
    Ok(UiElement::For {
        iter_var,
        iterable,
        body,
    })
}