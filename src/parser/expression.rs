use crate::ast::{
    BinaryOp, BinaryOperator, Block, Call, CallArg, CallValue, EnumRef, Expression, FnExpr, IfExpr,
    IndexExpr, Literal, MatchArm, MatchExpr, MatchPattern, MemberAccess, NullishExpr, ObjectField,
    RangeExpr, Statement, TernaryExpr, UnaryOp, UnaryOperator,
};
use crate::lexer::token::Token;

use super::statement::{block, params_list};
use super::types::type_expr;
use super::{ident_name, name, opt_tk, peek_is, tk, verified_tk, In, PErr, Pr};
use winnow::error::ErrMode;

const ADD_OPS: &[(Token, BinaryOperator)] = &[
    (Token::Plus, BinaryOperator::Add),
    (Token::Minus, BinaryOperator::Sub),
];

const MUL_OPS: &[(Token, BinaryOperator)] = &[
    (Token::Star, BinaryOperator::Mul),
    (Token::Slash, BinaryOperator::Div),
];

const CMP_OPS: &[(Token, BinaryOperator)] = &[
    (Token::Eq, BinaryOperator::Eq),
    (Token::Neq, BinaryOperator::Neq),
    (Token::Lt, BinaryOperator::Lt),
    (Token::Gt, BinaryOperator::Gt),
    (Token::Lte, BinaryOperator::Lte),
    (Token::Gte, BinaryOperator::Gte),
];

/// Full expression grammar, low to high precedence:
/// `ternary < or < and < nullish < comparison < additive < multiplicative <
/// unary < postfix < primary`.
pub fn expression(input: &mut In<'_>) -> Pr<Expression> {
    ternary(input)
}

/// Consume one binary operator from `ops` if present.
fn take_op(input: &mut In<'_>, ops: &[(Token, BinaryOperator)]) -> Pr<BinaryOperator> {
    let Some(t) = input.first() else {
        return Err(backtrack(input));
    };
    for (tok, op) in ops {
        if t.kind == *tok {
            *input = &input[1..];
            return Ok(*op);
        }
    }
    Err(backtrack(input))
}

fn backtrack(input: &In<'_>) -> winnow::error::ErrMode<PErr> {
    winnow::error::ErrMode::Backtrack(PErr::unexpected(input))
}

fn bin(lhs: Expression, operator: BinaryOperator, rhs: Expression) -> Expression {
    Expression::BinaryOp(Box::new(BinaryOp {
        left: lhs,
        operator,
        right: rhs,
    }))
}

fn ternary(input: &mut In<'_>) -> Pr<Expression> {
    let condition = logical_or(input)?;
    if opt_tk(input, Token::Question) {
        let then_value = expression(input)?;
        tk(input, Token::Colon)?;
        let else_value = expression(input)?;
        Ok(Expression::Ternary(Box::new(TernaryExpr {
            condition,
            then_value,
            else_value,
        })))
    } else {
        Ok(condition)
    }
}

fn logical_or(input: &mut In<'_>) -> Pr<Expression> {
    let mut lhs = logical_and(input)?;
    while opt_tk(input, Token::Or) {
        lhs = bin(lhs, BinaryOperator::Or, logical_and(input)?);
    }
    Ok(lhs)
}

fn logical_and(input: &mut In<'_>) -> Pr<Expression> {
    let mut lhs = nullish(input)?;
    while opt_tk(input, Token::And) {
        lhs = bin(lhs, BinaryOperator::And, nullish(input)?);
    }
    Ok(lhs)
}

fn nullish(input: &mut In<'_>) -> Pr<Expression> {
    let mut lhs = comparison(input)?;
    while opt_tk(input, Token::Nullish) {
        let rhs = comparison(input)?;
        lhs = Expression::Nullish(Box::new(NullishExpr {
            left: Box::new(lhs),
            right: Box::new(rhs),
        }));
    }
    Ok(lhs)
}

fn comparison(input: &mut In<'_>) -> Pr<Expression> {
    let mut lhs = additive(input)?;
    while let Ok(op) = take_op(input, CMP_OPS) {
        lhs = bin(lhs, op, additive(input)?);
    }
    if opt_tk(input, Token::RangeOp) {
        let end = additive(input)?;
        return Ok(range(lhs, end));
    }
    Ok(lhs)
}

