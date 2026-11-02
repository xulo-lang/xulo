use super::*;

/// Which rendering a caller asks for: the output table's user-type consult.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum RenderMode {
    /// `str`/`${…}`/`List.join`: a user type renders through `Display`.
    DisplayOnly,
    /// A `#Derive(Debug)` field: a user type renders through `Debug`, an
    /// enum without one falls back to the table's enum row.
    DebugField,
    /// `print`: a user type renders through `Display`, then `Debug`.
    Print,
}

/// A struct constructor's declaration: field names in order, each field's
/// optional default expression, and the environment defaults evaluate in.
#[derive(Clone)]
pub(super) struct StructCtorDecl {
    pub(super) env: Rc<RefCell<Env>>,
    pub(super) fields: Vec<(String, Option<Expression>)>,
}

impl Interpreter {
    /// The default `Format` every consumer hands a `display`/`debug`:
    /// `Format(precision: null, alternate: false)` (spec
    /// `types/traits.md`, "Built-in formatting traits").
    pub(super) fn default_format(&self) -> Value {
        let obj = Rc::new(RefCell::new(vec![
            ("precision".to_string(), Value::Null),
            ("alternate".to_string(), Value::Boolean(false)),
        ]));
        self.register_instance(&obj, "Format");
        Value::Object(obj)
    }

    /// Call a registered impl for `value`, when one exists. `extra` follows
    /// the receiver (a `Format` for `Display`/`Debug`).
    pub(super) fn call_trait_impl(
        &self,
        trait_name: &str,
        method: &str,
        value: &Value,
        extra: &[Value],
    ) -> Option<Result<Value, RunError>> {
        let type_name = match value {
            Value::Enum { enum_name, .. } => Some(enum_name.clone()),
            Value::Object(obj) => self.instance_type(obj),
            _ => None,
        }?;
        let mangled = impl_fn_name(trait_name, &type_name, method);
        let func = self.global.borrow().get(&mangled)?;
        let mut args = Vec::with_capacity(extra.len() + 1);
        args.push(value.clone());
        args.extend(extra.iter().cloned());
        Some(self.call_value_with_values(&func, &args))
    }

    fn impl_string(&self, out: Result<Value, RunError>) -> Result<String, RunError> {
        Ok(match out? {
            Value::String(s) => s.to_string(),
            other => other.format(),
        })
    }

