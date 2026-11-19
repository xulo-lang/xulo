use std::ops::Range;

use pest::iterators::Pair;

use xulo_core::ast::{
    AssignStmt, AssignValue, AttributeArg, AttributeValue, Block, DeferBody, DeferStmt, EnumDef,
    EnumPayloadParam, EnumVariant, ExportItem, ExportStmt, ExprStmt, Expression, ExternBlockDecl,
    ExternItem, FnBound, FnDef, ForStmt, GenericParam, GenericParamKind, ImplDecl, LetBinding,
    ModDecl, Param, ParamMode, PathHead, ReturnStmt, Statement, StructDef, StructField, TestDef,
    TestOption, TraitDecl, TraitMethod, Type, TypeAlias, UseDecl, UseEntry, UseTarget, WhileStmt,
};
use xulo_core::error::XuloError;

use super::expr::{
    assign_target, decode_float, decode_integer, decode_string, expression, ternary_entry,
};
use super::types::{R, check_rule, single, type_expr};
use super::{Rule, err, span_of};

pub fn program(root: Pair<'_, Rule>) -> Result<xulo_core::ast::Program, XuloError> {
    let mut statements = Vec::new();
    for pair in root.into_inner() {
        match pair.as_rule() {
            Rule::EOI => {}
            Rule::header_item => statements.push(header_item(pair)?),
            Rule::declaration => statements.push(declaration(pair)?),
            _ => unreachable!("program child {:?}", pair.as_rule()),
        }
    }
    Ok(xulo_core::ast::Program { statements })
}

fn declaration(pair: Pair<'_, Rule>) -> R<Statement> {
    let inner = single(pair);
    match inner.as_rule() {
        Rule::fn_decl => {
            let is_pub = has_pub(&inner);
            let f = fn_decl(inner)?;
            Ok(if is_pub {
                Statement::Export(ExportStmt {
                    item: ExportItem::Fn(f),
                })
            } else {
                Statement::Fn(f)
            })
        }
        Rule::test_decl => {
            let f = test_decl(inner)?;
            Ok(Statement::Test(f))
        }
        // `ExternFnDecl` MUST carry `pub` (grammar enforces it): it lowers
        // to the ordinary export-fn path with `FnDef::abi` set (spec ffi.md).
        Rule::extern_fn_decl => {
            let f = extern_fn_decl(inner)?;
            Ok(Statement::Export(ExportStmt {
                item: ExportItem::Fn(f),
            }))
        }
        Rule::extern_block => extern_block(inner).map(Statement::ExternBlock),
        Rule::struct_decl => {
            let is_pub = has_pub(&inner);
            let s = struct_decl(inner)?;
            Ok(if is_pub {
                Statement::Export(ExportStmt {
                    item: ExportItem::Struct(s),
                })
            } else {
                Statement::Struct(s)
            })
        }
        Rule::enum_decl => {
            let is_pub = has_pub(&inner);
            let e = enum_decl(inner)?;
            Ok(if is_pub {
                Statement::Export(ExportStmt {
                    item: ExportItem::Enum(e),
                })
            } else {
                Statement::Enum(e)
            })
        }
        Rule::trait_decl => {
            let is_pub = has_pub(&inner);
            let t = trait_decl(inner)?;
            Ok(if is_pub {
                Statement::Export(ExportStmt {
                    item: ExportItem::Trait(t),
                })
            } else {
                Statement::Trait(t)
            })
        }
        Rule::impl_decl => impl_decl(inner).map(Statement::Impl),
        Rule::type_alias => {
            let is_pub = has_pub(&inner);
            let t = type_alias(inner)?;
            Ok(if is_pub {
                Statement::Export(ExportStmt {
                    item: ExportItem::Type(t),
                })
            } else {
                Statement::TypeAlias(t)
            })
        }
        Rule::let_decl => {
            let is_pub = has_pub(&inner);
            let b = let_decl(inner)?;
            Ok(if is_pub {
                Statement::Export(ExportStmt {
                    item: ExportItem::Let(b),
                })
            } else {
                Statement::Let(b)
            })
        }
        Rule::const_decl => {
            let is_pub = has_pub(&inner);
            let b = const_decl(inner)?;
            Ok(if is_pub {
                Statement::Export(ExportStmt {
                    item: ExportItem::Let(b),
                })
            } else {
                Statement::Let(b)
            })
        }
        _ => unreachable!("declaration alternative {:?}", inner.as_rule()),
    }
}

fn has_pub(pair: &Pair<'_, Rule>) -> bool {
    pair.clone()
        .into_inner()
        .any(|p| p.as_rule() == Rule::pub_marker)
}

/// Build one `attribute_list` child (spec `AttributeList = { Attribute }`).
fn attribute_list(pair: Pair<'_, Rule>) -> R<Vec<xulo_core::ast::Attribute>> {
    let mut out = Vec::new();
    for a in pair.into_inner() {
        check_rule(&a, Rule::attribute)?;
        out.push(attribute(a)?);
    }
    Ok(out)
}

/// Build a single `#[name]` or `#[name(arg, …)]` attribute.
fn attribute(pair: Pair<'_, Rule>) -> R<xulo_core::ast::Attribute> {
    let pair_span = span_of(&pair);
    let pair_text = pair.as_str().to_string();
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut args = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::attribute_name => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::attribute_args => {
                for a in p.into_inner() {
                    check_rule(&a, Rule::attribute_arg)?;
                    args.push(attribute_arg(a)?);
                }
            }
            _ => unreachable!("attribute child {:?}", p.as_rule()),
        }
    }
    // `#[name]` vs `#[name()]`: the written text decides; an attribute's span
    // covers `#` through the closing `]`, never the gap pest's implicit
    // whitespace swallows after it.
    let has_parens = pair_text.contains('(');
    let close = pair_text
        .rfind(']')
        .map(|i| pair_span.start + i + 1)
        .unwrap_or(pair_span.end);
    Ok(xulo_core::ast::Attribute {
        name,
        name_span,
        args,
        span: pair_span.start..close,
        has_parens,
    })
}

