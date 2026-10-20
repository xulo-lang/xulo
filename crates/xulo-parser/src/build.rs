use std::ops::Range;

use pest::iterators::Pair;

use xulo_core::ast::{
    AssignStmt, BindingPattern, Block, ComponentStmt, DeferBody, DeferStmt, EffectStmt, EnumDef,
    EnumPayloadParam, EnumVariant, EnvStmt, ExportItem, ExportStmt, ExprStmt, Expression, FnBound,
    FnDef, ForStmt, ImplDecl, LetBinding, ModDecl, Param, ParamMode, PathHead, ReturnStmt,
    StateStmt, Statement, StoreStmt, StructDef, StructField, TraitDecl, TraitMethod, Type,
    TypeAlias, UiElement, UseDecl, UseEntry, UseTarget, WhileStmt,
};
use xulo_core::error::XuloError;

use super::expr::{assign_target, component_call, decode_string, expression, ternary_entry};
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

/// Build a single `#Name` or `#Name(arg, …)` attribute.
fn attribute(pair: Pair<'_, Rule>) -> R<xulo_core::ast::Attribute> {
    let pair_span = span_of(&pair);
    // `into_inner` consumes the pair; keep the matched source for the
    // `#Name` / `#Name()` distinction below.
    let pair_text = pair.as_str().to_string();
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut args = Vec::new();
    let mut seen_name = false;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::identifier if !seen_name => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
                seen_name = true;
            }
            Rule::attribute_args => {
                for a in p.into_inner() {
                    check_rule(&a, Rule::attribute_arg)?;
                    let id = a
                        .into_inner()
                        .next()
                        .expect("attribute_arg has an identifier");
                    args.push((id.as_str().to_string(), span_of(&id)));
                }
            }
            _ => unreachable!("attribute child {:?}", p.as_rule()),
        }
    }
    // `#Name` vs `#Name()`: the written text decides. Reading the matched
    // source rather than comparing spans keeps pest's implicit-whitespace
    // span tail (a bare attribute's pair swallows the gap before the next
    // token) out of both the flag and the reported span — registry shape
    // checks read both forms.
    let has_parens = pair_text.contains('(');
    let span = if has_parens {
        pair_span
    } else {
        pair_span.start..name_span.end
    };
    Ok(xulo_core::ast::Attribute {
        name,
        name_span,
        args,
        span,
        has_parens,
    })
}

fn fn_decl(pair: Pair<'_, Rule>) -> R<FnDef> {
    let span = span_of(&pair);
    let mut is_async = false;
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_params = Vec::new();
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
                let (tp, b) = generic_params(p)?;
                type_params = tp;
                bounds.extend(b);
            }
            Rule::parameter_list => params = parameter_list(p)?,
            Rule::type_expr => return_type = Some(type_expr(p)?),
            Rule::where_clause => bounds.extend(where_clause(p)?),
            Rule::block => body = Some(block(p)?),
            _ => unreachable!("fn_decl child {:?}", p.as_rule()),
        }
    }
    Ok(FnDef {
        name,
        name_span,
        params,
        return_type,
        type_params,
        bounds,
        is_async,
        body: body.expect("fn_decl has a body"),
        span,
        attributes,
        title: None,
    })
}

/// Build a `#Test [ "(" [ StringLiteral ] ")" ] [async] fn name() { … }` into
/// the fixed-shape `FnDef` the `Statement::Test` variant carries: no
/// parameters, no return type, no generics (the grammar already forbids them;
/// the fields are filled empty) and `title` from the optional group.
fn test_decl(pair: Pair<'_, Rule>) -> R<FnDef> {
    let span = span_of(&pair);
    let mut is_async = false;
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut body = None;
    let mut title = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::kw_fn => {}
            Rule::kw_async => is_async = true,
            Rule::string_literal => title = Some(decode_string(p.as_str())),
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::block => body = Some(block(p)?),
            _ => unreachable!("test_decl child {:?}", p.as_rule()),
        }
    }
    Ok(FnDef {
        name,
        name_span,
        params: Vec::new(),
        return_type: None,
        type_params: Vec::new(),
        bounds: Vec::new(),
        is_async,
        body: body.expect("test_decl has a body"),
        span,
        attributes: Vec::new(),
        title,
    })
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