    /// Render a value under one of the three user-type consult modes. The
    /// recursion reaches structural positions (a container's elements, a
    /// tuple's elements, an object's fields), so a user type inside a
    /// container is consulted exactly as at the top.
    pub(super) fn render_in_mode(
        &self,
        value: &Value,
        mode: RenderMode,
        seen: &mut std::collections::HashSet<usize>,
        format: &Value,
    ) -> Result<String, RunError> {
        let render = |v: &Value, seen: &mut std::collections::HashSet<usize>| {
            self.render_in_mode(v, mode, seen, format)
        };
        match value {
            Value::Object(fields) => {
                if let Some(type_name) = self.instance_type(fields) {
                    if type_name == "Format" {
                        let fields = fields.borrow();
                        let get = |name: &str| {
                            fields
                                .iter()
                                .find(|(k, _)| k == name)
                                .map(|(_, v)| v.clone())
                                .unwrap_or(Value::Null)
                        };
                        let precision = render(&get("precision"), seen)?;
                        let alternate = render(&get("alternate"), seen)?;
                        return Ok(format!(
                            "Format(precision: {precision}, alternate: {alternate})"
                        ));
                    }
                    if type_name == "Error" {
                        let fields = fields.borrow();
                        let message = fields
                            .iter()
                            .find(|(k, _)| k == "message")
                            .map(|(_, v)| v.clone())
                            .unwrap_or(Value::Null);
                        return Ok(format!("Error({})", render(&message, seen)?));
                    }
                    if mode != RenderMode::DebugField
                        && let Some(out) = self.call_trait_impl(
                            "Display",
                            "display",
                            value,
                            std::slice::from_ref(&self.default_format()),
                        )
                    {
                        return self.impl_string(out);
                    }
                    if mode != RenderMode::DisplayOnly
                        && let Some(out) =
                            self.call_trait_impl("Debug", "debug", value, std::slice::from_ref(format))
                    {
                        return self.impl_string(out);
                    }
                }
                let id = Rc::as_ptr(fields) as usize;
                if !seen.insert(id) {
                    return Ok("{ <cycle> }".into());
                }
                let parts = fields
                    .borrow()
                    .iter()
                    .map(|(k, v)| Ok(format!("{k}: {}", render(v, seen)?)))
                    .collect::<Result<Vec<_>, RunError>>()?;
                seen.remove(&id);
                Ok(format!("{{ {} }}", parts.join(", ")))
            }
            Value::Enum {
                enum_name,
                tag,
                payload,
            } => {
                if mode != RenderMode::DebugField
                    && let Some(out) = self.call_trait_impl(
                        "Display",
                        "display",
                        value,
                        std::slice::from_ref(&self.default_format()),
                    )
                {
                    return self.impl_string(out);
                }
                if mode != RenderMode::DisplayOnly
                    && let Some(out) =
                        self.call_trait_impl("Debug", "debug", value, std::slice::from_ref(format))
                {
                    return self.impl_string(out);
                }
                let sep = if mode == RenderMode::DebugField {
                    "::"
                } else {
                    "."
                };
                match payload {
                    None => Ok(format!("{enum_name}{sep}{tag}")),
                    Some(p) => Ok(format!("{enum_name}{sep}{tag}({})", render(p, seen)?)),
                }
            }
            Value::List(list) => {
                let id = Rc::as_ptr(list) as usize;
                if !seen.insert(id) {
                    return Ok("[<cycle>]".into());
                }
                let parts = list
                    .borrow()
                    .iter()
                    .map(|v| render(v, seen))
                    .collect::<Result<Vec<_>, RunError>>()?;
                seen.remove(&id);
                Ok(format!("[{}]", parts.join(", ")))
            }
            Value::Set(set) => {
                let id = Rc::as_ptr(set) as usize;
                if !seen.insert(id) {
                    return Ok("[<cycle>]".into());
                }
                let parts = set
                    .borrow()
                    .iter()
                    .map(|v| render(v, seen))
                    .collect::<Result<Vec<_>, RunError>>()?;
                seen.remove(&id);
                Ok(format!("Set([{}])", parts.join(", ")))
            }
            Value::Signal(cell) => render(&cell.borrow(), seen),
            other => Ok(other.format()),
        }
    }

    /// `print`'s rendering: `Display`, then `Debug`, then the output table
    /// (spec `builtins/intrinsic-functions.md`, "Output").
    pub(super) fn render_print(&self, value: &Value) -> Result<String, RunError> {
        self.render_in_mode(
            value,
            RenderMode::Print,
            &mut std::collections::HashSet::new(),
            &self.default_format(),
        )
    }

    /// `str`/`${…}`/`List.join`'s rendering: `Display`, then the table.
    pub(super) fn render_display(&self, value: &Value) -> Result<String, RunError> {
        self.render_in_mode(
            value,
            RenderMode::DisplayOnly,
            &mut std::collections::HashSet::new(),
            &self.default_format(),
        )
    }

    /// Register a `struct` declaration: the shared type object (inherent
    /// methods and a derived `debug` land on it), the constructor metadata
    /// `User(name: …)` builds instances from, and the name
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

    /// Whether a declaration carries `#Derive(Debug)` (spec
    /// `types/traits.md` registers exactly one derive target).
    pub(super) fn derives_debug(attributes: &[Attribute]) -> bool {
        attributes
            .iter()
            .any(|attr| attr.name == "Derive" && attr.args.iter().any(|(arg, _)| arg == "Debug"))
    }

    /// Render one derived field/parameter through `Debug` rules: a user
    /// type uses its own `debug`, an enum without one falls back to the
    /// table's enum row, and everything else renders directly.
    pub(super) fn derived_debug(
        &self,
        value: &Value,
        format: &Value,
    ) -> Result<String, RunError> {
        self.render_in_mode(
            value,
            RenderMode::DebugField,
            &mut std::collections::HashSet::new(),
            format,
        )
    }