/// Build one `[ Identifier "=" ] AttributeValue` argument (spec grammar.md).
fn attribute_arg(pair: Pair<'_, Rule>) -> R<AttributeArg> {
    let span = span_of(&pair);
    let mut name = None;
    let mut name_span = None;
    let mut value = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::identifier => {
                name = Some(p.as_str().to_string());
                name_span = Some(span_of(&p));
            }
            Rule::attribute_value => {
                let v = single(p);
                let val = match v.as_rule() {
                    Rule::identifier => AttributeValue::Identifier(v.as_str().to_string()),
                    Rule::string_literal => AttributeValue::String(decode_string(v.as_str())),
                    Rule::number_literal => match v.as_str().contains('.') {
                        // The decimal point, not the value, decides int vs
                        // float: `3.0` is a float literal, `3` is an integer.
                        true => AttributeValue::Float(decode_float(v.as_str())),
                        false => AttributeValue::Integer(
                            decode_integer(v.as_str())
                                .and_then(|n| i64::try_from(n).ok())
                                .unwrap_or(0),
                        ),
                    },
                    Rule::boolean_literal => AttributeValue::Boolean(v.as_str() == "true"),
                    other => unreachable!("attribute_value child {other:?}"),
                };
                value = Some(val);
            }
            other => unreachable!("attribute_arg child {other:?}"),
        }
    }
    Ok(AttributeArg {
        name,
        name_span,
        value: value.expect("attribute_arg has a value"),
        span,
    })
}

fn fn_decl(pair: Pair<'_, Rule>) -> R<FnDef> {
    let span = span_of(&pair);
    let mut is_async = false;
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_params = Vec::new();
    let mut generic_params = Vec::new();
    let mut bounds = Vec::new();
    let mut params = Vec::new();
    let mut return_type = None;
    let mut body = None;
    let mut attributes = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::attribute_list => attributes = attribute_list(p)?,
            Rule::pub_marker | Rule::kw_fn => {}
            Rule::kw_async => is_async = true,
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::generic_params => {
                let (tp, b, gp) = generic_params_of(p)?;
                type_params = tp;
                generic_params = gp;
                bounds.extend(b);
            }
            Rule::parameter_list => params = parameter_list(p)?,
            Rule::type_expr => return_type = Some(type_expr(p)?),
            Rule::where_clause => bounds.extend(where_clause(p)?),
            Rule::block => body = Some(block(p)?),
            _ => unreachable!("fn_decl child {:?}", p.as_rule()),
        }
    }
    let body = body.expect("fn_decl has a body");
    Ok(FnDef {
        name,
        name_span,
        params,
        return_type,
        type_params,
        generic_params,
        bounds,
        is_async,
        body,
        span,
        attributes,
        abi: None,
    })
}

/// Build a `#[test(options)] [async] fn name() { … }` into the fixed-shape
/// `TestDef` the `Statement::Test` variant carries: no parameters, no return
/// type, no generics (the grammar already forbids them; the fields are filled
/// empty). Option validity is a checking rule, so the options are recorded as
/// written.
fn test_decl(pair: Pair<'_, Rule>) -> R<TestDef> {
    let span = span_of(&pair);
    let mut is_async = false;
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut body = None;
    let mut options = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::kw_fn => {}
            Rule::kw_async => is_async = true,
            Rule::test_options => options = test_options(p),
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::block => body = Some(block(p)?),
            _ => unreachable!("test_decl child {:?}", p.as_rule()),
        }
    }
    Ok(TestDef {
        func: FnDef {
            name,
            name_span,
            params: Vec::new(),
            return_type: None,
            type_params: Vec::new(),
            generic_params: Vec::new(),
            bounds: Vec::new(),
            is_async,
            body: body.expect("test_decl has a body"),
            span,
            attributes: Vec::new(),
            abi: None,
        },
        options,
    })
}

/// Build the options of a `TestOptions` group, in written order.
fn test_options(pair: Pair<'_, Rule>) -> Vec<TestOption> {
    let mut options = Vec::new();
    for option in pair.into_inner() {
        if option.as_rule() != Rule::test_option {
            continue;
        }
        let option_span = span_of(&option);
        let inner = single(option);
        let mut strings = Vec::new();
        let mut integers = Vec::new();
        for child in inner.clone().into_inner() {
            let child_span = span_of(&child);
            match child.as_rule() {
                Rule::string_literal => strings.push((decode_string(child.as_str()), child_span)),
                Rule::integer_literal => {
                    let value = child
                        .as_str()
                        .parse::<u64>()
                        .ok()
                        .filter(|n| *n > 0)
                        .unwrap_or(0);
                    integers.push((value, child_span));
                }
                _ => {}
            }
        }
        let text = inner.as_str();
        match inner.as_rule() {
            Rule::test_expectation => {
                if text.starts_with("should_fail") {
                    options.push(TestOption::ShouldFail { span: option_span });
                } else {
                    let (message, message_span) = match strings.into_iter().next() {
                        Some((message, span)) => (Some(message), Some(span)),
                        None => (None, None),
                    };
                    options.push(TestOption::ShouldPanic {
                        message,
                        span: option_span,
                        message_span,
                    });
                }
            }
            Rule::test_skip => {
                let (reason, reason_span) = match strings.into_iter().next() {
                    Some((reason, span)) => (Some(reason), Some(span)),
                    None => (None, None),
                };
                options.push(TestOption::Skip {
                    reason,
                    span: option_span,
                    reason_span,
                });
            }
            Rule::test_timeout => {
                let (value, value_span) = integers
                    .into_iter()
                    .next()
                    .unwrap_or((0, option_span.clone()));
                options.push(TestOption::Timeout {
                    value,
                    span: option_span,
                    value_span,
                });
            }
            other => unreachable!("test_option child {other:?}"),
        }
    }
    options
}