fn range(start: Expression, end: Expression) -> Expression {
    Expression::Range(Box::new(RangeExpr {
        start: Box::new(start),
        end: Box::new(end),
    }))
}

fn additive(input: &mut In<'_>) -> Pr<Expression> {
    let mut lhs = multiplicative(input)?;
    while let Ok(op) = take_op(input, ADD_OPS) {
        lhs = bin(lhs, op, multiplicative(input)?);
    }
    Ok(lhs)
}

fn multiplicative(input: &mut In<'_>) -> Pr<Expression> {
    let mut lhs = unary(input)?;
    while let Ok(op) = take_op(input, MUL_OPS) {
        lhs = bin(lhs, op, unary(input)?);
    }
    Ok(lhs)
}

fn unary(input: &mut In<'_>) -> Pr<Expression> {
    let op = if peek_kind(input, Token::Minus) {
        Some(UnaryOperator::Neg)
    } else if peek_kind(input, Token::Bang) {
        Some(UnaryOperator::Not)
    } else {
        None
    };
    if let Some(operator) = op {
        let tok = input.first().unwrap();
        *input = &input[1..];
        let _ = tok;
        let operand = unary(input)?;
        Ok(Expression::Unary(Box::new(UnaryOp { operator, operand })))
    } else if peek_kind(input, Token::Await) {
        *input = &input[1..];
        let operand = unary(input)?;
        Ok(Expression::Await(Box::new(operand)))
    } else {
        postfix(input)
    }
}

fn peek_kind(input: &In<'_>, kind: Token) -> bool {
    matches!(input.first().map(|t| t.kind), Some(k) if k == kind)
}

fn postfix(input: &mut In<'_>) -> Pr<Expression> {
    let mut expr = primary(input)?;
    loop {
        if opt_tk(input, Token::Dot) {
            let property = name(input)?;
            expr = Expression::Member(Box::new(MemberAccess {
                object: expr,
                property,
                optional: false,
            }));
        } else if opt_tk(input, Token::QuestionDot) {
            let property = name(input)?;
            expr = Expression::Member(Box::new(MemberAccess {
                object: expr,
                property,
                optional: true,
            }));
        } else if opt_tk(input, Token::LBracket) {
            let index = expression(input)?;
            tk(input, Token::RBracket)?;
            expr = Expression::Index(Box::new(IndexExpr {
                object: Box::new(expr),
                index: Box::new(index),
            }));
        } else if matches!(input.first().map(|t| t.kind), Some(Token::LParen)) {
            let arguments = call_args(input)?;
            match expr {
                Expression::Identifier(name) => {
                    expr = Expression::Call(Call {
                        callee: name,
                        object: None,
                        method: None,
                        arguments,
                    });
                }
                Expression::EnumRef(r) => {
                    expr = Expression::Call(Call {
                        callee: format!("{}::{}", r.enum_name, r.variant),
                        object: None,
                        method: None,
                        arguments,
                    });
                }
                Expression::Member(m) => {
                    let object = m.object;
                    let property = m.property.clone();
                    let _ = m.optional;
                    expr = Expression::Call(Call {
                        callee: property.clone(),
                        object: Some(Box::new(object)),
                        method: Some(property),
                        arguments,
                    });
                }
                other => {
                    // Calling a function value held in an arbitrary
                    // expression: `xs[0](10)`, `getFn()(x)`, `(fn() {...})(5)`.
                    expr = Expression::CallValue(Box::new(CallValue {
                        callee: Box::new(other),
                        arguments,
                    }));
                }
            }
        } else {
            break;
        }
    }
    Ok(expr)
}

fn primary(input: &mut In<'_>) -> Pr<Expression> {
    match input.first().map(|t| t.kind) {
        Some(Token::If) => if_expr(input).map(|e| Expression::If(Box::new(e))),
        Some(Token::Match) => match_expr(input).map(|e| Expression::Match(Box::new(e))),
        Some(Token::Fn) => fn_expr(input),
        Some(Token::Ident) | Some(Token::Print) => ident_or_enum(input),
        Some(Token::LBracket) => list_literal(input),
        Some(Token::LBrace) => object_literal(input),
        Some(Token::String) | Some(Token::Number) | Some(Token::Boolean) | Some(Token::Null) => {
            literal_value(input)
        }
        Some(Token::LParen) => paren_expr(input),
        _ => Err(backtrack(input)),
    }
}