    /// Register `#Derive(Debug)` for a struct (spec `types/traits.md`): the
    /// generated method sits on the struct's object so `user.debug(fmt)`
    /// resolves like a hand written member, and under its mangled name on the
    /// shared root so `Debug::debug(user, fmt)` dispatches through the same
    /// function.
    pub(super) fn register_derived_struct(&self, s: &StructDef, env: &Rc<RefCell<Env>>) {
        if !Self::derives_debug(&s.attributes) {
            return;
        }
        let type_name = s.name.clone();
        let fields: Vec<String> = s.fields.iter().map(|f| f.name.clone()).collect();
        let renderer: Value = Value::Native(Rc::new(move |interp, args| {
            let receiver = args.first().cloned().unwrap_or(Value::Null);
            let format = args.get(1).cloned().unwrap_or_else(|| interp.default_format());
            let out = interp.render_derived_struct(&type_name, &fields, &receiver, &format)?;
            Ok(Value::String(out.into()))
        }));
        if let Some(Value::Object(obj)) = env.borrow().get(&s.name) {
            let mut obj = obj.borrow_mut();
            if !obj.iter().any(|(k, _)| k == "debug") {
                obj.push(("debug".to_string(), renderer.clone()));
            }
            // The qualified key is what a generic `T: Debug` bound's
            // dynamic dispatch looks for on the receiver's struct object —
            // the same slot a hand-written trait impl registers.
            let qualified = "Debug::debug".to_string();
            if !obj.iter().any(|(k, _)| *k == qualified) {
                obj.push((qualified, renderer.clone()));
            }
        }
        self.global.borrow_mut().define(
            &impl_fn_name("Debug", &s.name, "debug"),
            renderer,
            false,
        );
    }

    /// Render a struct instance as `Type(field: value, …)` in declaration
    /// order (spec `types/traits.md` derive example).
    pub(super) fn render_derived_struct(
        &self,
        type_name: &str,
        fields: &[String],
        receiver: &Value,
        format: &Value,
    ) -> Result<String, RunError> {
        let Value::Object(instance) = receiver else {
            return Ok(receiver.format());
        };
        let instance = instance.borrow();
        let mut parts = Vec::with_capacity(fields.len());
        for name in fields {
            let value = instance
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
                .unwrap_or(Value::Null);
            parts.push(format!("{name}: {}", self.derived_debug(&value, format)?));
        }
        Ok(format!("{type_name}({})", parts.join(", ")))
    }

    /// Register `#Derive(Debug)` for an enum: enum instances carry no
    /// object, so the generated method exists only under its mangled name —
    /// `variant.debug(fmt)` dispatches to it, as do `Debug::debug` calls and
    /// `print(v)`.
    pub(super) fn register_derived_enum(&self, e: &EnumDef) {
        if !Self::derives_debug(&e.attributes) {
            return;
        }
        let type_name = e.name.clone();
        let variants: Vec<(String, usize)> = e
            .variants
            .iter()
            .map(|v| (v.name.clone(), v.payload.as_deref().unwrap_or(&[]).len()))
            .collect();
        let renderer: Value = Value::Native(Rc::new(move |interp, args| {
            let receiver = args.first().cloned().unwrap_or(Value::Null);
            let format = args.get(1).cloned().unwrap_or_else(|| interp.default_format());
            let out = interp.render_derived_enum(&type_name, &variants, &receiver, &format)?;
            Ok(Value::String(out.into()))
        }));
        self.global.borrow_mut().define(
            &impl_fn_name("Debug", &e.name, "debug"),
            renderer,
            false,
        );
    }

    /// Render an enum instance as `Enum::Variant` or `Enum::Variant(p, …)`,
    /// each payload slot through the derive's `Debug` field rules.
    pub(super) fn render_derived_enum(
        &self,
        type_name: &str,
        variants: &[(String, usize)],
        receiver: &Value,
        format: &Value,
    ) -> Result<String, RunError> {
        let Value::Enum { tag, payload, .. } = receiver else {
            return Ok(receiver.format());
        };
        let Some((_, arity)) = variants.iter().find(|(name, _)| name == tag) else {
            return Ok(receiver.format());
        };
        let slots: Vec<Value> = match payload {
            None => Vec::new(),
            // A single-slot variant stores its one value directly — even when
            // that value is itself a list.
            Some(p) if *arity == 1 => vec![(**p).clone()],
            Some(p) => match &**p {
                Value::List(items) => items.borrow().clone(),
                single => vec![single.clone()],
            },
        };
        if slots.len() != *arity {
            return Ok(receiver.format());
        }
        let mut parts = Vec::with_capacity(slots.len());
        for value in &slots {
            parts.push(self.derived_debug(value, format)?);
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
            generic_params: f.generic_params.clone(),
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
                generic_params: method.generic_params.clone(),
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