/// `ExternFnDecl = "pub" "extern" AbiTag "fn" Identifier "("
/// [ ExternParamList ] ")" [ ":" Type ] Block` — built as an ordinary
/// `FnDef` carrying `abi`, ready for the export path (spec ffi.md).
fn extern_fn_decl(pair: Pair<'_, Rule>) -> R<FnDef> {
    let span = span_of(&pair);
    let mut abi = None;
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut params = Vec::new();
    let mut return_type = None;
    let mut body = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::pub_marker | Rule::kw_extern | Rule::kw_fn => {}
            Rule::abi_tag => abi = Some(abi_value(p)?),
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::extern_param_list => params = extern_param_list(p)?,
            Rule::type_expr => return_type = Some(type_expr(p)?),
            Rule::block => body = Some(block(p)?),
            _ => unreachable!("extern_fn_decl child {:?}", p.as_rule()),
        }
    }
    Ok(FnDef {
        name,
        name_span,
        params,
        return_type,
        type_params: Vec::new(),
        generic_params: Vec::new(),
        bounds: Vec::new(),
        is_async: false,
        body: body.expect("extern_fn_decl has a body"),
        span,
        attributes: Vec::new(),
        abi: Some(abi.expect("extern_fn_decl has an AbiTag")),
    })
}

/// `[pub] extern AbiTag "{" { ExternItem } "}"` — an FFI import block.
fn extern_block(pair: Pair<'_, Rule>) -> R<ExternBlockDecl> {
    let span = span_of(&pair);
    let is_pub = has_pub(&pair);
    let mut abi = None;
    let mut items = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::pub_marker | Rule::kw_extern => {}
            Rule::abi_tag => abi = Some(abi_value(p)?),
            Rule::extern_item => items.push(extern_item(p)?),
            _ => unreachable!("extern_block child {:?}", p.as_rule()),
        }
    }
    Ok(ExternBlockDecl {
        is_pub,
        abi: abi.expect("extern_block has an AbiTag"),
        items,
        span,
    })
}

/// The `AbiTag` string literal's decoded value.
fn abi_value(pair: Pair<'_, Rule>) -> R<String> {
    let lit = single(pair);
    check_rule(&lit, Rule::string_literal)?;
    Ok(decode_string(lit.as_str()))
}

/// One `fn name(…) [: T] [as local] ;` inside an extern block: a signature
/// declaration (no body, no mode, no defaults) whose `local` binding
/// defaults to the declared name.
fn extern_item(pair: Pair<'_, Rule>) -> R<ExternItem> {
    let span = span_of(&pair);
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut params = Vec::new();
    let mut return_type = None;
    let mut local_pair: Option<Pair<'_, Rule>> = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::kw_fn | Rule::kw_as => {}
            Rule::identifier => {
                if name.is_empty() {
                    name = p.as_str().to_string();
                    name_span = span_of(&p);
                } else {
                    local_pair = Some(p);
                }
            }
            Rule::extern_param_list => params = extern_param_list(p)?,
            Rule::type_expr => return_type = Some(type_expr(p)?),
            _ => unreachable!("extern_item child {:?}", p.as_rule()),
        }
    }
    let (local_name, local_name_span) = match local_pair {
        Some(lp) => {
            let s = span_of(&lp);
            (lp.as_str().to_string(), s)
        }
        None => (name.clone(), name_span.clone()),
    };
    Ok(ExternItem {
        local_name,
        local_name_span,
        name,
        name_span,
        params,
        return_type,
        span,
    })
}

/// `ExternParamList = ExternParam { "," ExternParam } [ "," ]` — each
/// `ExternParam = Identifier ":" Type` with no mode and no default.
fn extern_param_list(pair: Pair<'_, Rule>) -> R<Vec<Param>> {
    let mut params = Vec::new();
    for p in pair.into_inner() {
        check_rule(&p, Rule::extern_param)?;
        let span = span_of(&p);
        let mut inner = p.into_inner();
        let name_pair = inner.next().expect("extern_param has a name");
        let type_pair = inner.next().expect("extern_param has a type");
        check_rule(&name_pair, Rule::identifier)?;
        check_rule(&type_pair, Rule::type_expr)?;
        params.push(Param {
            name: name_pair.as_str().to_string(),
            type_annotation: Some(type_expr(type_pair)?),
            default: None,
            mode: None,
            span,
        });
    }
    Ok(params)
}

fn parameter_list(pair: Pair<'_, Rule>) -> R<Vec<Param>> {
    let mut params = Vec::new();
    for p in pair.into_inner() {
        check_rule(&p, Rule::parameter)?;
        params.push(parameter(p)?);
    }
    Ok(params)
}