/// An anonymous function literal `fn(a: number): number { ... }` (an async
/// variant is written `fn(): async { ... }`).
fn fn_expr(input: &mut In<'_>) -> Pr<Expression> {
    tk(input, Token::Fn)?;
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
    Ok(Expression::FnExpr(Box::new(FnExpr {
        params,
        return_type,
        body,
        is_async,
    })))
}

fn paren_expr(input: &mut In<'_>) -> Pr<Expression> {
    tk(input, Token::LParen)?;
    let e = expression(input)?;
    tk(input, Token::RParen)?;
    Ok(e)
}

fn ident_or_enum(input: &mut In<'_>) -> Pr<Expression> {
    let first = name(input)?;
    if opt_tk(input, Token::DoubleColon) {
        let variant = ident_name(input)?;
        Ok(Expression::EnumRef(EnumRef {
            enum_name: first,
            variant,
        }))
    } else {
        Ok(Expression::Identifier(first))
    }
}

fn call_args(input: &mut In<'_>) -> Pr<Vec<CallArg>> {
    tk(input, Token::LParen)?;
    let mut args = Vec::new();
    if !matches!(input.first().map(|t| t.kind), Some(Token::RParen)) {
        loop {
            args.push(call_arg(input)?);
            if !opt_tk(input, Token::Comma) {
                break;
            }
        }
    }
    tk(input, Token::RParen)?;
    Ok(args)
}

/// A call argument: `[label ":"] expr`.
fn call_arg(input: &mut In<'_>) -> Pr<CallArg> {
    let labeled = matches!(input.first().map(|t| t.kind), Some(Token::Ident))
        && matches!(input.get(1).map(|t| t.kind), Some(Token::Colon));
    if labeled {
        let label = ident_name(input)?;
        tk(input, Token::Colon)?;
        let value = expression(input)?;
        Ok(CallArg {
            name: Some(label),
            value,
        })
    } else {
        let value = expression(input)?;
        Ok(CallArg { name: None, value })
    }
}

/// `[item, ...expr, item]` — `...expr` spreads a list into the literal.
fn list_literal(input: &mut In<'_>) -> Pr<Expression> {
    tk(input, Token::LBracket)?;
    let mut items = Vec::new();
    if !matches!(input.first().map(|t| t.kind), Some(Token::RBracket)) {
        loop {
            if opt_tk(input, Token::Ellipsis) {
                let value = expression(input)?;
                items.push(Expression::Spread(Box::new(value)));
            } else {
                items.push(expression(input)?);
            }
            if !opt_tk(input, Token::Comma) {
                break;
            }
        }
    }
    tk(input, Token::RBracket)?;
    Ok(Expression::Literal(Literal::List(items)))
}

/// `{ key: value, ...expr, ... }` — a block `{}` never appears inside an
/// expression, so a `{` here unambiguously begins an object literal.
fn object_literal(input: &mut In<'_>) -> Pr<Expression> {
    tk(input, Token::LBrace)?;
    let mut fields = Vec::new();
    if !matches!(input.first().map(|t| t.kind), Some(Token::RBrace)) {
        loop {
            if opt_tk(input, Token::Ellipsis) {
                let value = expression(input)?;
                fields.push(ObjectField::Spread { value });
            } else {
                let key = object_key(input)?;
                tk(input, Token::Colon)?;
                let value = expression(input)?;
                fields.push(ObjectField::Field { name: key, value });
            }
            if !opt_tk(input, Token::Comma) {
                break;
            }
        }
    }
    tk(input, Token::RBrace)?;
    Ok(Expression::Literal(Literal::Object(fields)))
}

fn object_key(input: &mut In<'_>) -> Pr<String> {
    match input.first().map(|t| t.kind) {
        Some(Token::Ident) => ident_name(input),
        Some(Token::String) => string_value(input),
        _ => Err(backtrack(input)),
    }
}

