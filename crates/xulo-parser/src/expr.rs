use std::ops::Range;

use pest::iterators::Pair;

use xulo_core::ast::{
    ArrowBody, ArrowClosure, AssignTarget, BinaryOp, BinaryOperator, Block, Call, CallArg,
    CallValue, ComponentStmt, EnumRef, ExprStmt, Expression, FnExpr, IfExpr, IndexExpr, IsExpr,
    Literal, LockExpr, MatchArm, MatchBody, MatchExpr, MatchPattern, MemberAccess, NullishExpr,
    ObjectField, Param, PatternBinder, RangeExpr, SpawnBody, SpawnExpr, SpawnField, SpawnTarget,
    Statement, TernaryExpr, Type, TypedMap, TypedMapEntry, UnaryOp, UnaryOperator,
};
use xulo_core::error::{ErrorKind, XuloError};

use super::build::{block, to_ui_elements};
use super::types::{R, check_rule, single, type_expr};
use super::{Rule, err, span_of};

pub(crate) fn expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let inner = single(pair);
    check_rule(&inner, Rule::ternary_expression)?;
    ternary_expression(inner)
}

/// Entry point for grammar positions that hold a bare `ternary_expression`
/// (statement and assignment right-hand sides) rather than an `expression`.
pub(crate) fn ternary_entry(pair: Pair<'_, Rule>) -> R<Expression> {
    check_rule(&pair, Rule::ternary_expression)?;
    ternary_expression(pair)
}

fn ternary_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let base = logical_or_expression(next(&mut inner, Rule::logical_or_expression))?;
    let rest: Vec<Pair<'_, Rule>> = inner.collect();
    if rest.len() < 2 {
        return Ok(base);
    }
    let then_value = expression(rest[0].clone())?;
    let else_value = ternary_expression(rest[1].clone())?;
    Ok(Expression::Ternary(Box::new(TernaryExpr {
        condition: base,
        then_value,
        else_value,
        span,
    })))
}

fn logical_or_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    left_assoc(
        pair,
        Rule::logical_and_expression,
        logical_and_expression,
        BinaryOperator::Or,
    )
}

fn logical_and_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    left_assoc(
        pair,
        Rule::nullish_expression,
        nullish_expression,
        BinaryOperator::And,
    )
}

fn nullish_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span_start = pair.as_span().start();
    let mut inner = pair.into_inner();
    let mut lhs = equality_expression(next(&mut inner, Rule::equality_expression))?;
    for p in inner {
        check_rule(&p, Rule::equality_expression)?;
        let rhs = equality_expression(p)?;
        let span = span_start..rhs.span().end;
        lhs = Expression::Nullish(Box::new(NullishExpr {
            left: Box::new(lhs),
            right: Box::new(rhs),
            span,
        }));
    }
    Ok(lhs)
}

fn equality_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span_start = pair.as_span().start();
    let mut inner = pair.into_inner();
    let mut lhs = is_expression(next(&mut inner, Rule::is_expression))?;
    while let Some(p) = inner.next() {
        let operator = match p.as_str().trim() {
            "==" => BinaryOperator::Eq,
            "!=" => BinaryOperator::Neq,
            _ => {
                check_rule(&p, Rule::eq_op)?;
                unreachable!()
            }
        };
        let rhs = is_expression(next(&mut inner, Rule::is_expression))?;
        lhs = bin(span_start, lhs, operator, rhs);
    }
    Ok(lhs)
}

fn is_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let base = relational_expression(next(&mut inner, Rule::relational_expression))?;
    let mut negated = false;
    let mut ty = None;
    for p in inner {
        match p.as_rule() {
            Rule::kw_is => {}
            Rule::kw_not => negated = true,
            Rule::type_expr => ty = Some(type_expr(p)?),
            _ => unreachable!("is_expression child {:?}", p.as_rule()),
        }
    }
    match ty {
        Some(type_) => Ok(Expression::Is(Box::new(IsExpr {
            expr: Box::new(base),
            negated,
            type_,
            span,
        }))),
        None => Ok(base),
    }
}

fn relational_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span_start = pair.as_span().start();
    let mut inner = pair.into_inner();
    let lhs = range_expression(next(&mut inner, Rule::range_expression))?;
    let Some(op_pair) = inner.next() else {
        return Ok(lhs);
    };
    let operator = match op_pair.as_str().trim() {
        "<=" => BinaryOperator::Lte,
        ">=" => BinaryOperator::Gte,
        "<" => BinaryOperator::Lt,
        ">" => BinaryOperator::Gt,
        _ => unreachable!("rel_op"),
    };
    let rhs = range_expression(next(&mut inner, Rule::range_expression))?;
    Ok(bin(span_start, lhs, operator, rhs))
}

fn range_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let lhs = bit_or_expression(next(&mut inner, Rule::bit_or_expression))?;
    let Some(op_pair) = inner.next() else {
        return Ok(lhs);
    };
    let end_inclusive = op_pair.as_str().trim() == "...";
    let rhs = bit_or_expression(next(&mut inner, Rule::bit_or_expression))?;
    Ok(Expression::Range(Box::new(RangeExpr {
        start: Box::new(lhs),
        end: Box::new(rhs),
        end_inclusive,
        span,
    })))
}

fn bit_or_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    left_assoc(
        pair,
        Rule::bit_xor_expression,
        bit_xor_expression,
        BinaryOperator::BitOr,
    )
}