fn parameter(pair: Pair<'_, Rule>) -> R<Param> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let first = inner.next().expect("parameter has a child");
    if first.as_rule() == Rule::receiver {
        let mode = if first.as_str().starts_with("mut") {
            Some(ParamMode::Mut)
        } else {
            None
        };
        return Ok(Param {
            name: "self".into(),
            type_annotation: None,
            default: None,
            mode,
            span,
        });
    }
    check_rule(&first, Rule::identifier)?;
    let name = first.as_str().to_string();
    let mut type_annotation = None;
    let mut default = None;
    let mut mode = None;
    for p in inner {
        match p.as_rule() {
            Rule::param_mode => {
                mode = Some(match p.as_str() {
                    "mut" => ParamMode::Mut,
                    "shared" => ParamMode::Shared,
                    _ => unreachable!("param_mode"),
                });
            }
            Rule::type_expr => type_annotation = Some(type_expr(p)?),
            Rule::expression => default = Some(expression(p)?),
            _ => unreachable!("parameter child {:?}", p.as_rule()),
        }
    }
    Ok(Param {
        name,
        type_annotation,
        default,
        mode,
        span,
    })
}

fn generic_params_of(pair: Pair<'_, Rule>) -> R<(Vec<String>, Vec<FnBound>, Vec<GenericParam>)> {
    let mut params = Vec::new();
    let mut bounds = Vec::new();
    let mut all = Vec::new();
    for p in pair.into_inner() {
        check_rule(&p, Rule::generic_param)?;
        let mut inner = p.into_inner();
        let first = inner.next().expect("generic_param has a child");
        if first.as_rule() == Rule::kw_const {
            // `const N: int` — the declared type follows the name.
            let name_pair = inner.next().expect("const parameter has a name");
            check_rule(&name_pair, Rule::identifier)?;
            let name = name_pair.as_str().to_string();
            let type_pair = inner.next().expect("const parameter has a type");
            check_rule(&type_pair, Rule::type_expr)?;
            all.push(GenericParam {
                name,
                kind: GenericParamKind::Const,
                const_type: Some(type_expr(type_pair)?),
            });
            continue;
        }
        check_rule(&first, Rule::identifier)?;
        let name = first.as_str().to_string();
        if let Some(tb) = inner.next() {
            check_rule(&tb, Rule::trait_bound)?;
            bounds.push(FnBound {
                param: name.clone(),
                traits: trait_bound(tb)?,
            });
        }
        params.push(name.clone());
        all.push(GenericParam {
            name,
            kind: GenericParamKind::Type,
            const_type: None,
        });
    }
    Ok((params, bounds, all))
}

fn trait_bound(pair: Pair<'_, Rule>) -> R<Vec<String>> {
    let mut traits = Vec::new();
    for p in pair.into_inner() {
        check_rule(&p, Rule::type_expr)?;
        let ty = type_expr(p.clone())?;
        // A bound names a trait, never an application of one: a trait is not a
        // type constructor (spec `types/generics.md`, E0811).
        if let Type::Named(_, args) = &ty
            && !args.is_empty()
        {
            return Err(err(
                format!(
                    "`{}` applies type arguments; a trait has none",
                    ty.display()
                ),
                span_of(&p),
            )
            .with_code("E0811"));
        }
        traits.push(ty.name());
    }
    Ok(traits)
}

fn where_clause(pair: Pair<'_, Rule>) -> R<Vec<FnBound>> {
    let mut bounds = Vec::new();
    for item in pair.into_inner() {
        if item.as_rule() == Rule::kw_where {
            continue;
        }
        check_rule(&item, Rule::where_item)?;
        let mut inner = item.into_inner();
        let name = inner
            .next()
            .expect("where_item has a name")
            .as_str()
            .to_string();
        let tb = inner.next().expect("where_item has a bound");
        check_rule(&tb, Rule::trait_bound)?;
        bounds.push(FnBound {
            param: name,
            traits: trait_bound(tb)?,
        });
    }
    Ok(bounds)
}

fn struct_decl(pair: Pair<'_, Rule>) -> R<StructDef> {
    let span = span_of(&pair);
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_params = Vec::new();
    let mut generic_params = Vec::new();
    let mut type_bounds = Vec::new();
    let mut fields = Vec::new();
    let mut attributes = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::attribute_list => attributes = attribute_list(p)?,
            Rule::pub_marker | Rule::kw_struct => {}
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::generic_params => {
                let (tp, bd, gp) = generic_params_of(p)?;
                type_params = tp;
                type_bounds = bd;
                generic_params = gp;
            }
            Rule::field_list => {
                for f in p.into_inner() {
                    check_rule(&f, Rule::field)?;
                    let inner = f.into_inner();
                    let mut field_name = String::new();
                    let mut field_name_span = 0..0;
                    let mut type_ = None;
                    for part in inner {
                        match part.as_rule() {
                            Rule::pub_marker => {}
                            Rule::identifier => {
                                field_name = part.as_str().to_string();
                                field_name_span = span_of(&part);
                            }
                            Rule::type_expr => type_ = Some(type_expr(part)?),
                            _ => unreachable!("field child {:?}", part.as_rule()),
                        }
                    }
                    fields.push(StructField {
                        name: field_name,
                        name_span: field_name_span,
                        type_: type_.expect("field has a type"),
                        default: None,
                    });
                }
            }
            _ => unreachable!("struct_decl child {:?}", p.as_rule()),
        }
    }
    Ok(StructDef {
        name,
        name_span,
        type_params,
        generic_params,
        type_bounds,
        fields,
        span,
        attributes,
    })
}

