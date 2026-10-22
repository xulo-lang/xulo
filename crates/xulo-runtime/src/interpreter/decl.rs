use super::*;

/// How a derived `toString` renders one field or payload parameter:
/// structurally, or by delegating to that type's `ToString` impl (hand
/// written or itself derived — both live under the same mangled name).
pub(super) enum DerivedForm {
    Structural,
    ToStr(String),
}

pub(super) fn derived_form(ty: &Type) -> DerivedForm {
    match ty {
        // `string` parses as `Type::String`, so a named type here is a
        // user type; the semantic pass has already rejected the derive
        // unless the type has a string form (E0212/E0801).
        Type::Named(name, _) => DerivedForm::ToStr(name.clone()),
        _ => DerivedForm::Structural,
    }
}

/// A struct constructor's declaration: field names in order, each field's
/// optional default expression, and the environment defaults evaluate in.
#[derive(Clone)]
pub(super) struct StructCtorDecl {
    pub(super) env: Rc<RefCell<Env>>,
    pub(super) fields: Vec<(String, Option<Expression>)>,
}

impl Interpreter {
    /// Register a `struct` declaration: the shared type object (inherent
    /// methods and a derived `toString` land on it), the constructor
    /// metadata `User(name: …)` builds instances from, and the name
    /// `struct_method`/dispatch look the object up by.
    pub(super) fn register_struct(&self, s: &StructDef, env: &Rc<RefCell<Env>>) {
        self.struct_names.borrow_mut().push(s.name.clone());
        env.borrow_mut().define(
            &s.name,
            Value::Object(Rc::new(RefCell::new(Vec::new()))),
            false,
        );
        self.struct_fields.borrow_mut().insert(
            s.name.clone(),
            StructCtorDecl {
                env: env.clone(),
                fields: s
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), f.default.clone()))
                    .collect(),
            },
        );
        self.register_derived_struct(s, env);
    }

    /// Whether a declaration carries `#Derive(ToString)` (spec
    /// `types/traits.md` registers exactly one derive target).
    pub(super) fn derives_to_string(attributes: &[Attribute]) -> bool {
        attributes
            .iter()
            .any(|attr| attr.name == "Derive" && attr.args.iter().any(|(arg, _)| arg == "ToString"))
    }

    /// Render one derived field/parameter. Nested named types resolve through
    /// the interpreter root, where every `impl` — hand written and derived —
    /// registers its mangled name, so derived renderers compose.
    pub(super) fn derived_to_string(
        &self,
        value: &Value,
        form: &DerivedForm,
    ) -> Result<String, RunError> {
        match form {
            DerivedForm::Structural => Ok(value.format()),
            DerivedForm::ToStr(type_name) => {
                let mangled = impl_fn_name("ToString", type_name, "toString");
                let func = self.global.borrow().get(&mangled);
                match func {
                    Some(func) => {
                        let out =
                            self.call_value_with_values(&func, std::slice::from_ref(value))?;
                        Ok(match out {
                            Value::String(s) => s.to_string(),
                            other => other.format(),
                        })
                    }
                    // No analysis pass registered an impl: fall back to
                    // structural rendering, matching an underived type.
                    None => Ok(value.format()),
                }
            }
        }
    }

    /// Register `#Derive(ToString)` for a struct (spec `types/traits.md`): the
    /// generated method sits on the struct's object so `user.toString()`
    /// resolves like a hand written member, and under its mangled name on the
    /// shared root so `ToString::toString(user)` and an annotated `str(user)`
    /// dispatch through the same function.
    pub(super) fn register_derived_struct(&self, s: &StructDef, env: &Rc<RefCell<Env>>) {
        if !Self::derives_to_string(&s.attributes) {
            return;
        }
        let type_name = s.name.clone();
        let forms: Vec<(String, DerivedForm)> = s
            .fields
            .iter()
            .map(|f| (f.name.clone(), derived_form(&f.type_)))
            .collect();
        let renderer: Value = Value::Native(Rc::new(move |interp, args| {
            let receiver = args.first().cloned().unwrap_or(Value::Null);
            let out = interp.render_derived_struct(&type_name, &forms, &receiver)?;
            Ok(Value::String(out.into()))
        }));
        if let Some(Value::Object(obj)) = env.borrow().get(&s.name) {
            let mut obj = obj.borrow_mut();
            if !obj.iter().any(|(k, _)| k == "toString") {
                obj.push(("toString".to_string(), renderer.clone()));
            }
            // The qualified key is what a generic `T: ToString` bound's
            // dynamic dispatch looks for on the receiver's struct object —
            // the same slot a hand-written trait impl registers.
            let qualified = "ToString::toString".to_string();
            if !obj.iter().any(|(k, _)| *k == qualified) {
                obj.push((qualified, renderer.clone()));
            }
        }
        self.global.borrow_mut().define(
            &impl_fn_name("ToString", &s.name, "toString"),
            renderer,
            false,
        );
    }

    /// Render a struct instance as `Type(field: value, …)` in declaration
    /// order (spec `types/traits.md` derive example).
    pub(super) fn render_derived_struct(
        &self,
        type_name: &str,
        forms: &[(String, DerivedForm)],
        receiver: &Value,
    ) -> Result<String, RunError> {
        let Value::Object(fields) = receiver else {
            return Ok(receiver.format());
        };
        let fields = fields.borrow();
        let mut parts = Vec::with_capacity(forms.len());
        for (name, form) in forms {
            let value = fields
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .unwrap_or(Value::Null);
            parts.push(format!("{name}: {}", self.derived_to_string(&value, form)?));
        }
        Ok(format!("{type_name}({})", parts.join(", ")))
    }

    /// Register `#Derive(ToString)` for an enum: enum instances carry no
    /// object, so the generated method exists only under its mangled name —
    /// `variant.to_string()` dispatches to it, as do `Trait::method` calls and
    /// annotated `str(v)`.
    pub(super) fn register_derived_enum(&self, e: &EnumDef) {
        if !Self::derives_to_string(&e.attributes) {
            return;
        }
        let type_name = e.name.clone();
        let variants: Vec<(String, Vec<DerivedForm>)> = e
            .variants
            .iter()
            .map(|v| {
                let forms = v
                    .payload
                    .as_deref()
                    .unwrap_or(&[])
                    .iter()
                    .map(|p| derived_form(&p.type_))
                    .collect();
                (v.name.clone(), forms)
            })
            .collect();
        let renderer: Value = Value::Native(Rc::new(move |interp, args| {
            let receiver = args.first().cloned().unwrap_or(Value::Null);
            let out = interp.render_derived_enum(&type_name, &variants, &receiver)?;
            Ok(Value::String(out.into()))
        }));
        self.global.borrow_mut().define(
            &impl_fn_name("ToString", &e.name, "toString"),
            renderer,
            false,
        );
    }

    /// Render an enum instance as `Enum::Variant` or `Enum::Variant(p, …)`,
    /// each payload slot through its declared type's string form.
    pub(super) fn render_derived_enum(
        &self,
        type_name: &str,
        variants: &[(String, Vec<DerivedForm>)],
        receiver: &Value,
    ) -> Result<String, RunError> {
        let Value::Enum { tag, payload, .. } = receiver else {
            return Ok(receiver.format());
        };
        let Some((_, forms)) = variants.iter().find(|(name, _)| name == tag) else {
            return Ok(receiver.format());
        };
        let slots: Vec<Value> = match payload {
            None => Vec::new(),
            // A single-slot variant stores its one value directly — even when
            // that value is itself a list.
            Some(p) if forms.len() == 1 => vec![(**p).clone()],
            Some(p) => match &**p {
                Value::List(items) => items.borrow().clone(),
                single => vec![single.clone()],
            },
        };
        if slots.len() != forms.len() {
            return Ok(receiver.format());
        }
        let mut parts = Vec::with_capacity(slots.len());
        for (value, form) in slots.iter().zip(forms) {
            parts.push(self.derived_to_string(value, form)?);
        }
        if parts.is_empty() {
            Ok(format!("{type_name}::{tag}"))
        } else {
            Ok(format!("{type_name}::{tag}({})", parts.join(", ")))
        }
    }

    pub(super) fn register_fn(&self, f: &FnDef, env: &Rc<RefCell<Env>>) {
        let func = Value::Function(Rc::new(FunctionValue {
            params: f.params.clone(),
            body: f.body.clone(),
            return_type: f.return_type.clone(),
            is_async: f.is_async,
            closure: env.clone(),
        }));
        env.borrow_mut().define(&f.name, func, false);
    }

    /// Register every method of an `impl` block under its mangled name
    /// (`impl_{Trait}_{Type}_{method}`), the same name `Trait::method` calls
    /// are annotated with. Registered up front like `fn` declarations, so
    /// dispatch calls are defined regardless of statement order. The name is
    /// defined on the shared root env — not the caller's module env — so a
    /// dependent module can dispatch to an `impl` declared in another module;
    /// the function value keeps `closure` as its own module's env so its body
    /// still resolves that module's locals.
    pub(super) fn register_impl(&self, imp: &ImplDecl, closure: &Rc<RefCell<Env>>) {
        for method in &imp.methods {
            let func = Value::Function(Rc::new(FunctionValue {
                params: method.params.clone(),
                body: method.body.clone(),
                return_type: method.return_type.clone(),
                is_async: method.is_async,
                closure: closure.clone(),
            }));
            if imp.is_inherent {
                // Inherent impl: add the method to the struct object in the
                // scope that registered the impl (module env under
                // `exec_module`, the root env under `run`). Inherent methods
                // take precedence over trait methods of the same name (spec
                // member resolution tries the inherent impl first), so
                // re-registering replaces an entry instead of appending
                // behind an earlier trait registration.
                if let Some(Value::Object(obj)) = closure.borrow().get(&imp.type_name) {
                    let mut obj = obj.borrow_mut();
                    if let Some(slot) = obj
                        .iter()
                        .position(|(k, _)| k == &method.name)
                        .map(|i| &mut obj[i])
                    {
                        slot.1 = func.clone();
                    } else {
                        obj.push((method.name.clone(), func.clone()));
                    }
                }
            } else {
                // Trait impl: register as standalone function with mangled name
                let name = impl_fn_name(&imp.trait_name, &imp.type_name, &method.name);
                self.global.borrow_mut().define(&name, func.clone(), false);
                // The struct object also carries the method: under its bare
                // name for member calls (`r.area()`), and under
                // `Trait::method` for run-time resolution of dispatch through
                // a generic parameter's bound. An inherent method of the same
                // name keeps the bare key (member resolution prefers it).
                if let Some(Value::Object(obj)) = closure.borrow().get(&imp.type_name) {
                    let mut obj = obj.borrow_mut();
                    if !obj.iter().any(|(k, _)| k == &method.name) {
                        obj.push((method.name.clone(), func.clone()));
                    }
                    obj.push((format!("{}::{}", imp.trait_name, method.name), func.clone()));
                }
            }
        }
    }
}