fn bit_xor_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    left_assoc(
        pair,
        Rule::bit_and_expression,
        bit_and_expression,
        BinaryOperator::Xor,
    )
}

fn bit_and_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    left_assoc(
        pair,
        Rule::shift_expression,
        shift_expression,
        BinaryOperator::BitAnd,
    )
}

fn shift_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span_start = pair.as_span().start();
    let mut inner = pair.into_inner();
    let mut lhs = additive_expression(next(&mut inner, Rule::additive_expression))?;
    while let Some(p) = inner.next() {
        let operator = match p.as_str().trim() {
            "<<" => BinaryOperator::Shl,
            ">>" => BinaryOperator::Shr,
            _ => {
                check_rule(&p, Rule::shift_op)?;
                unreachable!()
            }
        };
        let rhs = additive_expression(next(&mut inner, Rule::additive_expression))?;
        lhs = bin(span_start, lhs, operator, rhs);
    }
    Ok(lhs)
}

fn additive_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span_start = pair.as_span().start();
    let mut inner = pair.into_inner();
    let mut lhs = multiplicative_expression(next(&mut inner, Rule::multiplicative_expression))?;
    while let Some(p) = inner.next() {
        let operator = match p.as_str().trim() {
            "+" => BinaryOperator::Add,
            "-" => BinaryOperator::Sub,
            _ => {
                check_rule(&p, Rule::add_op)?;
                unreachable!()
            }
        };
        let rhs = multiplicative_expression(next(&mut inner, Rule::multiplicative_expression))?;
        lhs = bin(span_start, lhs, operator, rhs);
    }
    Ok(lhs)
}

fn multiplicative_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span_start = pair.as_span().start();
    let mut inner = pair.into_inner();
    let mut lhs = power_expression(next(&mut inner, Rule::power_expression))?;
    while let Some(p) = inner.next() {
        let operator = match p.as_str().trim() {
            "*" => BinaryOperator::Mul,
            "/" => BinaryOperator::Div,
            "%" => BinaryOperator::Mod,
            _ => {
                check_rule(&p, Rule::mul_op)?;
                unreachable!()
            }
        };
        let rhs = power_expression(next(&mut inner, Rule::power_expression))?;
        lhs = bin(span_start, lhs, operator, rhs);
    }
    Ok(lhs)
}

fn power_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span_start = pair.as_span().start();
    let mut inner = pair.into_inner();
    let base = unary_expression(next(&mut inner, Rule::unary_expression))?;
    let Some(exp_pair) = inner.next() else {
        return Ok(base);
    };
    let exp = power_expression(exp_pair)?;
    Ok(bin(span_start, base, BinaryOperator::Pow, exp))
}

fn unary_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().expect("unary_expression has a child");
    if first.as_rule() == Rule::postfix_expression {
        return postfix_expression(first);
    }
    check_rule(&first, Rule::unary_op)?;
    let operator_text = first.as_str().trim();
    let operand = unary_expression(next(&mut inner, Rule::unary_expression))?;
    match operator_text {
        "!" | "-" | "~" | "move" | "copy" => {
            let operator = match operator_text {
                "!" => UnaryOperator::Not,
                "-" => UnaryOperator::Neg,
                "~" => UnaryOperator::BitNot,
                "move" => UnaryOperator::Move,
                "copy" => UnaryOperator::Copy,
                _ => unreachable!("unary_op"),
            };
            Ok(Expression::Unary(Box::new(UnaryOp {
                operator,
                operand,
                span,
            })))
        }
        "await" => Ok(Expression::Await {
            expr: Box::new(operand),
            span,
        }),
        _ => unreachable!("unary_op `{operator_text}`"),
    }
}

fn postfix_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span_start = pair.as_span().start();
    let mut inner = pair.into_inner();
    let mut expr = primary_expression(next(&mut inner, Rule::primary_expression))?;
    for op in inner {
        expr = apply_postfix(expr, op, span_start)?;
    }
    Ok(expr)
}

fn apply_postfix(expr: Expression, op: Pair<'_, Rule>, span_start: usize) -> R<Expression> {
    let span = span_start..op.as_span().end();
    let is_optional_member = op.as_str().starts_with("?.");
    let inner = single(op);
    match inner.as_rule() {
        Rule::call_args => {
            // `call_args` may open with a written `CallTypeArgs` list
            // (`first<int>(…)`); PEG backtracking already dropped the
            // alternative when the material is not a type list followed by
            // `(`, so whatever matched here is a type-argument reading.
            let mut type_args = None;
            let mut args = Vec::new();
            for part in inner.into_inner() {
                match part.as_rule() {
                    Rule::call_type_args => type_args = Some(call_type_args(part)?),
                    Rule::argument => args.push(argument(part)?),
                    _ => unreachable!("call_args child {:?}", part.as_rule()),
                }
            }
            Ok(make_call(expr, args, span, type_args))
        }
        Rule::expression => {
            let index = expression(inner)?;
            Ok(Expression::Index(Box::new(IndexExpr {
                object: Box::new(expr),
                index: Box::new(index),
                span,
            })))
        }
        Rule::identifier | Rule::integer_literal => {
            Ok(Expression::Member(Box::new(MemberAccess {
                object: expr,
                property: inner.as_str().to_string(),
                optional: is_optional_member,
                span,
            })))
        }
        Rule::propagation_q => Ok(Expression::Propagate {
            expr: Box::new(expr),
            span,
        }),
        _ => unreachable!("postfix_op alternative {:?}", inner.as_rule()),
    }
}