fn enum_decl(pair: Pair<'_, Rule>) -> R<EnumDef> {
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_params = Vec::new();
    let mut generic_params = Vec::new();
    let mut type_bounds = Vec::new();
    let mut variants = Vec::new();
    let mut attributes = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::attribute_list => attributes = attribute_list(p)?,
            Rule::pub_marker | Rule::kw_enum => {}
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::generic_params => {
                let (tp, bd, gp) = generic_params_of(p)?;
                type_params = tp;
                type_bounds = bd;
                generic_params = gp;
            }
            Rule::variant_list => {
                for v in p.into_inner() {
                    check_rule(&v, Rule::variant)?;
                    let mut inner = v.into_inner();
                    let name_pair = inner.next().expect("variant has a name");
                    let vname = name_pair.as_str().to_string();
                    let vname_span = span_of(&name_pair);
                    let payload = match inner.next() {
                        None => None,
                        Some(pl) => {
                            check_rule(&pl, Rule::variant_payload)?;
                            Some(variant_payload(pl)?)
                        }
                    };
                    variants.push(EnumVariant {
                        name: vname,
                        name_span: vname_span,
                        payload,
                    });
                }
            }
            _ => unreachable!("enum_decl child {:?}", p.as_rule()),
        }
    }
    Ok(EnumDef {
        name,
        name_span,
        type_params,
        generic_params,
        type_bounds,
        variants,
        attributes,
    })
}

fn variant_payload(pair: Pair<'_, Rule>) -> R<Vec<EnumPayloadParam>> {
    let mut params = Vec::new();
    let Some(inner) = pair.into_inner().next() else {
        return Ok(params);
    };
    match inner.as_rule() {
        Rule::name_type_list => {
            for np in inner.into_inner() {
                check_rule(&np, Rule::named_payload)?;
                let mut parts = np.into_inner();
                let name = parts
                    .next()
                    .expect("named_payload has a name")
                    .as_str()
                    .to_string();
                let ty = type_expr(parts.next().expect("named_payload has a type"))?;
                params.push(EnumPayloadParam {
                    name: Some(name),
                    type_: ty,
                });
            }
        }
        Rule::type_list => {
            for t in inner.into_inner() {
                check_rule(&t, Rule::type_expr)?;
                params.push(EnumPayloadParam {
                    name: None,
                    type_: type_expr(t)?,
                });
            }
        }
        _ => unreachable!("variant_payload {:?}", inner.as_rule()),
    }
    Ok(params)
}

fn trait_decl(pair: Pair<'_, Rule>) -> R<TraitDecl> {
    let span = span_of(&pair);
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_params = Vec::new();
    let mut generic_params = Vec::new();
    let mut methods = Vec::new();
    let mut attributes = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::attribute_list => attributes = attribute_list(p)?,
            Rule::pub_marker | Rule::kw_trait => {}
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::generic_params => {
                let (tp, _, gp) = generic_params_of(p)?;
                type_params = tp;
                generic_params = gp;
            }
            Rule::trait_methods => {
                for m in p.into_inner() {
                    check_rule(&m, Rule::trait_method)?;
                    methods.push(trait_method(m)?);
                }
            }
            _ => unreachable!("trait_decl child {:?}", p.as_rule()),
        }
    }
    Ok(TraitDecl {
        name,
        name_span,
        type_params,
        generic_params,
        methods,
        span,
        attributes,
    })
}

fn trait_method(pair: Pair<'_, Rule>) -> R<TraitMethod> {
    let span = span_of(&pair);
    let mut is_async = false;
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut params = Vec::new();
    let mut return_type = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::kw_fn => {}
            Rule::kw_async => is_async = true,
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::parameter_list => params = parameter_list(p)?,
            Rule::type_expr => return_type = Some(type_expr(p)?),
            _ => unreachable!("trait_method child {:?}", p.as_rule()),
        }
    }
    let has_self = params.first().is_some_and(|p| p.name == "self");
    let params = params.into_iter().filter(|p| p.name != "self").collect();
    Ok(TraitMethod {
        name,
        name_span,
        has_self,
        params,
        return_type,
        is_async,
        span,
    })
}

fn impl_decl(pair: Pair<'_, Rule>) -> R<ImplDecl> {
    let span = span_of(&pair);
    let mut trait_name = String::new();
    let mut type_name = String::new();
    let mut type_params = Vec::new();
    let mut generic_params = Vec::new();
    let mut type_bounds = Vec::new();
    let mut type_args = Vec::new();
    let mut methods = Vec::new();
    let mut is_inherent = true;
    let mut seen_type = false;
    let mut attributes = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::attribute_list => attributes = attribute_list(p)?,
            Rule::kw_impl | Rule::kw_for => {}
            Rule::generic_params => {
                let (params, bounds, gp) = generic_params_of(p)?;
                type_params = params;
                type_bounds = bounds;
                generic_params = gp;
            }
            Rule::identifier if !seen_type => {
                trait_name = p.as_str().to_string();
                is_inherent = false;
            }
            Rule::type_expr => {
                let ty = type_expr(p)?;
                type_name = ty.name();
                if let Type::Named(_, args) = ty {
                    type_args = args;
                }
                seen_type = true;
            }
            Rule::fn_decl => methods.push(fn_decl(p)?),
            _ => unreachable!("impl_decl child {:?}", p.as_rule()),
        }
    }
    Ok(ImplDecl {
        trait_name,
        type_name,
        type_params,
        generic_params,
        type_bounds,
        type_args,
        methods,
        span,
        is_inherent,
        attributes,
    })
}

