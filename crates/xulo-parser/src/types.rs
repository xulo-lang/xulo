use pest::iterators::Pair;

use xulo_core::ast::Type;
use xulo_core::error::XuloError;

use super::expr::decode_string;
use super::{Rule, err, span_of};

pub(crate) type R<T> = Result<T, XuloError>;

/// Lower a `type_expr` pair (spec: `Type`).
pub(crate) fn type_expr(pair: Pair<'_, Rule>) -> R<Type> {
    let inner = single(pair);
    check_rule(&inner, Rule::union_type)?;
    union_type(inner)
}

fn union_type(pair: Pair<'_, Rule>) -> R<Type> {
    let mut parts = Vec::new();
    for inner in pair.into_inner() {
        check_rule(&inner, Rule::intersection_type)?;
        parts.push(intersection_type(inner)?);
    }
    if parts.len() == 1 {
        Ok(parts.pop().unwrap())
    } else {
        Ok(Type::Union(parts))
    }
}

fn intersection_type(pair: Pair<'_, Rule>) -> R<Type> {
    let mut parts = Vec::new();
    for inner in pair.into_inner() {
        check_rule(&inner, Rule::postfix_type)?;
        parts.push(postfix_type(inner)?);
    }
    if parts.len() == 1 {
        Ok(parts.pop().unwrap())
    } else {
        Ok(Type::Intersection(parts))
    }
}

fn postfix_type(pair: Pair<'_, Rule>) -> R<Type> {
    let mut inner = pair.into_inner();
    let primary = inner.next().expect("postfix_type always has a primary");
    check_rule(&primary, Rule::primary_type)?;
    let ty = primary_type(primary)?;
    let optional = inner.any(|p| p.as_rule() == Rule::opt_question);
    if optional {
        Ok(Type::Optional(Box::new(ty)))
    } else {
        Ok(ty)
    }
}

fn primary_type(pair: Pair<'_, Rule>) -> R<Type> {
    let inner = single(pair);
    match inner.as_rule() {
        Rule::named_type => named_type(inner),
        Rule::primitive_type => primitive_type(inner),
        Rule::function_type => function_type(inner),
        Rule::literal_type => literal_type(inner),
        Rule::paren_or_tuple_type => {
            let mut parts = Vec::new();
            for t in inner.into_inner() {
                check_rule(&t, Rule::type_expr)?;
                parts.push(type_expr(t)?);
            }
            if parts.len() == 1 {
                Ok(parts.pop().expect("one type"))
            } else {
                Ok(Type::Tuple(parts))
            }
        }
        _ => unreachable!("primary_type alternative {:?}", inner.as_rule()),
    }
}

fn named_type(pair: Pair<'_, Rule>) -> R<Type> {
    let mut inner = pair.into_inner();
    let name_pair = inner.next().expect("named_type has a type_path");
    check_rule(&name_pair, Rule::type_path)?;
    let name = name_pair.as_str().to_string();
    let args = match inner.next() {
        Some(type_list) => {
            check_rule(&type_list, Rule::type_args)?;
            let mut items = Vec::new();
            for t in single(type_list).into_inner() {
                check_rule(&t, Rule::type_expr)?;
                items.push(type_expr(t)?);
            }
            items
        }
        None => Vec::new(),
    };

    match name.as_str() {
        // `List` keeps its dedicated form only for exactly one argument; a
        // missing or surplus one stays an application so the arity rule
        // reports it (spec E0230). Every primitive value type is a keyword
        // (spec `types/primitive-types.md`), so it never reaches this rule:
        // a capitalized spelling is an ordinary name and stays a `Named`
        // head for the checker to resolve or reject.
        "List" if args.len() == 1 => Ok(Type::List(Box::new(args.into_iter().next().unwrap()))),
        // A named type keeps its written type arguments; whether they are the
        // right number, and whether they satisfy their bounds, is checked by
        // the semantic analyzer.
        _ => Ok(Type::Named(name, args)),
    }
}

/// Lower a `primitive_type` pair (spec: `PrimitiveType`) to its type. The
/// spelling *is* the node: these are keywords, never identifiers, and they
/// carry no type-argument list.
fn primitive_type(pair: Pair<'_, Rule>) -> R<Type> {
    let spelling = pair.as_str();
    Ok(match spelling {
        "bool" => Type::Boolean,
        "string" => Type::String,
        // `number`, `int`, and `float` all lower to the implementation's one
        // numeric type; the checker does not yet separate the three (spec
        // `types/primitive-types.md`, "User-facing layer").
        "number" | "int" | "float" => Type::Number,
        "null" => Type::Null,
        // `unit`, `unknown`, and the fixed-bit numerics keep a named head
        // spelled exactly as the keyword: they are known through their fixed
        // arity and are not yet types of their own in the checker.
        _ => Type::Named(spelling.to_string(), Vec::new()),
    })
}

fn function_type(pair: Pair<'_, Rule>) -> R<Type> {
    let mut params = Vec::new();
    let mut ret = None;
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::kw_async | Rule::kw_fn => {}
            Rule::fn_type_params => {
                for p in inner.into_inner() {
                    check_rule(&p, Rule::fn_type_param)?;
                    params.push(fn_type_param(p)?);
                }
            }
            Rule::type_expr => ret = Some(Box::new(type_expr(inner)?)),
            _ => unreachable!("function_type child {:?}", inner.as_rule()),
        }
    }
    Ok(Type::FnSig { params, ret })
}

fn fn_type_param(pair: Pair<'_, Rule>) -> R<Type> {
    let mut ty = None;
    for inner in pair.into_inner() {
        match inner.as_rule() {
            Rule::ident_raw | Rule::param_mode => {}
            Rule::type_expr => ty = Some(type_expr(inner)?),
            _ => unreachable!("fn_type_param child {:?}", inner.as_rule()),
        }
    }
    Ok(ty.unwrap_or(Type::Any))
}

fn literal_type(pair: Pair<'_, Rule>) -> R<Type> {
    let inner = single(pair);
    match inner.as_rule() {
        Rule::string_literal => Ok(Type::Literal(decode_string(inner.as_str()))),
        Rule::number_literal => Ok(Type::Literal(inner.as_str().to_string())),
        Rule::boolean_literal => Ok(Type::Boolean),
        _ => unreachable!("literal_type alternative {:?}", inner.as_rule()),
    }
}

pub(crate) fn single(pair: Pair<'_, Rule>) -> Pair<'_, Rule> {
    pair.into_inner()
        .next()
        .expect("expected exactly one inner pair")
}

pub(crate) fn check_rule(pair: &Pair<'_, Rule>, expected: Rule) -> R<()> {
    if pair.as_rule() == expected {
        Ok(())
    } else {
        Err(err(
            format!("expected {expected:?}, found {:?}", pair.as_rule()),
            span_of(pair),
        ))
    }
}