fn literal_value(input: &mut In<'_>) -> Pr<Expression> {
    let t = verified_tk(input, Token::String)
        .or_else(|_| verified_tk(input, Token::Number))
        .or_else(|_| verified_tk(input, Token::Boolean))
        .or_else(|_| verified_tk(input, Token::Null))?;
    let lit = match t.kind {
        Token::String => Literal::String(decode_string(&t.text)),
        Token::Number => Literal::Number(t.text.parse().unwrap_or(0.0)),
        Token::Boolean => Literal::Boolean(t.text == "true"),
        Token::Null => Literal::Null,
        _ => unreachable!(),
    };
    Ok(Expression::Literal(lit))
}

fn string_value(input: &mut In<'_>) -> Pr<String> {
    verified_tk(input, Token::String).map(|t| decode_string(&t.text))
}

/// `if <cond> { ... } else { ... }` — usable as an expression or a statement.
pub fn if_expr(input: &mut In<'_>) -> Pr<IfExpr> {
    tk(input, Token::If)?;
    let condition = expression(input)?;
    let then_branch = block(input)?;
    let else_branch = if opt_tk(input, Token::Else) {
        Some(else_tail(input)?)
    } else {
        None
    };
    Ok(IfExpr {
        condition,
        then_branch,
        else_branch,
    })
}

/// `match <expr> { <arm>* }` where each arm is `pattern => value`.
fn match_expr(input: &mut In<'_>) -> Pr<MatchExpr> {
    tk(input, Token::Match)?;
    let value = expression(input)?;
    tk(input, Token::LBrace)?;
    let mut arms = Vec::new();
    while !matches!(input.first().map(|t| t.kind), Some(Token::RBrace)) {
        let pattern = match_pattern(input)?;
        tk(input, Token::Arrow)?;
        let arm_value = expression(input)?;
        arms.push(MatchArm {
            pattern,
            value: arm_value,
        });
        opt_tk(input, Token::Comma);
    }
    tk(input, Token::RBrace)?;
    Ok(MatchExpr { value, arms })
}

fn match_pattern(input: &mut In<'_>) -> Pr<MatchPattern> {
    match input.first().map(|t| t.kind) {
        Some(Token::Ident) => {
            let first = ident_name(input)?;
            if first == "_" {
                return Ok(MatchPattern::Wildcard);
            }
            if opt_tk(input, Token::DoubleColon) {
                let variant = ident_name(input)?;
                if opt_tk(input, Token::LParen) {
                    let binding = ident_name(input)?;
                    tk(input, Token::RParen)?;
                    Ok(MatchPattern::EnumPayload {
                        enum_name: first,
                        variant,
                        binding,
                    })
                } else {
                    Ok(MatchPattern::Enum(EnumRef {
                        enum_name: first,
                        variant,
                    }))
                }
            } else {
                // A bare identifier matches nothing in the current grammar;
                // only literals, enum members, and `_` are valid.
                Err(ErrMode::Cut(PErr::unexpected(input)))
            }
        }
        Some(Token::String) | Some(Token::Number) | Some(Token::Boolean) | Some(Token::Null) => {
            let expr = literal_value(input)?;
            if let Expression::Literal(lit) = expr {
                Ok(MatchPattern::Literal(lit))
            } else {
                Err(ErrMode::Cut(PErr::unexpected(input)))
            }
        }
        _ => Err(backtrack(input)),
    }
}

/// After `else`: an `if` (else-if) or a trailing block.
fn else_tail(input: &mut In<'_>) -> Pr<Block> {
    if matches!(input.first().map(|t| t.kind), Some(Token::If)) {
        if_expr(input).map(|e| Block {
            statements: vec![Statement::Expr(Expression::If(Box::new(e)))],
        })
    } else {
        block(input)
    }
}

/// Decode a quoted string token (with quotes and escapes) into its value.
pub fn decode_string(raw: &str) -> String {
    let close = raw.chars().next().unwrap_or('"');
    let mut chars = raw[1..].chars().peekable();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c == close {
            break;
        }
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some(other) => out.push(other),
                None => break,
            }
        } else {
            out.push(c);
        }
    }
    out
}