fn make_call(
    callee: Expression,
    arguments: Vec<CallArg>,
    span: Range<usize>,
    type_args: Option<Vec<Type>>,
) -> Expression {
    match callee {
        Expression::Identifier {
            name,
            span: name_span,
        } => Expression::Call(Call {
            callee: name,
            callee_span: Some(name_span),
            object: None,
            method: None,
            optional: false,
            arguments,
            span,
            trait_impl: None,
            type_args,
        }),
        Expression::EnumRef(r) => Expression::Call(Call {
            callee: format!("{}::{}", r.enum_name, r.variant),
            callee_span: None,
            object: None,
            method: None,
            optional: false,
            arguments,
            span,
            trait_impl: None,
            type_args,
        }),
        Expression::Member(m) => {
            let property = m.property.clone();
            Expression::Call(Call {
                callee: property.clone(),
                callee_span: None,
                object: Some(Box::new(m.object)),
                method: Some(property),
                optional: m.optional,
                arguments,
                span,
                trait_impl: None,
                type_args,
            })
        }
        other => Expression::CallValue(Box::new(CallValue {
            callee: Box::new(other),
            arguments,
            span,
            type_args,
        })),
    }
}

/// Build a written `CallTypeArgs` list — `<` … `>` with a comma-separated
/// type list, or the empty material of `f<>(…)`.
fn call_type_args(pair: Pair<'_, Rule>) -> R<Vec<Type>> {
    check_rule(&pair, Rule::call_type_args)?;
    let mut items = Vec::new();
    for part in pair.into_inner() {
        // The `gt_close` child is a named rule and shows up in the tree;
        // only `type_list` carries written arguments (absent for `f<>`).
        if part.as_rule() != Rule::type_list {
            check_rule(&part, Rule::gt_close)?;
            continue;
        }
        for t in part.into_inner() {
            check_rule(&t, Rule::type_expr)?;
            items.push(type_expr(t)?);
        }
    }
    Ok(items)
}

fn primary_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let inner = single(pair);
    match inner.as_rule() {
        Rule::literal_expression => literal_expression(inner),
        Rule::template_literal => template_literal(inner),
        Rule::variant_path => variant_path(inner),
        Rule::function_expression => function_expression(inner),
        Rule::arrow_closure => arrow_closure(inner),
        Rule::if_expression => Ok(Expression::If(Box::new(if_expression(inner)?))),
        Rule::match_expression => Ok(Expression::Match(Box::new(match_expression(inner)?))),
        Rule::panic_expression => panic_expression(inner),
        Rule::spawn_expression => Ok(Expression::Spawn(Box::new(spawn_expression(inner)?))),
        Rule::lock_expression => Ok(Expression::Lock(Box::new(lock_expression(inner)?))),
        Rule::typed_map => Ok(Expression::TypedMap(Box::new(typed_map(inner)?))),
        Rule::paren_or_tuple => paren_or_tuple(inner),
        Rule::list_literal => list_literal(inner),
        Rule::brace_map => brace_map(inner),
        Rule::self_expression => {
            let name_pair = single(inner);
            check_rule(&name_pair, Rule::kw_self)?;
            Ok(Expression::Identifier {
                name: "self".to_string(),
                span: span_of(&name_pair),
            })
        }
        Rule::identifier_expression => {
            let name_pair = single(inner);
            check_rule(&name_pair, Rule::identifier)?;
            Ok(Expression::Identifier {
                name: name_pair.as_str().to_string(),
                span: span_of(&name_pair),
            })
        }
        _ => unreachable!("primary alternative {:?}", inner.as_rule()),
    }
}

fn literal_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let inner = single(pair);
    let span = span_of(&inner);
    Ok(Expression::Literal {
        value: literal_of(inner)?,
        span,
    })
}

pub(crate) fn literal_of(pair: Pair<'_, Rule>) -> R<Literal> {
    match pair.as_rule() {
        Rule::number_literal => Ok(Literal::Number(decode_number(pair.as_str()))),
        Rule::string_literal => Ok(Literal::String(decode_string(pair.as_str()))),
        Rule::boolean_literal => Ok(Literal::Boolean(pair.as_str() == "true")),
        Rule::null_literal => Ok(Literal::Null),
        _ => unreachable!("literal {:?}", pair.as_rule()),
    }
}

