use super::*;

/// Whether a pattern literal (including a `"Enum.Variant"` string) matches a
/// value, mirroring the JS codegen: `__m === "Enum.Variant"` or
/// `__m.tag === "Variant"`.
pub(super) fn literal_matches(lit: &Literal, value: &Value) -> bool {
    match lit {
        Literal::String(s) => match value {
            Value::Enum { enum_name, tag, .. } => format!("{enum_name}.{tag}") == *s || *tag == *s,
            Value::String(v) => v.as_ref() == s.as_str(),
            _ => false,
        },
        Literal::Int(n) => value
            .num()
            .and_then(|v| v.integer())
            .is_some_and(|v| v == *n),
        Literal::Float(n) => value.num().is_some_and(|v| v.to_f64() == *n),
        Literal::Boolean(b) => matches!(value, Value::Boolean(v) if v == b),
        Literal::Null => matches!(value, Value::Null),
        Literal::List(_) | Literal::Object(_) => false,
    }
}

/// Convert a list/string index value into a `usize`, rejecting fractional,
/// negative, and NaN indices. Silently truncating (`-1.0 as usize` is 0 in
/// Rust) used to read the wrong element — the JS path yields `undefined` for
/// those, so the native runtime must not guess either.
pub(super) fn list_index(n: Num, span: &std::ops::Range<usize>) -> Result<usize, RunError> {
    match n.integer() {
        Some(i) if i >= 0 && i <= usize::MAX as i128 => Ok(i as usize),
        _ => Err(RunError::err(
            format!(
                "index must be a non-negative integer, found {}",
                Value::from_num(n).format()
            ),
            span.clone(),
        )),
    }
}

/// Whether an enum-pattern matches: the JS codegen accepts both a
/// `"Enum.Variant"` string and any value whose tag is the variant.
pub(super) fn enum_ref_matches(enum_name: &str, variant: &str, value: &Value) -> bool {
    match value {
        Value::Enum { tag, .. } => tag == variant,
        Value::String(s) => {
            let full = format!("{enum_name}.{variant}");
            s.as_ref() == full.as_str() || s.as_ref() == variant
        }
        _ => false,
    }
}

/// Bind a matched enum payload into `env`: a single binding receives the value
/// itself; several bindings receive the elements of a list payload.
pub(super) fn bind_enum_payload(
    env: &Rc<RefCell<Env>>,
    bindings: &[String],
    payload: &Value,
    span: &std::ops::Range<usize>,
) -> Result<(), RunError> {
    if bindings.len() == 1 {
        let name = &bindings[0];
        if name != "_" {
            env.borrow_mut().define(name, payload.clone(), false);
        }
        return Ok(());
    }
    let Value::List(list) = payload else {
        return Err(RunError::err(
            "expected a list payload for this enum variant",
            span.clone(),
        ));
    };
    let list = list.borrow();
    for (i, name) in bindings.iter().enumerate() {
        if name == "_" {
            continue;
        }
        let value = list.get(i).cloned().unwrap_or(Value::Null);
        env.borrow_mut().define(name, value, false);
    }
    Ok(())
}

