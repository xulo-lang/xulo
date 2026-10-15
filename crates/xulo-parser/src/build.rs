use std::ops::Range;

use pest::iterators::Pair;

use xulo_core::ast::{
    AssignStmt, BindingPattern, Block, ComponentStmt, DeferBody, DeferStmt, EffectStmt, EnumDef,
    EnumPayloadParam, EnumVariant, EnvStmt, ExportItem, ExportStmt, ExprStmt, Expression, FnBound,
    FnDef, ForStmt, ImplDecl, ImportSpec, ImportStmt, LetBinding, Param, ParamMode, ReturnStmt,
    StateStmt, Statement, StoreStmt, StructDef, StructField, TraitDecl, TraitMethod, TypeAlias,
    UiElement, WhileStmt,
};
use xulo_core::error::XuloError;

use super::expr::{
    assign_target, component_call, decode_string, expression, ternary_entry,
};
use super::types::{check_rule, single, type_expr, R};
use super::{Rule, err, span_of};

pub fn program(root: Pair<'_, Rule>) -> Result<xulo_core::ast::Program, XuloError> {
    let mut statements = Vec::new();
    for pair in root.into_inner() {
        match pair.as_rule() {
            Rule::EOI => {}
            Rule::import_decl => statements.push(Statement::Import(import_decl(pair)?)),
            Rule::declaration => statements.push(declaration(pair)?),
            _ => unreachable!("program child {:?}", pair.as_rule()),
        }
    }
    Ok(xulo_core::ast::Program { statements })
}

fn declaration(pair: Pair<'_, Rule>) -> R<Statement> {
    let inner = single(pair);
    match inner.as_rule() {
        Rule::pub_use => pub_use(inner),
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
    for p in pair.into_inner() {
        match p.as_rule() {
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
    })
}

/// Build a `test [async] fn name() { … }` into the fixed-shape `FnDef` the
/// `Statement::Test` variant carries: no parameters, no return type, no
/// generics (the grammar already forbids them; the fields are filled empty).
fn test_decl(pair: Pair<'_, Rule>) -> R<FnDef> {
    let span = span_of(&pair);
    let mut is_async = false;
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut body = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::kw_test | Rule::kw_fn => {}
            Rule::kw_async => is_async = true,
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
        traits.push(type_expr(p)?.name());
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
        let name = inner.next().expect("where_item has a name").as_str().to_string();
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
    let mut fields = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::pub_marker | Rule::kw_struct => {}
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::generic_params => {
                type_params = generic_params(p)?.0;
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
        fields,
        span,
    })
}