/// A backtick template literal, desugared into `+`/`str()` concatenation:
/// `` `a${b}c` `` becomes `"a" + str(b) + "c"` (spec: literals.md).
fn template_literal(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let start = span.start;
    let mut acc: Option<Expression> = None;
    let mut text = String::new();
    let mut text_span: Option<Range<usize>> = None;
    for part in pair.into_inner() {
        match part.as_rule() {
            // Leading text run: the pair includes the opening backtick.
            Rule::template_start => {
                let raw = part.as_str();
                let decoded = decode_template_chunk(&raw[1..]);
                let part_span = span_of(&part);
                text_span = Some(match text_span {
                    Some(s) => s.start..part_span.end,
                    None => (part_span.start + 1)..part_span.end,
                });
                text.push_str(&decoded);
            }
            Rule::interpolation => {
                let interp_span = span_of(&part);
                let mut tail: Option<Pair<'_, Rule>> = None;
                let mut value: Option<Expression> = None;
                for inner in part.into_inner() {
                    match inner.as_rule() {
                        Rule::expression => value = Some(expression(inner)?),
                        Rule::interp_tail => tail = Some(inner),
                        _ => unreachable!("interpolation part {:?}", inner.as_rule()),
                    }
                }
                if !text.is_empty() {
                    let lit = Expression::Literal {
                        value: Literal::String(std::mem::take(&mut text)),
                        span: text_span.take().unwrap_or_else(|| interp_span.clone()),
                    };
                    acc = concat_add(acc, lit, start);
                }
                let value = value.expect("interpolation without an expression");
                let call = Expression::Call(Call {
                    callee: "str".into(),
                    callee_span: None,
                    object: None,
                    method: None,
                    optional: false,
                    arguments: vec![CallArg { name: None, value }],
                    span: interp_span.clone(),
                    trait_impl: None,
                    type_args: None,
                });
                acc = concat_add(acc, call, start);
                // Text following the interpolation is folded into the tail
                // (including its leading `}`); keep it for the next flush.
                if let Some(tail) = tail {
                    let raw = tail.as_str();
                    let decoded = decode_template_chunk(&raw[1..]);
                    let tail_span = span_of(&tail);
                    text_span = Some(match text_span {
                        Some(s) => s.start..tail_span.end,
                        None => (tail_span.start + 1)..tail_span.end,
                    });
                    text.push_str(&decoded);
                }
            }
            _ => unreachable!("template part {:?}", part.as_rule()),
        }
    }
    if !text.is_empty() {
        let lit = Expression::Literal {
            value: Literal::String(text),
            span: text_span.unwrap_or_else(|| span.clone()),
        };
        acc = concat_add(acc, lit, start);
    }
    Ok(acc.unwrap_or(Expression::Literal {
        value: Literal::String(String::new()),
        span,
    }))
}

fn only_expr(pair: Pair<'_, Rule>) -> Pair<'_, Rule> {
    pair.into_inner()
        .next()
        .expect("expected a lone expression pair")
}

fn concat_add(
    left: Option<Expression>,
    right: Expression,
    span_start: usize,
) -> Option<Expression> {
    match left {
        None => Some(right),
        Some(l) => {
            let span = span_start..right.span().end;
            Some(Expression::BinaryOp(Box::new(BinaryOp {
                left: l,
                operator: BinaryOperator::Add,
                right,
                span,
                list_concat: false,
            })))
        }
    }
}

/// Compose the `(base…, member)` of a qualified path from its head and the
/// `::`-separated segments after it: `form::fields::Field` becomes
/// base `form::fields`, member `Field`. The two-segment case the checker
/// historically saw (`Theme::Dark`) is just base `Theme`, member `Dark`.
fn split_path(head: Pair<'_, Rule>, mut segments: Vec<String>) -> (String, String) {
    let mut base = head.as_str().to_string();
    let member = segments.pop().expect("a path carries a member segment");
    for seg in segments {
        base.push_str("::");
        base.push_str(&seg);
    }
    (base, member)
}

fn variant_path(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let head = next(&mut inner, Rule::path_head);
    let mut segments = Vec::new();
    let mut args = None;
    for p in inner {
        match p.as_rule() {
            Rule::identifier => segments.push(p.as_str().to_string()),
            Rule::variant_call_args => args = Some(p),
            other => unreachable!("variant_path child {other:?}"),
        }
    }
    let (enum_name, variant) = split_path(head, segments);
    let Some(args) = args else {
        return Ok(Expression::EnumRef(EnumRef {
            enum_name,
            variant,
            span,
        }));
    };
    check_rule(&args, Rule::variant_call_args)?;
    let arguments = match args.into_inner().next() {
        Some(list) => {
            check_rule(&list, Rule::argument_list)?;
            argument_list(list)?
        }
        // `Time::now()` — the empty list is still a call.
        None => Vec::new(),
    };
    // A written list after the variant (`Color::Red<int>(…)`) is not part of
    // `variant_call_args`: the postfix pass reads it and the checker reports
    // `E0807` (constructors declare no type parameters).
    Ok(Expression::Call(Call {
        callee: format!("{enum_name}::{variant}"),
        callee_span: None,
        object: None,
        method: None,
        optional: false,
        arguments,
        span,
        trait_impl: None,
        type_args: None,
    }))
}

pub(crate) fn component_call(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let name_pair = next(&mut inner, Rule::identifier);
    let name_span = span_of(&name_pair);
    let name = name_pair.as_str().to_string();
    let mut args = Vec::new();
    let mut children = Vec::new();
    let mut type_args = None;
    for p in inner {
        match p.as_rule() {
            Rule::call_type_args => type_args = Some(call_type_args(p)?),
            Rule::argument_list => args = argument_list(p)?,
            Rule::component_block => children = to_ui_elements(p)?,
            _ => unreachable!("component_call child {:?}", p.as_rule()),
        }
    }
    Ok(Expression::Component(ComponentStmt {
        name,
        name_span,
        args,
        children,
        span,
        type_args,
    }))
}

fn function_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let mut is_async = false;
    let mut params = Vec::new();
    let mut return_type = None;
    let mut body = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::kw_async => is_async = true,
            Rule::kw_fn => {}
            Rule::closure_param_list => params = closure_params(p)?,
            Rule::type_expr => return_type = Some(type_expr(p)?),
            Rule::block => body = Some(block(p)?),
            _ => unreachable!("function_expression child {:?}", p.as_rule()),
        }
    }
    Ok(Expression::FnExpr(Box::new(FnExpr {
        params,
        return_type,
        body: body.expect("function_expression has a body"),
        is_async,
        span,
    })))
}