/// Match a scrutinee against a pattern, binding names into `env` on success.
pub(super) fn pattern_matches(
    pattern: &xulo_core::ast::MatchPattern,
    scrutinee: &Value,
    env: &Rc<RefCell<Env>>,
) -> Result<bool, RunError> {
    Ok(match pattern {
        xulo_core::ast::MatchPattern::Wildcard => true,
        xulo_core::ast::MatchPattern::Literal(lit) => literal_matches(lit, scrutinee),
        xulo_core::ast::MatchPattern::Enum(r) => {
            enum_ref_matches(&r.enum_name, &r.variant, scrutinee)
        }
        xulo_core::ast::MatchPattern::EnumPayload {
            variant,
            bindings,
            span,
            ..
        } => {
            if let Value::Enum { tag, payload, .. } = scrutinee
                && tag == variant
                && let Some(payload) = payload
            {
                bind_enum_payload(env, bindings, payload, span)?;
                true
            } else {
                false
            }
        }
        xulo_core::ast::MatchPattern::Range {
            start,
            end,
            end_inclusive,
            ..
        } => match scrutinee.num() {
            Some(n) => {
                let n = n.to_f64();
                if *end_inclusive {
                    n >= *start && n <= *end
                } else {
                    n >= *start && n < *end
                }
            }
            None => match scrutinee {
                Value::String(s) => {
                    let lo = s.as_ref() >= start.to_string().as_str();
                    let hi = if *end_inclusive {
                        s.as_ref() <= end.to_string().as_str()
                    } else {
                        s.as_ref() < end.to_string().as_str()
                    };
                    lo && hi
                }
                _ => false,
            },
        },
        xulo_core::ast::MatchPattern::Binding { name, .. } => {
            env.borrow_mut().define(name, scrutinee.clone(), true);
            true
        }
        xulo_core::ast::MatchPattern::TypePattern {
            type_name,
            binder,
            optional,
            ..
        } => {
            let ok = value_is_named_type(scrutinee, type_name)
                || (*optional && matches!(scrutinee, Value::Null));
            if ok && let xulo_core::ast::PatternBinder::Binding(b) = binder {
                env.borrow_mut().define(b, scrutinee.clone(), true);
            }
            ok
        }
        xulo_core::ast::MatchPattern::Or { patterns, .. } => {
            for p in patterns {
                if pattern_matches(p, scrutinee, env)? {
                    return Ok(true);
                }
            }
            false
        }
        xulo_core::ast::MatchPattern::Struct { name, bindings, .. } => {
            if let Value::Enum {
                enum_name, payload, ..
            } = scrutinee
                && enum_name == name
            {
                bind_struct_payload(env, bindings, payload.as_deref());
                true
            } else {
                false
            }
        }
    })
}

/// Bind a struct's positional payload values to the pattern's binding names.
pub(super) fn bind_struct_payload(
    env: &Rc<RefCell<Env>>,
    bindings: &[xulo_core::ast::MatchPattern],
    payload: Option<&Value>,
) {
    let values: Vec<Value> = match payload {
        Some(Value::List(list)) => list.borrow().clone(),
        Some(v) => vec![v.clone()],
        None => Vec::new(),
    };
    for (i, binder) in bindings.iter().enumerate() {
        if let xulo_core::ast::MatchPattern::Binding { name, .. } = binder {
            let value = values.get(i).cloned().unwrap_or(Value::Null);
            env.borrow_mut().define(name, value, true);
        }
    }
}

/// Coerce a full [`Type`] to a runtime test against a value.
pub(super) fn type_matches_value(type_: &Type, value: &Value) -> bool {
    match type_ {
        Type::String => matches!(value, Value::String(_)),
        Type::Number => value.num().is_some(),
        Type::Boolean => matches!(value, Value::Boolean(_)),
        Type::Null => matches!(value, Value::Null),
        Type::List(_) => matches!(value, Value::List(_)),
        Type::Object | Type::ObjectType(_) => matches!(value, Value::Object(_)),
        Type::Named(name, _) => value_is_named_type(value, name),
        Type::Optional(inner) => matches!(value, Value::Null) || type_matches_value(inner, value),
        Type::Union(parts) => parts.iter().any(|p| type_matches_value(p, value)),
        Type::Intersection(parts) => parts.iter().all(|p| type_matches_value(p, value)),
        Type::Literal(s) => match value {
            Value::String(v) => v.as_ref() == s.as_str(),
            _ => false,
        },
        Type::Tuple(_) => matches!(value, Value::List(_)),
        Type::FnSig { .. } => matches!(value, Value::Function(_)),
        Type::Async(_) => matches!(value, Value::Promise(_)),
        Type::Any => true,
    }
}