fn generic_params(pair: Pair<'_, Rule>) -> R<(Vec<String>, Vec<FnBound>)> {
    let mut params = Vec::new();
    let mut bounds = Vec::new();
    for p in pair.into_inner() {
        check_rule(&p, Rule::generic_param)?;
        let mut inner = p.into_inner();
        let name_pair = inner.next().expect("generic_param has a name");
        let name = name_pair.as_str().to_string();
        if let Some(tb) = inner.next() {
            check_rule(&tb, Rule::trait_bound)?;
            bounds.push(FnBound {
                param: name.clone(),
                traits: trait_bound(tb)?,
            });
        }
        params.push(name);
    }
    Ok((params, bounds))
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
                let (tp, bd) = generic_params(p)?;
                type_params = tp;
                type_bounds = bd;
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
                let (tp, bd) = generic_params(p)?;
                type_params = tp;
                type_bounds = bd;
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
                type_params = generic_params(p)?.0;
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
                let (params, bounds) = generic_params(p)?;
                type_params = params;
                type_bounds = bounds;
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
        type_bounds,
        type_args,
        methods,
        span,
        is_inherent,
        attributes,
    })
}

fn type_alias(pair: Pair<'_, Rule>) -> R<TypeAlias> {
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_params = Vec::new();
    let mut type_bounds = Vec::new();
    let mut type_ = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::pub_marker | Rule::kw_type => {}
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::generic_params => {
                let (tp, bd) = generic_params(p)?;
                type_params = tp;
                type_bounds = bd;
            }
            Rule::type_expr => type_ = Some(type_expr(p)?),
            _ => unreachable!("type_alias child {:?}", p.as_rule()),
        }
    }
    Ok(TypeAlias {
        name,
        name_span,
        type_params,
        type_bounds,
        type_: type_.expect("type_alias has a type"),
    })
}

fn let_decl(pair: Pair<'_, Rule>) -> R<LetBinding> {
    let mut binding = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::pub_marker | Rule::kw_let => {}
            Rule::mut_binding | Rule::simple_binding | Rule::destructuring_binding => {
                binding = Some(p);
            }
            _ => unreachable!("let_decl child {:?}", p.as_rule()),
        }
    }
    let binding = binding.expect("let_decl has a binding");
    match binding.as_rule() {
        Rule::mut_binding => mut_binding(binding),
        Rule::simple_binding => simple_binding(binding),
        Rule::destructuring_binding => destructuring_binding(binding),
        _ => unreachable!("let_decl alternative {:?}", binding.as_rule()),
    }
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
        memo: false,
        memo_deps: None,
        tuple_names: None,
        object_destructuring: None,
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
        memo: false,
        memo_deps: None,
        tuple_names: None,
        object_destructuring: None,
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
        memo: false,
        memo_deps: None,
        tuple_names,
        object_destructuring,
    })
}

fn const_decl(pair: Pair<'_, Rule>) -> R<LetBinding> {
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_annotation = None;
    let mut value = None;
    for p in pair.into_inner() {
        match p.as_rule() {
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
        memo: false,
        memo_deps: None,
        tuple_names: None,
        object_destructuring: None,
    })
}

fn component_decl(pair: Pair<'_, Rule>) -> R<Statement> {
    let inner = single(pair);
    match inner.as_rule() {
        Rule::state_decl => state_decl(inner),
        Rule::store_decl => store_decl(inner),
        Rule::effect_decl => effect_decl(inner),
        Rule::environment_decl => environment_decl(inner),
        _ => unreachable!("component_decl {:?}", inner.as_rule()),
    }
}

fn check_deco(pair: &Pair<'_, Rule>, expected: &str) -> R<()> {
    let deco = pair
        .clone()
        .into_inner()
        .find(|p| p.as_rule() == Rule::identifier)
        .expect("component decl has a decorator name");
    let name = deco.as_str();
    if name == expected {
        Ok(())
    } else {
        Err(err(
            format!(
                "unknown component declaration `@{name}`: expected `@State`, `@Store`, `@Effect`, or `@Environment`"
            ),
            span_of(&deco),
        ))
    }
}

fn state_decl(pair: Pair<'_, Rule>) -> R<Statement> {
    check_deco(&pair, "State")?;
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_annotation = None;
    let mut value = None;
    let mut seen_let = false;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::identifier if !seen_let => {
                // The decorator name (`State`), already validated.
            }
            Rule::kw_let => seen_let = true,
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::type_expr => type_annotation = Some(type_expr(p)?),
            Rule::expression => value = Some(expression(p)?),
            _ => unreachable!("state_decl child {:?}", p.as_rule()),
        }
    }
    let binding = LetBinding {
        name,
        name_span,
        type_annotation,
        value,
        is_const: false,
        is_mutable: false,
        is_shared: false,
        memo: false,
        memo_deps: None,
        tuple_names: None,
        object_destructuring: None,
    };
    Ok(Statement::State(StateStmt { binding }))
}