fn arrow_closure(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let mut is_async = false;
    let mut params = Vec::new();
    let mut return_type = None;
    let mut body = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::kw_async => is_async = true,
            Rule::arrow_params => params = arrow_params(p)?,
            Rule::type_expr => return_type = Some(type_expr(p)?),
            Rule::block => body = Some(ArrowBody::Block(block(p)?)),
            Rule::expression => body = Some(ArrowBody::Expr(expression(p)?)),
            _ => unreachable!("arrow_closure child {:?}", p.as_rule()),
        }
    }
    Ok(Expression::Arrow(Box::new(ArrowClosure {
        params,
        return_type,
        body: body.expect("arrow_closure has a body"),
        is_async,
        span,
    })))
}

fn arrow_params(pair: Pair<'_, Rule>) -> R<Vec<Param>> {
    match pair.into_inner().next() {
        None => Ok(Vec::new()),
        Some(p) if p.as_rule() == Rule::closure_param_list => closure_params(p),
        Some(p) => {
            check_rule(&p, Rule::identifier)?;
            let span = span_of(&p);
            Ok(vec![Param {
                name: p.as_str().to_string(),
                type_annotation: None,
                default: None,
                mode: None,
                span,
            }])
        }
    }
}

fn closure_params(pair: Pair<'_, Rule>) -> R<Vec<Param>> {
    let mut params = Vec::new();
    for p in pair.into_inner() {
        check_rule(&p, Rule::closure_param)?;
        let span = span_of(&p);
        let mut inner = p.into_inner();
        let name_pair = next(&mut inner, Rule::identifier);
        let name = name_pair.as_str().to_string();
        let type_annotation = match inner.next() {
            Some(t) => Some(type_expr(t)?),
            None => None,
        };
        params.push(Param {
            name,
            type_annotation,
            default: None,
            mode: None,
            span,
        });
    }
    Ok(params)
}

fn if_expression(pair: Pair<'_, Rule>) -> R<IfExpr> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let _kw = next(&mut inner, Rule::kw_if);
    let condition = expression(next(&mut inner, Rule::expression))?;
    let then_branch = block(next(&mut inner, Rule::block))?;
    let else_branch = match inner.next() {
        None => None,
        Some(kw) => {
            check_rule(&kw, Rule::kw_else)?;
            let tail = inner.next().expect("else has a tail");
            match tail.as_rule() {
                Rule::block => Some(block(tail)?),
                Rule::if_expression => {
                    let e = if_expression(tail)?;
                    let span = e.span.clone();
                    Some(Block {
                        statements: vec![Statement::Expr(ExprStmt {
                            expr: Expression::If(Box::new(e)),
                            has_semicolon: false,
                            span,
                        })],
                    })
                }
                _ => unreachable!("else tail {:?}", tail.as_rule()),
            }
        }
    };
    Ok(IfExpr {
        condition,
        then_branch,
        else_branch,
        span,
    })
}

fn match_expression(pair: Pair<'_, Rule>) -> R<MatchExpr> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let _kw = next(&mut inner, Rule::kw_match);
    let value = expression(next(&mut inner, Rule::expression))?;
    let mut arms = Vec::new();
    if let Some(arm_list) = inner.next() {
        check_rule(&arm_list, Rule::match_arm_list)?;
        for arm in arm_list.into_inner() {
            check_rule(&arm, Rule::match_arm)?;
            arms.push(match_arm(arm)?);
        }
    }
    Ok(MatchExpr { value, arms, span })
}

fn match_arm(pair: Pair<'_, Rule>) -> R<MatchArm> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let pattern = pattern(next(&mut inner, Rule::pattern))?;
    let mut guard = None;
    let mut value = None;
    for p in inner {
        match p.as_rule() {
            Rule::guard => {
                let mut g = p.into_inner();
                let _kw = next(&mut g, Rule::kw_if);
                let e = next(&mut g, Rule::expression);
                guard = Some(expression(e)?);
            }
            Rule::block => value = Some(MatchBody::Block(block(p)?)),
            Rule::expression => value = Some(MatchBody::Expr(expression(p)?)),
            _ => unreachable!("match_arm child {:?}", p.as_rule()),
        }
    }
    Ok(MatchArm {
        pattern,
        guard,
        value: value.expect("match_arm has a body"),
        span,
    })
}

fn panic_expression(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let _kw = next(&mut inner, Rule::kw_panic);
    let e = expression(next(&mut inner, Rule::expression))?;
    Ok(Expression::Panic {
        expr: Box::new(e),
        span,
    })
}

fn spawn_expression(pair: Pair<'_, Rule>) -> R<SpawnExpr> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let _kw = next(&mut inner, Rule::kw_spawn);
    let inner = inner.next().expect("spawn_expression has an alternative");
    match inner.as_rule() {
        Rule::service_spawn => service_spawn(inner, span),
        Rule::task_spawn => task_spawn(inner, span),
        _ => unreachable!("spawn alternative {:?}", inner.as_rule()),
    }
}

fn task_spawn(pair: Pair<'_, Rule>, span: Range<usize>) -> R<SpawnExpr> {
    let mut target = SpawnTarget::Default;
    let mut body = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::spawn_target => target = spawn_target(p)?,
            Rule::kw_async => {}
            Rule::block => body = Some(SpawnBody::Async(block(p)?)),
            _ => unreachable!("task_spawn child {:?}", p.as_rule()),
        }
    }
    Ok(SpawnExpr {
        target,
        body: body.expect("task_spawn has a body"),
        span,
    })
}