/// A coarse runtime type test for `value is Type` / type patterns.
pub(super) fn value_is_named_type(value: &Value, type_name: &str) -> bool {
    match type_name {
        // The spec primitive spellings (spec `types/primitive-types.md`),
        // including the fixed-bit layer of the numeric system. Fixed-bit
        // widths are not tracked at runtime, so a width test checks the
        // value's range — the closest available approximation.
        "number" => value.num().is_some(),
        "int" | "i64" | "isize" => matches!(value, Value::Int(_)),
        "i8" => matches!(value, Value::Int(n) if i8::try_from(*n).is_ok()),
        "i16" => matches!(value, Value::Int(n) if i16::try_from(*n).is_ok()),
        "i32" => matches!(value, Value::Int(n) if i32::try_from(*n).is_ok()),
        "u64" | "usize" => {
            matches!(value, Value::UInt(_)) || matches!(value, Value::Int(n) if *n >= 0)
        }
        "u8" => matches!(value, Value::Int(n) if u8::try_from(*n).is_ok()),
        "u16" => matches!(value, Value::Int(n) if u16::try_from(*n).is_ok()),
        "u32" => matches!(value, Value::Int(n) if u32::try_from(*n).is_ok()),
        "float" | "f32" | "f64" => matches!(value, Value::Float(_)),
        "string" => matches!(value, Value::String(_)),
        "bool" => matches!(value, Value::Boolean(_)),
        "null" => matches!(value, Value::Null),
        "List" => matches!(value, Value::List(_)),
        "Map" => matches!(value, Value::Object(_)),
        "unknown" => true,
        _ => match value {
            Value::Enum { enum_name, .. } => enum_name == type_name,
            _ => false,
        },
    }
}

/// Deep-copy mutable containers so `copy x` detaches from the original
/// (spec `memory-and-runtime.md`, "Ownership with `copy`"). Lists, maps, sets,
/// structs, enums, tuples, and builders are duplicated; strings are immutable
/// and identity-carrying values keep their identity. The visited map mirrors a
/// cycle into the copy instead of recursing forever.
pub(super) fn deep_copy_value(value: &Value) -> Value {
    deep_copy_value_inner(value, &mut std::collections::HashMap::new())
}

fn deep_copy_value_inner(
    value: &Value,
    seen: &mut std::collections::HashMap<usize, Value>,
) -> Value {
    match value {
        Value::List(list) => {
            let key = Rc::as_ptr(list) as usize;
            if let Some(copy) = seen.get(&key) {
                return copy.clone();
            }
            let copy = Value::List(Rc::new(RefCell::new(Vec::new())));
            seen.insert(key, copy.clone());
            let items: Vec<Value> = list
                .borrow()
                .iter()
                .map(|item| deep_copy_value_inner(item, seen))
                .collect();
            if let Value::List(out) = &copy {
                *out.borrow_mut() = items;
            }
            copy
        }
        Value::Set(set) => {
            let key = Rc::as_ptr(set) as usize;
            if let Some(copy) = seen.get(&key) {
                return copy.clone();
            }
            let copy = Value::Set(Rc::new(RefCell::new(Vec::new())));
            seen.insert(key, copy.clone());
            let items: Vec<Value> = set
                .borrow()
                .iter()
                .map(|item| deep_copy_value_inner(item, seen))
                .collect();
            if let Value::Set(out) = &copy {
                *out.borrow_mut() = items;
            }
            copy
        }
        Value::Object(fields) => {
            let key = Rc::as_ptr(fields) as usize;
            if let Some(copy) = seen.get(&key) {
                return copy.clone();
            }
            let copy = Value::Object(Rc::new(RefCell::new(Vec::new())));
            seen.insert(key, copy.clone());
            let items: Vec<(String, Value)> = fields
                .borrow()
                .iter()
                .map(|(k, v)| (k.clone(), deep_copy_value_inner(v, seen)))
                .collect();
            if let Value::Object(out) = &copy {
                *out.borrow_mut() = items;
            }
            copy
        }
        Value::Enum {
            enum_name,
            tag,
            payload,
        } => Value::Enum {
            enum_name: enum_name.clone(),
            tag: tag.clone(),
            payload: payload
                .as_ref()
                .map(|inner| Box::new(deep_copy_value_inner(inner, seen))),
        },
        Value::StringBuilder(builder) => {
            Value::StringBuilder(Rc::new(RefCell::new(builder.borrow().clone())))
        }
        other => other.clone(),
    }
}