fn type_alias(pair: Pair<'_, Rule>) -> R<TypeAlias> {
    let mut attributes = Vec::new();
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_params = Vec::new();
    let mut generic_params = Vec::new();
    let mut type_bounds = Vec::new();
    let mut type_ = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::attribute_list => attributes = attribute_list(p)?,
            Rule::pub_marker | Rule::kw_type => {}
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::generic_params => {
                let (tp, bd, gp) = generic_params_of(p)?;
                type_params = tp;
                type_bounds = bd;
                generic_params = gp;
            }
            Rule::type_expr => type_ = Some(type_expr(p)?),
            _ => unreachable!("type_alias child {:?}", p.as_rule()),
        }
    }
    Ok(TypeAlias {
        name,
        name_span,
        type_params,
        generic_params,
        type_bounds,
        type_: type_.expect("type_alias has a type"),
        attributes,
    })
}

fn let_decl(pair: Pair<'_, Rule>) -> R<LetBinding> {
    let mut binding = None;
    let mut attributes = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::attribute_list => attributes = attribute_list(p)?,
            Rule::pub_marker | Rule::kw_let => {}
            Rule::mut_binding | Rule::simple_binding | Rule::destructuring_binding => {
                binding = Some(p);
            }
            _ => unreachable!("let_decl child {:?}", p.as_rule()),
        }
    }
    let binding = binding.expect("let_decl has a binding");
    let mut out = match binding.as_rule() {
        Rule::mut_binding => mut_binding(binding),
        Rule::simple_binding => simple_binding(binding),
        Rule::destructuring_binding => destructuring_binding(binding),
        _ => unreachable!("let_decl alternative {:?}", binding.as_rule()),
    }?;
    out.attributes = attributes;
    Ok(out)
}

fn mut_binding(pair: Pair<'_, Rule>) -> R<LetBinding> {
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_annotation = None;
    let mut value = None;
    let mut is_shared = false;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::kw_mut | Rule::kw_shared => {
                if p.as_rule() == Rule::kw_shared {
                    is_shared = true;
                }
            }
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::type_expr => type_annotation = Some(type_expr(p)?),
            Rule::expression => value = Some(expression(p)?),
            _ => unreachable!("mut_binding child {:?}", p.as_rule()),
        }
    }
    Ok(LetBinding {
        name,
        name_span,
        type_annotation,
        value,
        is_const: false,
        is_mutable: true,
        is_shared,
        tuple_names: None,
        object_destructuring: None,
        attributes: Vec::new(),
    })
}

fn simple_binding(pair: Pair<'_, Rule>) -> R<LetBinding> {
    let text = pair.as_str().to_string();
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_annotation = None;
    let mut value = None;
    let mut is_mutable = false;
    let mut is_shared = false;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::kw_shared => is_shared = true,
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::type_expr => type_annotation = Some(type_expr(p)?),
            Rule::expression => value = Some(expression(p)?),
            _ => unreachable!("simple_binding child {:?}", p.as_rule()),
        }
    }
    // `let x := expr` is sugar for `let mut x = expr`; the `:=` spelling is
    // only reachable through this binding form (spec: let-and-assignment.md).
    let is_colon_assign = match text.find('=') {
        Some(i) => i > 0 && text.as_bytes()[i - 1] == b':',
        None => false,
    };
    if is_colon_assign {
        is_mutable = true;
    }
    Ok(LetBinding {
        name,
        name_span,
        type_annotation,
        value,
        is_const: false,
        is_mutable,
        is_shared,
        tuple_names: None,
        object_destructuring: None,
        attributes: Vec::new(),
    })
}

fn destructuring_binding(pair: Pair<'_, Rule>) -> R<LetBinding> {
    let span = span_of(&pair);
    let is_object = pair.as_str().trim_start().starts_with('{');
    let mut inner = pair.into_inner();
    let id_list = next(&mut inner, Rule::identifier_list);
    let value = expression(next(&mut inner, Rule::expression))?;
    let names: Vec<String> = id_list
        .into_inner()
        .map(|p| p.as_str().to_string())
        .collect();
    let (tuple_names, object_destructuring) = if is_object {
        (None, Some(names.into_iter().map(|n| (n, None)).collect()))
    } else {
        (Some(names), None)
    };
    Ok(LetBinding {
        name: String::new(),
        name_span: span,
        type_annotation: None,
        value: Some(value),
        is_const: false,
        is_mutable: false,
        is_shared: false,
        tuple_names,
        object_destructuring,
        attributes: Vec::new(),
    })
}

fn const_decl(pair: Pair<'_, Rule>) -> R<LetBinding> {
    let mut attributes = Vec::new();
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_annotation = None;
    let mut value = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::attribute_list => attributes = attribute_list(p)?,
            Rule::pub_marker | Rule::kw_const => {}
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::type_expr => type_annotation = Some(type_expr(p)?),
            Rule::expression => value = Some(expression(p)?),
            _ => unreachable!("const_decl child {:?}", p.as_rule()),
        }
    }
    Ok(LetBinding {
        name,
        name_span,
        type_annotation,
        value,
        is_const: true,
        is_mutable: false,
        is_shared: false,
        tuple_names: None,
        object_destructuring: None,
        attributes,
    })
}

fn return_stmt(pair: Pair<'_, Rule>) -> R<ReturnStmt> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let _kw = next(&mut inner, Rule::kw_return);
    let value = match inner.next() {
        None => None,
        Some(e) => Some(expression(e)?),
    };
    Ok(ReturnStmt { value, span })
}