fn spawn_target(pair: Pair<'_, Rule>) -> R<SpawnTarget> {
    let text = pair.as_str().to_string();
    let inner = pair.into_inner().next();
    if let Some(e) = inner {
        check_rule(&e, Rule::expression)?;
        Ok(SpawnTarget::On(Box::new(expression(e)?)))
    } else if text.starts_with(".thread") {
        Ok(SpawnTarget::Thread)
    } else {
        Ok(SpawnTarget::Process)
    }
}

fn service_spawn(pair: Pair<'_, Rule>, span: Range<usize>) -> R<SpawnExpr> {
    let mut type_name = String::new();
    let mut fields = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::type_expr => type_name = type_expr(p)?.name(),
            Rule::field_init => {
                let mut inner = p.into_inner();
                let name = next(&mut inner, Rule::identifier).as_str().to_string();
                let value = expression(next(&mut inner, Rule::expression))?;
                fields.push(SpawnField { name, value });
            }
            _ => unreachable!("service_spawn child {:?}", p.as_rule()),
        }
    }
    Ok(SpawnExpr {
        target: SpawnTarget::Process,
        body: SpawnBody::Service { type_name, fields },
        span,
    })
}

fn lock_expression(pair: Pair<'_, Rule>) -> R<LockExpr> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let _kw = next(&mut inner, Rule::kw_lock);
    let name_pair = next(&mut inner, Rule::identifier);
    let name_span = span_of(&name_pair);
    let name = name_pair.as_str().to_string();
    let body = block(next(&mut inner, Rule::block))?;
    Ok(LockExpr {
        name,
        name_span,
        body,
        span,
    })
}

fn typed_map(pair: Pair<'_, Rule>) -> R<TypedMap> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let _map_kw = next(&mut inner, Rule::map_kw);
    let type_args = next(&mut inner, Rule::type_args);
    let mut types = Vec::new();
    for t in single(type_args).into_inner() {
        check_rule(&t, Rule::type_expr)?;
        types.push(type_expr(t)?);
    }
    let key_type = types.first().cloned().unwrap_or(Type::Any);
    let value_type = types.get(1).cloned().unwrap_or(Type::Any);
    let mut entries = Vec::new();
    for e in inner {
        check_rule(&e, Rule::map_entry)?;
        let mut ep = e.into_inner();
        let key = expression(next(&mut ep, Rule::expression))?;
        let value = expression(next(&mut ep, Rule::expression))?;
        entries.push(TypedMapEntry { key, value });
    }
    Ok(TypedMap {
        key_type,
        value_type,
        entries,
        span,
    })
}

fn paren_or_tuple(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let mut items = Vec::new();
    for e in pair.into_inner() {
        check_rule(&e, Rule::expression)?;
        items.push(expression(e)?);
    }
    match items.len() {
        // `( e )` groups; the AST has no grouping node, so return the inner
        // expression with its own span.
        1 => Ok(items.pop().expect("one item")),
        _ => Ok(Expression::Tuple { items, span }),
    }
}

fn list_literal(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let mut items = Vec::new();
    for e in pair.into_inner() {
        check_rule(&e, Rule::list_element)?;
        let e = single(e);
        match e.as_rule() {
            Rule::spread => {
                let spread_span = span_of(&e);
                items.push(Expression::Spread {
                    expr: Box::new(expression(only_expr(e))?),
                    span: spread_span,
                });
            }
            Rule::expression => items.push(expression(e)?),
            _ => unreachable!("list element {:?}", e.as_rule()),
        }
    }
    Ok(Expression::Literal {
        value: Literal::List(items),
        span,
    })
}

fn brace_map(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let mut fields = Vec::new();
    for f in pair.into_inner() {
        check_rule(&f, Rule::map_field)?;
        let mut inner = f.into_inner();
        let first = inner.next().expect("map_field has children");
        match first.as_rule() {
            Rule::spread => {
                let value = expression(only_expr(first))?;
                fields.push(ObjectField::Spread { value });
            }
            Rule::identifier => {
                let name = first.as_str().to_string();
                let value = expression(next(&mut inner, Rule::expression))?;
                fields.push(ObjectField::Field { name, value });
            }
            _ => unreachable!("map_field child {:?}", first.as_rule()),
        }
    }
    Ok(Expression::Literal {
        value: Literal::Object(fields),
        span,
    })
}

fn argument_list(pair: Pair<'_, Rule>) -> R<Vec<CallArg>> {
    let mut args = Vec::new();
    for a in pair.into_inner() {
        check_rule(&a, Rule::argument)?;
        args.push(argument(a)?);
    }
    Ok(args)
}

fn argument(pair: Pair<'_, Rule>) -> R<CallArg> {
    let mut name = None;
    let mut value = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::ident_raw => name = Some(p.as_str().to_string()),
            Rule::dollar_place => value = Some(dollar_place(p)?),
            Rule::expression => value = Some(expression(p)?),
            _ => unreachable!("argument child {:?}", p.as_rule()),
        }
    }
    Ok(CallArg {
        name,
        value: value.expect("argument has a value"),
    })
}