fn store_decl(pair: Pair<'_, Rule>) -> R<Statement> {
    check_deco(&pair, "Store")?;
    let mut names = Vec::new();
    let mut value = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::identifier => {}
            Rule::kw_let => {}
            Rule::identifier_list => {
                names = p.into_inner().map(|q| q.as_str().to_string()).collect();
            }
            Rule::expression => value = Some(expression(p)?),
            _ => unreachable!("store_decl child {:?}", p.as_rule()),
        }
    }
    Ok(Statement::Store(StoreStmt {
        pattern: BindingPattern::Destructure(names.into_iter().map(|n| (n, None)).collect()),
        value: value.expect("store_decl has a value"),
    }))
}

fn effect_decl(pair: Pair<'_, Rule>) -> R<Statement> {
    check_deco(&pair, "Effect")?;
    let deco_span = span_of(&pair);
    let mut exprs = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::identifier => {}
            Rule::expression => exprs.push(p),
            _ => unreachable!("effect_decl child {:?}", p.as_rule()),
        }
    }
    let mut exprs = exprs.into_iter();
    let closure_expr = expression(exprs.next().expect("effect_decl has a closure"))?;
    let Expression::FnExpr(closure) = closure_expr else {
        return Err(err("`@Effect` expects a function expression", deco_span));
    };
    let deps = exprs.next().map(expression).transpose()?;
    Ok(Statement::Effect(EffectStmt {
        closure: *closure,
        deps: deps.map(|d| vec![d]),
    }))
}

fn environment_decl(pair: Pair<'_, Rule>) -> R<Statement> {
    check_deco(&pair, "Environment")?;
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_ = None;
    let mut seen_let = false;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::identifier if !seen_let => {}
            Rule::kw_let => seen_let = true,
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::type_expr => type_ = Some(type_expr(p)?),
            _ => unreachable!("environment_decl child {:?}", p.as_rule()),
        }
    }
    Ok(Statement::Environment(EnvStmt {
        name,
        name_span,
        type_: type_.expect("environment_decl has a type"),
    }))
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