fn enum_decl(pair: Pair<'_, Rule>) -> R<EnumDef> {
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_params = Vec::new();
    let mut variants = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::pub_marker | Rule::kw_enum => {}
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::generic_params => {
                type_params = generic_params(p)?.0;
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
        variants,
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
                let name = parts.next().expect("named_payload has a name").as_str().to_string();
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
    for p in pair.into_inner() {
        match p.as_rule() {
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
    let mut methods = Vec::new();
    let mut is_inherent = true;
    let mut seen_type = false;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::generic_params | Rule::kw_impl | Rule::kw_for => {}
            Rule::identifier if !seen_type => {
                trait_name = p.as_str().to_string();
                is_inherent = false;
            }
            Rule::type_expr => {
                type_name = type_expr(p)?.name();
                seen_type = true;
            }
            Rule::fn_decl => methods.push(fn_decl(p)?),
            _ => unreachable!("impl_decl child {:?}", p.as_rule()),
        }
    }
    Ok(ImplDecl {
        trait_name,
        type_name,
        methods,
        span,
        is_inherent,
    })
}

fn type_alias(pair: Pair<'_, Rule>) -> R<TypeAlias> {
    let mut name = String::new();
    let mut name_span = 0..0;
    let mut type_params = Vec::new();
    let mut type_ = None;
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::pub_marker | Rule::kw_type => {}
            Rule::identifier => {
                name = p.as_str().to_string();
                name_span = span_of(&p);
            }
            Rule::generic_params => {
                type_params = generic_params(p)?.0;
            }
            Rule::type_expr => type_ = Some(type_expr(p)?),
            _ => unreachable!("type_alias child {:?}", p.as_rule()),
        }
    }
    Ok(TypeAlias {
        name,
        name_span,
        type_params,
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
    let names: Vec<String> = id_list.into_inner().map(|p| p.as_str().to_string()).collect();
    let (tuple_names, object_destructuring) = if is_object {
        (
            None,
            Some(names.into_iter().map(|n| (n, None)).collect()),
        )
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

fn assignment(pair: Pair<'_, Rule>) -> R<Statement> {
    let span = span_of(&pair);
    let mut inner = pair.into_inner();
    let place = next(&mut inner, Rule::place);
    let target = assign_target(place)?;
    let _op = next(&mut inner, Rule::assign_op);
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
    Ok(Statement::Assign(AssignStmt {
        target,
        value,
        span,
    }))
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
        Rule::assignment => assignment(pair),
        Rule::expression_statement => {
            let span = span_of(&pair);
            let expr = ternary_entry(single(pair))?;
            statement_from_expr(expr, span)
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

fn import_decl(pair: Pair<'_, Rule>) -> R<ImportStmt> {
    let mut inner = pair.into_inner();
    let _kw = next(&mut inner, Rule::kw_import);
    let inner = inner.next().expect("import_decl has an alternative");
    match inner.as_rule() {
        Rule::type_import => {
            let (names, source) = named_import_parts(inner)?;
            Ok(ImportStmt {
                source,
                spec: ImportSpec::Named(names),
                type_only: true,
            })
        }
        Rule::named_import => {
            let (names, source) = named_import_parts(inner)?;
            Ok(ImportStmt {
                source,
                spec: ImportSpec::Named(names),
                type_only: false,
            })
        }
        Rule::namespace_import => {
            let mut ns = String::new();
            let mut source = String::new();
            for p in inner.into_inner() {
                match p.as_rule() {
                    Rule::kw_as | Rule::kw_from => {}
                    Rule::identifier => ns = p.as_str().to_string(),
                    Rule::string_literal => source = decode_string(p.as_str()),
                    _ => unreachable!("namespace_import child {:?}", p.as_rule()),
                }
            }
            Ok(ImportStmt {
                source,
                spec: ImportSpec::Namespace(ns),
                type_only: false,
            })
        }
        Rule::side_effect_import => {
            let lit = single(inner);
            check_rule(&lit, Rule::string_literal)?;
            Ok(ImportStmt {
                source: decode_string(lit.as_str()),
                spec: ImportSpec::Bare,
                type_only: false,
            })
        }
        _ => unreachable!("import alternative {:?}", inner.as_rule()),
    }
}

/// One imported name: the local binding, optionally renamed from the export.
type ImportName = (String, Option<String>);

fn named_import_parts(pair: Pair<'_, Rule>) -> R<(Vec<ImportName>, String)> {
    let mut names = Vec::new();
    let mut source = String::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::kw_type | Rule::kw_from => {}
            Rule::named_imports => {
                for entry in p.into_inner() {
                    check_rule(&entry, Rule::import_entry)?;
                    let mut name = None;
                    let mut alias = None;
                    for e in entry.into_inner() {
                        match e.as_rule() {
                            Rule::identifier => {
                                if name.is_none() {
                                    name = Some(e.as_str().to_string());
                                } else {
                                    alias = Some(e.as_str().to_string());
                                }
                            }
                            Rule::kw_as => {}
                            _ => unreachable!("import_entry child {:?}", e.as_rule()),
                        }
                    }
                    names.push((
                        name.expect("import_entry has a name"),
                        alias,
                    ));
                }
            }
            Rule::string_literal => source = decode_string(p.as_str()),
            _ => unreachable!("import child {:?}", p.as_rule()),
        }
    }
    Ok((names, source))
}

fn pub_use(pair: Pair<'_, Rule>) -> R<Statement> {
    let mut names = Vec::new();
    for p in pair.into_inner() {
        match p.as_rule() {
            Rule::identifier => names.push(p.as_str().to_string()),
            Rule::kw_pub | Rule::kw_use | Rule::kw_from | Rule::string_literal => {}
            _ => unreachable!("pub_use child {:?}", p.as_rule()),
        }
    }
    Ok(Statement::Export(ExportStmt {
        item: ExportItem::Names(names),
    }))
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
                else_branch: i
                    .else_branch
                    .as_ref()
                    .map(statements_to_ui)
                    .transpose()?,
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
        Statement::Import(_) => "import",
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