/// `$place` in argument position: a `$`-prefixed binding reference. A dotted
/// or indexed place is kept as its rendered text so the checker rejects it
/// (spec: binding.md — only a single state cell may be bound).
fn dollar_place(pair: Pair<'_, Rule>) -> R<Expression> {
    let span = span_of(&pair);
    let place = single(pair);
    Ok(Expression::Binding {
        name: place_text(place),
        span,
    })
}

fn place_text(pair: Pair<'_, Rule>) -> String {
    match pair.as_rule() {
        Rule::place => {
            let mut text = String::new();
            for p in pair.into_inner() {
                match p.as_rule() {
                    Rule::place_primary => text.push_str(&place_text(p)),
                    Rule::place_op => text.push_str(&place_op_text(p)),
                    _ => unreachable!("place child {:?}", p.as_rule()),
                }
            }
            text
        }
        Rule::place_primary => {
            let inner = single(pair);
            match inner.as_rule() {
                Rule::ident_raw => inner.as_str().to_string(),
                Rule::place => format!("({})", place_text(inner)),
                _ => unreachable!("place_primary {:?}", inner.as_rule()),
            }
        }
        Rule::place_op => place_op_text(pair),
        _ => unreachable!("place {:?}", pair.as_rule()),
    }
}

fn place_op_text(pair: Pair<'_, Rule>) -> String {
    let inner = single(pair);
    match inner.as_rule() {
        Rule::ident_raw | Rule::integer_literal => format!(".{}", inner.as_str()),
        Rule::expression => format!("[{}]", inner.as_str()),
        _ => unreachable!("place_op {:?}", inner.as_rule()),
    }
}

/// The left-hand side of an assignment. The grammar takes any expression
/// there so a target that is not a place — `f() = 2`, `1 = 2` — fails here
/// with the semantic diagnostic the error table promises (spec
/// `statements/expression-statements.md`, E0402).
pub(crate) fn assign_target(expr: Expression, span: Range<usize>) -> R<AssignTarget> {
    fn is_place(expr: &Expression) -> bool {
        match expr {
            Expression::Identifier { .. } => true,
            Expression::Member(m) => is_place(&m.object),
            Expression::Index(i) => is_place(&i.object),
            _ => false,
        }
    }
    if !is_place(&expr) {
        return Err(
            XuloError::new(ErrorKind::Semantic, "invalid assignment target")
                .with_code("E0402")
                .at(span),
        );
    }
    Ok(match expr {
        Expression::Identifier { name, .. } => AssignTarget::Name(name),
        Expression::Member(m) => AssignTarget::Member(Box::new(m.object), m.property),
        Expression::Index(i) => AssignTarget::Index(i.object, i.index),
        _ => unreachable!("assign_target accepted only place shapes"),
    })
}

pub(crate) fn pattern(pair: Pair<'_, Rule>) -> R<MatchPattern> {
    let span = span_of(&pair);
    let mut alts = Vec::new();
    for a in pair.into_inner() {
        check_rule(&a, Rule::pattern_alt)?;
        alts.push(pattern_alt(a)?);
    }
    if alts.len() == 1 {
        Ok(alts.pop().unwrap())
    } else {
        Ok(MatchPattern::Or {
            patterns: alts,
            span,
        })
    }
}

fn pattern_alt(pair: Pair<'_, Rule>) -> R<MatchPattern> {
    let span = span_of(&pair);
    let inner = single(pair);
    match inner.as_rule() {
        Rule::range_pattern => range_pattern(inner, span),
        Rule::variant_pattern => variant_pattern(inner, span),
        Rule::struct_pattern => struct_pattern(inner, span),
        Rule::type_pattern => type_pattern(inner, span),
        Rule::paren_pattern => {
            let p = single(inner);
            check_rule(&p, Rule::pattern)?;
            pattern(p)
        }
        Rule::literal_pattern => Ok(MatchPattern::Literal(literal_of(single(inner))?)),
        Rule::wildcard_binding => {
            let name = inner.as_str();
            if name == "_" {
                Ok(MatchPattern::Wildcard)
            } else {
                Ok(MatchPattern::Binding {
                    name: name.to_string(),
                    span,
                })
            }
        }
        _ => unreachable!("pattern_alt {:?}", inner.as_rule()),
    }
}

fn range_pattern(pair: Pair<'_, Rule>, span: Range<usize>) -> R<MatchPattern> {
    let mut inner = pair.into_inner();
    let start = decode_number(next(&mut inner, Rule::number_literal).as_str());
    let end_inclusive = next(&mut inner, Rule::range_pat_op).as_str() == "...";
    let end = decode_number(next(&mut inner, Rule::number_literal).as_str());
    Ok(MatchPattern::Range {
        start,
        end,
        end_inclusive,
        span,
    })
}

fn variant_pattern(pair: Pair<'_, Rule>, span: Range<usize>) -> R<MatchPattern> {
    let mut inner = pair.into_inner();
    let head = next(&mut inner, Rule::path_head);
    let mut segments = Vec::new();
    let mut pl = None;
    for p in inner {
        match p.as_rule() {
            Rule::identifier => segments.push(p.as_str().to_string()),
            Rule::pattern_list => pl = Some(p),
            other => unreachable!("variant_pattern child {other:?}"),
        }
    }
    let (enum_name, variant) = split_path(head, segments);
    let Some(pl) = pl else {
        return Ok(MatchPattern::Enum(EnumRef {
            enum_name,
            variant,
            span,
        }));
    };
    check_rule(&pl, Rule::pattern_list)?;
    let mut bindings = Vec::new();
    for item in pl.into_inner() {
        bindings.push(payload_binding(item)?);
    }
    Ok(MatchPattern::EnumPayload {
        enum_name,
        variant,
        bindings,
        span,
    })
}