fn defer_stmt(pair: Pair<'_, Rule>) -> R<DeferStmt> {
    let mut inner = pair.into_inner();
    let _kw = next(&mut inner, Rule::kw_defer);
    let body_pair = inner.next().expect("defer_stmt has a body");
    let body = match body_pair.as_rule() {
        Rule::block => DeferBody::Block(block(body_pair)?),
        Rule::expression => DeferBody::Expr(expression(body_pair)?),
        _ => unreachable!("defer body {:?}", body_pair.as_rule()),
    };
    Ok(DeferStmt { body })
}

fn for_stmt(pair: Pair<'_, Rule>) -> R<ForStmt> {
    let mut inner = pair.into_inner();
    let _kw_for = next(&mut inner, Rule::kw_for);
    let var_pair = next(&mut inner, Rule::identifier);
    let iter_var_span = span_of(&var_pair);
    let iter_var = var_pair.as_str().to_string();
    let _kw_in = next(&mut inner, Rule::kw_in);
    let iterable = expression(next(&mut inner, Rule::expression))?;
    let body = block(next(&mut inner, Rule::block))?;
    Ok(ForStmt {
        iter_var,
        iter_var_span,
        iterable,
        body,
    })
}

fn while_stmt(pair: Pair<'_, Rule>) -> R<WhileStmt> {
    let mut inner = pair.into_inner();
    let _kw = next(&mut inner, Rule::kw_while);
    let condition = expression(next(&mut inner, Rule::expression))?;
    let body = block(next(&mut inner, Rule::block))?;
    Ok(WhileStmt { condition, body })
}

pub(crate) fn statement(pair: Pair<'_, Rule>) -> R<Statement> {
    let stmt_span = span_of(&pair);
    let inner = single(pair);
    let has_semicolon = stmt_span.end > inner.as_span().end();
    let stmt = inner_statement(inner)?;
    match stmt {
        Statement::Expr(es) => Ok(Statement::Expr(ExprStmt {
            expr: es.expr,
            has_semicolon: es.has_semicolon || has_semicolon,
            span: if has_semicolon { stmt_span } else { es.span },
        })),
        other => Ok(other),
    }
}

/// Lower `AssignmentExpression`: a ternary expression, or a nested
/// assignment (`a = b = c` becomes `a = (b = c)`, spec `ast.md`,
/// "Statements").
pub(crate) fn assignment_value(pair: Pair<'_, Rule>) -> R<AssignValue> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let first = next(&mut inner, Rule::ternary_expression);
    let first_span = span_of(&first);
    let expr = ternary_entry(first)?;
    match inner.next() {
        None => Ok(AssignValue::Expr(expr)),
        Some(op) if op.as_rule() == Rule::assign_op => {
            let rhs = next(&mut inner, Rule::assignment_expression);
            let value = assignment_value(rhs)?;
            let target = assign_target(expr, first_span)?;
            Ok(AssignValue::Assign(Box::new(AssignStmt {
                target,
                value,
                span,
            })))
        }
        Some(other) => unreachable!("assignment_expression child {:?}", other.as_rule()),
    }
}

fn inner_statement(pair: Pair<'_, Rule>) -> R<Statement> {
    let pair = single(pair);
    match pair.as_rule() {
        Rule::let_decl => let_decl(pair).map(Statement::Let),
        Rule::const_decl => const_decl(pair).map(Statement::Let),
        Rule::fn_decl => fn_decl(pair).map(Statement::Fn),
        Rule::return_stmt => return_stmt(pair).map(Statement::Return),
        Rule::break_stmt => Ok(Statement::Break),
        Rule::continue_stmt => Ok(Statement::Continue),
        Rule::defer_stmt => defer_stmt(pair).map(Statement::Defer),
        Rule::for_stmt => for_stmt(pair).map(Statement::For),
        Rule::while_stmt => while_stmt(pair).map(Statement::While),
        Rule::expr_stmt_or_assignment => {
            let span = span_of(&pair);
            let mut inner = pair.into_inner();
            let lhs = next(&mut inner, Rule::ternary_expression);
            let lhs_span = span_of(&lhs);
            let expr = ternary_entry(lhs)?;
            match inner.next() {
                None => statement_from_expr(expr, span),
                Some(op) if op.as_rule() == Rule::assign_op => {
                    let rhs = next(&mut inner, Rule::assignment_expression);
                    let value = assignment_value(rhs)?;
                    let target = assign_target(expr, lhs_span)?;
                    Ok(Statement::Assign(AssignStmt {
                        target,
                        value,
                        span,
                    }))
                }
                Some(other) => unreachable!("expr_stmt_or_assignment child {:?}", other.as_rule()),
            }
        }
        _ => unreachable!("inner_statement {:?}", pair.as_rule()),
    }
}

/// Lower an expression in statement position.
pub(crate) fn statement_from_expr(expr: Expression, span: Range<usize>) -> R<Statement> {
    Ok(Statement::Expr(ExprStmt {
        expr,
        has_semicolon: false,
        span,
    }))
}

pub(crate) fn block(pair: Pair<'_, Rule>) -> R<Block> {
    let mut statements = Vec::new();
    for p in pair.into_inner() {
        check_rule(&p, Rule::statement)?;
        statements.push(statement(p)?);
    }
    Ok(Block { statements })
}

fn header_item(pair: Pair<'_, Rule>) -> R<Statement> {
    let inner = single(pair);
    match inner.as_rule() {
        Rule::mod_decl => mod_decl(inner),
        Rule::use_decl => use_decl(inner, false),
        Rule::pub_use => pub_use(inner),
        _ => unreachable!("header alternative {:?}", inner.as_rule()),
    }
}

