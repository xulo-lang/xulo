use crate::ast::Type;
use crate::lexer::token::Token;

use super::expression::decode_string;
use super::{backtrack_p, ident_name, opt_tk, tk, verified_tk, In, Pr};

/// A full type expression:
///
/// ```text
/// type_expr       = union
/// union           = intersection { "|" intersection }
/// intersection    = postfix { "&" postfix }
/// postfix         = primary [ "?" ]
/// primary         = "string" | "number" | "boolean" | "null" | "object"
///                 | "list" "<" type_expr ">"
///                 | "fn" "(" [ fn_param { "," fn_param } ] ")" [ ":" type_expr ]
///                 | "{" [ field { "," field } [ "," ] ] "}"
///                 | string_literal
///                 | identifier
///                 | "(" type_expr ")"
/// field           = identifier ":" type_expr
/// fn_param        = [ identifier ":" ] type_expr
/// ```
pub fn type_expr(input: &mut In<'_>) -> Pr<Type> {
    union(input)
}

fn union(input: &mut In<'_>) -> Pr<Type> {
    let mut parts = vec![intersection(input)?];
    while opt_tk(input, Token::Pipe) {
        parts.push(intersection(input)?);
    }
    if parts.len() == 1 {
        Ok(parts.pop().unwrap())
    } else {
        Ok(Type::Union(parts))
    }
}

fn intersection(input: &mut In<'_>) -> Pr<Type> {
    let mut parts = vec![postfix(input)?];
    while opt_tk(input, Token::Amp) {
        parts.push(postfix(input)?);
    }
    if parts.len() == 1 {
        Ok(parts.pop().unwrap())
    } else {
        Ok(Type::Intersection(parts))
    }
}

fn postfix(input: &mut In<'_>) -> Pr<Type> {
    let ty = primary(input)?;
    if opt_tk(input, Token::Question) {
        Ok(Type::Optional(Box::new(ty)))
    } else {
        Ok(ty)
    }
}

fn primary(input: &mut In<'_>) -> Pr<Type> {
    match input.first().map(|t| t.kind) {
        Some(Token::Null) => {
            tk(input, Token::Null)?;
            Ok(Type::Null)
        }
        Some(Token::Fn) => fn_type(input),
        Some(Token::LBrace) => object_type(input),
        Some(Token::String) => {
            let t = verified_tk(input, Token::String)?;
            Ok(Type::Literal(decode_string(&t.text)))
        }
        Some(Token::Ident) => {
            let t = verified_tk(input, Token::Ident)?;
            Ok(match t.text.as_str() {
                "string" => Type::String,
                "number" => Type::Number,
                "boolean" => Type::Boolean,
                "object" => Type::Object,
                "list" => {
                    if opt_tk(input, Token::Lt) {
                        let inner = type_expr(input)?;
                        tk(input, Token::Gt)?;
                        Type::List(Box::new(inner))
                    } else {
                        Type::List(Box::new(Type::Any))
                    }
                }
                name => {
                    // Generic named type: `Result<T>` erases to the named type.
                    if opt_tk(input, Token::Lt) {
                        if !matches!(input.first().map(|t| t.kind), Some(Token::Gt)) {
                            loop {
                                type_expr(input)?;
                                if !opt_tk(input, Token::Comma) {
                                    break;
                                }
                            }
                        }
                        tk(input, Token::Gt)?;
                    }
                    Type::Named(name.to_string())
                }
            })
        }
        Some(Token::LParen) => {
            tk(input, Token::LParen)?;
            let ty = type_expr(input)?;
            tk(input, Token::RParen)?;
            Ok(ty)
        }
        _ => Err(backtrack_p(input)),
    }
}

fn object_type(input: &mut In<'_>) -> Pr<Type> {
    tk(input, Token::LBrace)?;
    let mut fields = Vec::new();
    loop {
        if matches!(input.first().map(|t| t.kind), Some(Token::RBrace)) {
            break;
        }
        let name = ident_name(input)?;
        tk(input, Token::Colon)?;
        let ty = type_expr(input)?;
        fields.push((name, ty));
        opt_tk(input, Token::Comma);
    }
    tk(input, Token::RBrace)?;
    Ok(Type::ObjectType(fields))
}

fn fn_type(input: &mut In<'_>) -> Pr<Type> {
    tk(input, Token::Fn)?;
    tk(input, Token::LParen)?;
    let mut params = Vec::new();
    if !matches!(input.first().map(|t| t.kind), Some(Token::RParen)) {
        loop {
            params.push(fn_param(input)?);
            if !opt_tk(input, Token::Comma) {
                break;
            }
        }
    }
    tk(input, Token::RParen)?;
    let ret = if opt_tk(input, Token::Colon) {
        Some(Box::new(type_expr(input)?))
    } else {
        None
    };
    Ok(Type::FnSig { params, ret })
}

/// A function-type parameter: either a bare type or a `name: type` pair.
fn fn_param(input: &mut In<'_>) -> Pr<Type> {
    let is_named = matches!(input.first().map(|t| t.kind), Some(Token::Ident))
        && matches!(input.get(1).map(|t| t.kind), Some(Token::Colon));
    if is_named {
        let _name = ident_name(input)?;
        tk(input, Token::Colon)?;
        type_expr(input)
    } else {
        type_expr(input)
    }
}