fn payload_binding(pair: Pair<'_, Rule>) -> R<String> {
    let span = span_of(&pair);
    let bad = || {
        err(
            "enum payload patterns support only names and `_`",
            span.clone(),
        )
    };
    let mut inner = pair.into_inner();
    let first = inner.next().ok_or_else(bad)?;
    if inner.next().is_some() {
        return Err(bad());
    }
    let alt = single(first);
    if alt.as_rule() != Rule::wildcard_binding {
        return Err(bad());
    }
    let name = alt.as_str();
    if name == "_" {
        Ok("_".to_string())
    } else {
        Ok(name.to_string())
    }
}

fn struct_pattern(pair: Pair<'_, Rule>, span: Range<usize>) -> R<MatchPattern> {
    let mut inner = pair.into_inner();
    let name = next(&mut inner, Rule::identifier).as_str().to_string();
    let mut bindings = Vec::new();
    if let Some(pl) = inner.next() {
        check_rule(&pl, Rule::pattern_list)?;
        for item in pl.into_inner() {
            bindings.push(pattern(item)?);
        }
    }
    Ok(MatchPattern::Struct {
        name,
        bindings,
        span,
    })
}

fn type_pattern(pair: Pair<'_, Rule>, span: Range<usize>) -> R<MatchPattern> {
    let mut inner = pair.into_inner();
    let designator = next(&mut inner, Rule::pattern_type);
    let type_name = designator.as_str().trim().to_string();
    let optional = type_name
        .split('|')
        .any(|part| part.trim_end().ends_with('?'));
    let binder = match inner.next() {
        None => PatternBinder::Wildcard,
        Some(b) => {
            check_rule(&b, Rule::pattern_binder)?;
            let inner_b = single(b);
            match inner_b.as_rule() {
                Rule::wildcard_us => PatternBinder::Wildcard,
                Rule::identifier => PatternBinder::Binding(inner_b.as_str().to_string()),
                other => unreachable!("pattern_binder alternative {other:?}"),
            }
        }
    };
    Ok(MatchPattern::TypePattern {
        type_name,
        optional,
        binder,
        span,
    })
}

pub(crate) fn decode_number(raw: &str) -> f64 {
    let lower = raw.to_ascii_lowercase();
    let radix_body = |prefix: &str, radix| -> Option<f64> {
        lower
            .strip_prefix(prefix)
            .and_then(|body| i64::from_str_radix(body, radix).ok())
            .map(|v| v as f64)
    };
    if let Some(v) = radix_body("0x", 16) {
        return v;
    }
    if let Some(v) = radix_body("0b", 2) {
        return v;
    }
    if let Some(v) = radix_body("0o", 8) {
        return v;
    }
    raw.parse::<f64>().unwrap_or(0.0)
}

pub(crate) fn decode_string(raw: &str) -> String {
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
                Some('u') => out.push(decode_unicode_escape(&mut chars)),
                Some(other) => out.push(other),
                None => break,
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn decode_template_chunk(raw: &str) -> String {
    let mut chars = raw.chars().peekable();
    let mut out = String::new();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('`') => out.push('`'),
            Some('$') => out.push('$'),
            Some('u') => out.push(decode_unicode_escape(&mut chars)),
            Some(other) => out.push(other),
            None => break,
        }
    }
    out
}

fn decode_unicode_escape(chars: &mut std::iter::Peekable<std::str::Chars>) -> char {
    let mut hex = String::new();
    if chars.peek() == Some(&'{') {
        chars.next();
        for c in chars.by_ref() {
            if c == '}' {
                break;
            }
            hex.push(c);
        }
    } else {
        for _ in 0..4 {
            match chars.next() {
                Some(c) => hex.push(c),
                None => break,
            }
        }
    }
    u32::from_str_radix(&hex, 16)
        .ok()
        .and_then(char::from_u32)
        .unwrap_or('\u{fffd}')
}

fn bin(
    span_start: usize,
    left: Expression,
    operator: BinaryOperator,
    right: Expression,
) -> Expression {
    let span = span_start..right.span().end;
    Expression::BinaryOp(Box::new(BinaryOp {
        left,
        operator,
        right,
        span,
        list_concat: false,
    }))
}

fn left_assoc(
    pair: Pair<'_, Rule>,
    child_rule: Rule,
    child: fn(Pair<'_, Rule>) -> R<Expression>,
    operator: BinaryOperator,
) -> R<Expression> {
    let span_start = pair.as_span().start();
    let mut inner = pair.into_inner();
    let mut lhs = child(next(&mut inner, child_rule))?;
    for p in inner {
        if p.as_rule() == Rule::kw_and || p.as_rule() == Rule::kw_or {
            continue;
        }
        check_rule(&p, child_rule)?;
        let rhs = child(p)?;
        lhs = bin(span_start, lhs, operator, rhs);
    }
    Ok(lhs)
}

fn next<'i>(inner: &mut impl Iterator<Item = Pair<'i, Rule>>, rule: Rule) -> Pair<'i, Rule> {
    let p = inner
        .next()
        .unwrap_or_else(|| panic!("expected {rule:?}, found end of input"));
    if p.as_rule() != rule {
        panic!("expected {rule:?}, found {:?}", p.as_rule());
    }
    p
}