fn inner_statement(pair: Pair<'_, Rule>) -> R<Statement> {
    let pair = single(pair);
    match pair.as_rule() {
        Rule::component_decl => component_decl(pair),
        Rule::let_decl => let_decl(pair).map(Statement::Let),
        Rule::const_decl => const_decl(pair).map(Statement::Let),
        Rule::fn_decl => fn_decl(pair).map(Statement::Fn),
        Rule::return_stmt => return_stmt(pair).map(Statement::Return),
        Rule::break_stmt => Ok(Statement::Break),
        Rule::continue_stmt => Ok(Statement::Continue),
        Rule::defer_stmt => defer_stmt(pair).map(Statement::Defer),
        Rule::for_stmt => for_stmt(pair).map(Statement::For),
        Rule::while_stmt => while_stmt(pair).map(Statement::While),
        Rule::component_call => {
            let span = span_of(&pair);
            let expr = component_call(pair)?;
            statement_from_expr(expr, span)
        }
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
                    let mut parts = rhs.into_inner();
                    let first = next(&mut parts, Rule::ternary_expression);
                    if parts.next().is_some() {
                        return Err(err(
                            "chained assignment is not supported; split `a = b = c` into two statements",
                            span,
                        ));
                    }
                    let value = ternary_entry(first)?;
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

/// Lower an expression in statement position: a component invocation becomes
/// `Statement::Component`, and an uppercase plain call — `Text("Hi")` — reads
/// the same (spec: components/view-syntax.md).
pub(crate) fn statement_from_expr(expr: Expression, span: Range<usize>) -> R<Statement> {
    match expr {
        Expression::Component(c) => Ok(Statement::Component(c)),
        other => {
            if is_upper_plain_call(&other) {
                Ok(Statement::Component(component_from_call(other)))
            } else {
                Ok(Statement::Expr(ExprStmt {
                    expr: other,
                    has_semicolon: false,
                    span,
                }))
            }
        }
    }
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

// ============================================================
// UI lowering
// ============================================================

pub(crate) fn to_ui_elements(component_block: Pair<'_, Rule>) -> R<Vec<UiElement>> {
    let mut out = Vec::new();
    for item in component_block.into_inner() {
        check_rule(&item, Rule::child_item)?;
        let item = single(item);
        match item.as_rule() {
            Rule::for_stmt => out.push(ui_for(for_stmt(item)?)?),
            Rule::component_call => {
                let expr = component_call(item)?;
                out.push(to_ui_element(expr)?);
            }
            Rule::expression => out.push(to_ui_element(expression(item)?)?),
            _ => unreachable!("child_item {:?}", item.as_rule()),
        }
    }
    Ok(out)
}

fn ui_for(f: ForStmt) -> R<UiElement> {
    Ok(UiElement::For {
        iter_var: f.iter_var,
        iter_var_span: f.iter_var_span,
        iterable: f.iterable,
        body: statements_to_ui(&f.body)?,
    })
}

fn statements_to_ui(block: &Block) -> R<Vec<UiElement>> {
    block.statements.iter().map(statement_to_ui).collect()
}

fn statement_to_ui(stmt: &Statement) -> R<UiElement> {
    match stmt {
        Statement::Component(c) => Ok(UiElement::Component(c.clone())),
        Statement::For(f) => Ok(UiElement::For {
            iter_var: f.iter_var.clone(),
            iter_var_span: f.iter_var_span.clone(),
            iterable: f.iterable.clone(),
            body: statements_to_ui(&f.body)?,
        }),
        Statement::Expr(es) => match &es.expr {
            Expression::Literal {
                value: xulo_core::ast::Literal::String(s),
                ..
            } => Ok(UiElement::Text(s.clone())),
            Expression::Component(c) => Ok(UiElement::Component(c.clone())),
            Expression::If(i) => Ok(UiElement::If {
                condition: i.condition.clone(),
                then_branch: statements_to_ui(&i.then_branch)?,
                else_branch: i.else_branch.as_ref().map(statements_to_ui).transpose()?,
            }),
            e if is_upper_plain_call(e) => Ok(UiElement::Component(component_from_call(e.clone()))),
            e => Ok(UiElement::Expr(e.clone())),
        },
        other => Err(err(
            format!("`{}` is not allowed in a UI block", stmt_kind(other)),
            0..0,
        )),
    }
}

fn stmt_kind(stmt: &Statement) -> &'static str {
    match stmt {
        Statement::Fn(_) => "fn declaration",
        Statement::Test(_) => "test declaration",
        Statement::Let(_) => "let binding",
        Statement::Return(_) => "return",
        Statement::Assign(_) => "assignment",
        Statement::TypeAlias(_) => "type alias",
        Statement::Enum(_) => "enum declaration",
        Statement::Struct(_) => "struct declaration",
        Statement::Block(_) => "block",
        Statement::Try(_) => "try",
        Statement::Throw(_) => "throw",
        Statement::Mod(_) => "module declaration",
        Statement::Use(_) => "use declaration",
        Statement::Export(_) => "pub declaration",
        Statement::State(_) => "@State",
        Statement::Store(_) => "@Store",
        Statement::Effect(_) => "@Effect",
        Statement::Environment(_) => "@Environment",
        Statement::Trait(_) => "trait declaration",
        Statement::Impl(_) => "impl block",
        Statement::Break => "break",
        Statement::Continue => "continue",
        Statement::Defer(_) => "defer",
        Statement::Component(_) | Statement::Expr(_) | Statement::For(_) | Statement::While(_) => {
            unreachable!("allowed in UI blocks")
        }
    }
}

fn to_ui_element(expr: Expression) -> R<UiElement> {
    match &expr {
        Expression::Component(c) => Ok(UiElement::Component(c.clone())),
        Expression::Literal {
            value: xulo_core::ast::Literal::String(s),
            ..
        } => Ok(UiElement::Text(s.clone())),
        Expression::If(i) => Ok(UiElement::If {
            condition: i.condition.clone(),
            then_branch: statements_to_ui(&i.then_branch)?,
            else_branch: i.else_branch.as_ref().map(statements_to_ui).transpose()?,
        }),
        _ if is_upper_plain_call(&expr) => Ok(UiElement::Component(component_from_call(expr))),
        _ => Ok(UiElement::Expr(expr)),
    }
}

/// True for a plain call whose callee is a single uppercase identifier — the
/// `Text("Hi")` invocation form that carries no children block.
fn is_upper_plain_call(expr: &Expression) -> bool {
    match expr {
        Expression::Call(c) => {
            c.object.is_none()
                && !c.callee.contains("::")
                && c.callee
                    .chars()
                    .next()
                    .is_some_and(|ch| ch.is_ascii_uppercase())
        }
        _ => false,
    }
}

fn component_from_call(expr: Expression) -> ComponentStmt {
    let Expression::Call(c) = expr else {
        unreachable!("component_from_call requires a Call");
    };
    ComponentStmt {
        name: c.callee.clone(),
        name_span: c.callee_span.clone().unwrap_or_else(|| c.span.clone()),
        args: c.arguments.clone(),
        children: Vec::new(),
        span: c.span.clone(),
        type_args: c.type_args.clone(),
    }
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