fn mod_decl(pair: Pair<'_, Rule>) -> R<Statement> {
    let span = span_of(&pair);
    let mut is_pub = false;
    let mut name = None;
    let mut body = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::pub_marker => is_pub = true,
            Rule::kw_mod => {}
            Rule::identifier => name = Some(p.as_str().to_string()),
            Rule::module_body => {
                let mut stmts = Vec::new();
                for item in p.into_inner() {
                    match item.as_rule() {
                        Rule::header_item => stmts.push(header_item(item)?),
                        Rule::declaration => stmts.push(declaration(item)?),
                        _ => unreachable!("module_body child {:?}", item.as_rule()),
                    }
                }
                body = Some(stmts);
            }
            _ => unreachable!("mod_decl child {:?}", p.as_rule()),
        }
    }
    Ok(Statement::Mod(ModDecl {
        name: name.expect("mod declares a name"),
        is_pub,
        body,
        span,
    }))
}

fn use_decl(pair: Pair<'_, Rule>, is_pub: bool) -> R<Statement> {
    let span = span_of(&pair);
    let mut type_only = false;
    let mut target = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::kw_use | Rule::kw_pub => {}
            Rule::kw_type => type_only = true,
            Rule::use_target => target = Some(use_target(p)?),
            _ => unreachable!("use_decl child {:?}", p.as_rule()),
        }
    }
    Ok(Statement::Use(UseDecl {
        is_pub,
        type_only,
        target: target.expect("use declares a target"),
        span,
    }))
}

fn pub_use(pair: Pair<'_, Rule>) -> R<Statement> {
    let span = span_of(&pair);
    let mut target = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::kw_pub | Rule::kw_use => {}
            Rule::use_target => target = Some(use_target(p)?),
            Rule::pub_use_local => {
                let mut names = Vec::new();
                for n in p.into_inner() {
                    check_rule(&n, Rule::identifier)?;
                    names.push(n.as_str().to_string());
                }
                target = Some(UseTarget::Local(names));
            }
            _ => unreachable!("pub_use child {:?}", p.as_rule()),
        }
    }
    Ok(Statement::Use(UseDecl {
        is_pub: true,
        type_only: false,
        target: target.expect("pub use declares a target"),
        span,
    }))
}

fn use_target(pair: Pair<'_, Rule>) -> R<UseTarget> {
    let inner = single(pair);
    match inner.as_rule() {
        Rule::use_path => {
            let mut head = None;
            let mut segments = Vec::new();
            let mut alias = None;
            let mut after_as = false;
            for p in inner.into_inner() {
                match p.as_rule() {
                    Rule::path_head => head = Some(path_head(p)?),
                    Rule::kw_as => after_as = true,
                    Rule::identifier => {
                        let ident = p.as_str().to_string();
                        if after_as {
                            alias = Some(ident);
                            after_as = false;
                        } else {
                            segments.push(ident);
                        }
                    }
                    _ => unreachable!("use_path child {:?}", p.as_rule()),
                }
            }
            Ok(UseTarget::Path {
                head: head.expect("use_path has a head"),
                segments,
                alias,
            })
        }
        Rule::use_group => {
            let mut head = None;
            let mut entries = Vec::new();
            // The segments between the head and `{` (`use std::regex::{…}`)
            // are a path prefix shared by every entry.
            let mut prefix: Vec<String> = Vec::new();
            for p in inner.into_inner() {
                match p.as_rule() {
                    Rule::path_head => head = Some(path_head(p)?),
                    Rule::identifier => prefix.push(p.as_str().to_string()),
                    Rule::use_entry => {
                        let mut segments = Vec::new();
                        let mut alias = None;
                        let mut after_as = false;
                        for e in p.into_inner() {
                            match e.as_rule() {
                                Rule::relative_path => {
                                    segments = e.as_str().split("::").map(str::to_string).collect();
                                }
                                Rule::kw_as => after_as = true,
                                Rule::identifier => {
                                    if after_as {
                                        alias = Some(e.as_str().to_string());
                                        after_as = false;
                                    }
                                }
                                _ => unreachable!("use_entry child {:?}", e.as_rule()),
                            }
                        }
                        let mut full = prefix.clone();
                        full.append(&mut segments);
                        entries.push(UseEntry {
                            segments: full,
                            alias,
                        });
                    }
                    _ => unreachable!("use_group child {:?}", p.as_rule()),
                }
            }
            Ok(UseTarget::Group {
                head: head.expect("use_group has a head"),
                entries,
            })
        }
        Rule::use_glob => {
            let mut head = None;
            let mut segments = Vec::new();
            for p in inner.into_inner() {
                match p.as_rule() {
                    Rule::path_head => head = Some(path_head(p)?),
                    Rule::identifier => segments.push(p.as_str().to_string()),
                    _ => unreachable!("use_glob child {:?}", p.as_rule()),
                }
            }
            Ok(UseTarget::Glob {
                head: head.expect("use_glob has a head"),
                segments,
            })
        }
        _ => unreachable!("use_target alternative {:?}", inner.as_rule()),
    }
}

fn path_head(pair: Pair<'_, Rule>) -> R<PathHead> {
    let inner = single(pair);
    Ok(match inner.as_rule() {
        Rule::super_kw => PathHead::Super,
        Rule::kw_self => PathHead::SelfModule,
        Rule::package_head => PathHead::Package(inner.as_str().to_string()),
        Rule::identifier => PathHead::Name(inner.as_str().to_string()),
        _ => unreachable!("path_head alternative {:?}", inner.as_rule()),
    })
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
