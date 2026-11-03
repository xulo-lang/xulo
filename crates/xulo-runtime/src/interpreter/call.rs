use super::*;

impl Interpreter {
    pub(super) fn call(&self, call: &Call, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
        // Trait dispatch: call the mangled `impl_{Trait}_{Type}_{method}` free
        // function with the receiver as its first argument.
        if let Some(impl_name) = &call.trait_impl {
            // Dispatch through a generic parameter's bound carries a `dyn:`
            // marker instead of a mangled name: the receiver's type is not
            // static inside the generic body, so the impl is resolved on the
            // receiver value's struct object under `Trait::method` (the key
            // `impl Trait for Type` registered there).
            if let Some(rest) = impl_name.strip_prefix("dyn:") {
                let (trait_name, method) = rest.split_once(':').ok_or_else(|| {
                    RunError::err(
                        format!("malformed dynamic dispatch `{impl_name}`"),
                        call.span.clone(),
                    )
                })?;
                let mut args = Vec::new();
                if let Some(object) = &call.object {
                    args.push(self.eval(object, env)?);
                }
                args.extend(self.eval_args(&call.arguments, env)?);
                if matches!(
                    (trait_name, method),
                    ("Display", "display") | ("Debug", "debug")
                ) && args.len() == 1
                {
                    args.push(self.default_format());
                }
                let key = format!("{trait_name}::{method}");
                let mut resolved = None;
                for struct_name in self.struct_names.borrow().iter() {
                    // Like `struct_method`, the struct object is found through
                    // the calling environment: modules bind their declarations
                    // in a child of the interpreter root.
                    let obj = match env.borrow().get(struct_name) {
                        Some(Value::Object(obj)) => obj.clone(),
                        _ => continue,
                    };
                    if let Some(found) = obj
                        .borrow()
                        .iter()
                        .find(|(k, _)| k == &key)
                        .map(|(_, v)| v.clone())
                    {
                        resolved = Some(found);
                        break;
                    }
                }
                let func = resolved.ok_or_else(|| {
                    RunError::err(
                        format!("no impl `{trait_name}` applies to the receiver of `{method}`"),
                        call.span.clone(),
                    )
                })?;
                return self.call_value_with_values(&func, &args);
            }
            let callee = env.borrow().get(impl_name).ok_or_else(|| {
                RunError::err(
                    format!("undefined impl function `{impl_name}`"),
                    call.span.clone(),
                )
            })?;
            let mut values = Vec::new();
            if let Some(object) = &call.object {
                values.push(self.eval(object, env)?);
            }
            values.extend(self.eval_args(&call.arguments, env)?);
            // `str(v)` / `${v}` / `v.to_string()` dispatch to `display` at the
            // default `Format`; a written `Display::display(v, fmt)` already
            // carries one.
            if values.len() == 1
                && ((impl_name.starts_with("impl_Display_") && impl_name.ends_with("_display"))
                    || (impl_name.starts_with("impl_Debug_") && impl_name.ends_with("_debug")))
            {
                values.push(self.default_format());
            }
            return self.call_value_with_values(&callee, &values);
        }
        // `Map::from_entries` and `Set::from_list` are the static constructors
        // of the two built-in container types (spec
        // `types/composite-types.md`); they are not enum variants.
        if call.object.is_none()
            && !self
                .enum_names
                .borrow()
                .iter()
                .any(|n| n == "Map" || n == "Set")
        {
            if call.callee == "Map::from_entries" {
                return self.map_from_entries(call, env);
            }
            if call.callee == "Set::from_list" {
                return self.set_from_list(call, env);
            }
            if call.callee == "Array::filled" {
                return self.array_filled(call, env);
            }
        }

        // A built-in namespace head (`Task::all`) resolves to its intrinsic
        // object in the root environment (spec
        // `builtins/intrinsic-functions.md`). The `std` tree walks the
        // generic namespace path below instead. A member the table does not
        // have falls through to enum construction, which is what a
        // program-declared `enum`-style shadowing head needs (the semantic
        // pass rejects an unknown member on a genuine namespace).
        if let Some((head, member)) = call.callee.split_once("::")
            && !member.contains("::")
            && head == "Task"
            && let Some(func) = self.builtin_ns_member(head, member)
        {
            return self.call_value(&func, &call.arguments, env);
        }
        // A module-namespace member (`use mod as ns; ns::f()`): the head is
        // not a declared enum and names an object in scope — the namespace
        // object exported by `mod`/`use` bindings (spec
        // `expressions/path-and-access.md`: members are reached with `::`).
        if let Some((head, member)) = call.callee.split_once("::")
            && !self.enum_names.borrow().iter().any(|n| n == head)
        {
            let ns = env.borrow().get(head);
            if let Some(Value::Object(ns)) = ns {
                // Walk nested namespaces: `ns::inner::v` reads `inner` on the
                // head object, then `v` on that child object, then calls it.
                let segments: Vec<&str> = member.split("::").collect();
                let mut current = ns;
                for (i, seg) in segments.iter().enumerate() {
                    let found = current
                        .borrow()
                        .iter()
                        .find(|(k, _)| k == seg)
                        .map(|(_, v)| v.clone());
                    if i + 1 == segments.len() {
                        return match found {
                            Some(func) => {
                                // A built-in data type object called as its
                                // named-field constructor (`std::http::Response(…)`):
                                // the `__ctor` marker turns the call into the
                                // struct construction bare names take.
                                if let Value::Object(ref obj) = func
                                    && let Some(Value::String(type_name)) = obj
                                        .borrow()
                                        .iter()
                                        .find(|(k, _)| k == "__ctor")
                                        .map(|(_, v)| v.clone())
                                {
                                    return self.construct_struct(&type_name, call, env);
                                }
                                self.call_value(&func, &call.arguments, env)
                            }
                            None => Err(RunError::err(
                                format!("`{head}` has no member `{member}`"),
                                call.span.clone(),
                            )),
                        };
                    }
                    match found {
                        Some(Value::Object(child)) => current = child,
                        _ => {
                            return Err(RunError::err(
                                format!("`{head}` has no member `{seg}`"),
                                call.span.clone(),
                            ));
                        }
                    }
                }
            }
        }
        if let Some((enum_name, variant)) = call.enum_parts() {
            return self.enum_construct(enum_name, variant, &call.arguments, env);
        }
        if let Some(object) = &call.object {
            return self.method_call(call, object, env);
        }
        // A struct constructor: the callee names a registered `struct` type
        // and the call supplies its fields as named arguments (spec
        // `types/composite-types.md`: `User(name: "Alice", age: 30)`).
        if let Some(Value::Object(_)) = env.borrow().get(&call.callee)
            && self.struct_fields.borrow().contains_key(&call.callee)
        {
            return self.construct_struct(&call.callee, call, env);
        }
        let callee = env.borrow().get(&call.callee);
        match callee {
            Some(callee) => {
                // An extern import binds exactly like a declared `fn` at the
                // call site (spec ffi.md Calling): named arguments reorder by
                // the declaration, and an omitted parameter (only `Ptr?` may
                // be omitted) binds `null`. Registered under the bare callee
                // name — the only call position an import has (E0903) — and
                // only honored when that name still binds the import itself:
                // a shadowing local (permitted by the checker when the
                // spelling is re-declared) takes the ordinary path.
                let extern_names = self
                    .extern_params
                    .borrow()
                    .get(&call.callee)
                    .filter(|_| matches!(callee, Value::Native(_)))
                    .cloned();
                match extern_names {
                    Some(names) => {
                        let values = self.bind_extern_args(&names, &call.arguments, env)?;
                        self.call_value_with_values(&callee, &values)
                    }
                    None => {
                        if let Value::Function(func) = &callee
                            && func
                                .generic_params
                                .iter()
                                .any(|p| p.kind == xulo_core::ast::GenericParamKind::Const)
                        {
                            let consts = self.eval_const_args(func, call, env)?;
                            return self.call_function_with_consts(
                                func,
                                &call.arguments,
                                env,
                                &consts,
                            );
                        }
                        self.call_value(&callee, &call.arguments, env)
                    }
                }
            }
            None => Err(RunError::err(
                format!("undefined function `{}`", call.callee),
                call.span.clone(),
            )),
        }
    }

    /// Build a struct instance from `User(name: …, age: …)`: named arguments
    /// only, defaults evaluated against the declaration's environment, and an
    /// omitted field without a default left `null` — the semantic pass reports
    /// a missing required field as `E0205` before execution, while an omitted
    /// optional field carries no value of its own (spec
    /// `types/composite-types.md`).
    pub(super) fn construct_struct(
        &self,
        name: &str,
        call: &Call,
        call_env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        let decl = {
            let table = self.struct_fields.borrow();
            table.get(name).cloned().ok_or_else(|| {
                RunError::err(
                    format!("`{name}` is not a struct constructor"),
                    call.span.clone(),
                )
            })?
        };
        let mut args: std::collections::HashMap<&str, &CallArg> = std::collections::HashMap::new();
        for arg in &call.arguments {
            let Some(label) = arg.name.as_deref() else {
                return Err(RunError::err(
                    format!("struct `{name}` is constructed with named arguments"),
                    arg.value.span().clone(),
                ));
            };
            if !decl.fields.iter().any(|(fname, _)| fname == label) {
                return Err(RunError::err(
                    format!("`{name}` has no field `{label}`"),
                    arg.value.span().clone(),
                ));
            }
            if args.insert(label, arg).is_some() {
                return Err(RunError::err(
                    format!("argument `{label}` to `{name}` is provided twice"),
                    arg.value.span().clone(),
                ));
            }
        }
        let mut fields = Vec::with_capacity(decl.fields.len());
        for (fname, default) in &decl.fields {
            let value = if let Some(arg) = args.get(fname.as_str()) {
                self.eval(&arg.value, call_env)?
            } else if let Some(default) = default {
                self.eval(default, &decl.env)?
            } else {
                Value::Null
            };
            fields.push((fname.clone(), value));
        }
        let obj = Rc::new(RefCell::new(fields));
        self.instance_types.borrow_mut().insert(
            Rc::as_ptr(&obj) as usize,
            (Rc::downgrade(&obj), name.to_string()),
        );
        Ok(Value::Object(obj))
    }

    /// The struct type a constructed instance was built from, while its
    /// identity still resolves in the instance table.
    pub(super) fn instance_type(&self, obj: &Rc<RefCell<Vec<(String, Value)>>>) -> Option<String> {
        let ptr = Rc::as_ptr(obj) as usize;
        let hit = {
            let table = self.instance_types.borrow();
            let (weak, name) = table.get(&ptr)?;
            weak.upgrade()
                .map(|rc| (Rc::as_ptr(&rc) as usize == ptr, name.clone()))
        };
        match hit {
            Some((true, name)) => Some(name),
            Some((false, _)) => {
                self.instance_types.borrow_mut().remove(&ptr);
                None
            }
            None => None,
        }
    }

    /// Record a built instance's type — the same bookkeeping
    /// `construct_struct` does — so method dispatch finds that type's
    /// methods on it (`Response.text`'s output answers `.header`).
    pub(super) fn register_instance(&self, obj: &Rc<RefCell<Vec<(String, Value)>>>, name: &str) {
        self.instance_types.borrow_mut().insert(
            Rc::as_ptr(obj) as usize,
            (Rc::downgrade(obj), name.to_string()),
        );
    }

    /// A member of a built-in namespace object registered in the root
    /// environment (`Math`/`Time`/`Task`), cloned out for dispatch.
    pub(super) fn builtin_ns_member(&self, ns: &str, member: &str) -> Option<Value> {
        let global = self.global.borrow();
        match global.get(ns)? {
            Value::Object(map) => map
                .borrow()
                .iter()
                .find(|(k, _)| k == member)
                .map(|(_, v)| v.clone()),
            _ => None,
        }
    }

    pub(super) fn method_call(
        &self,
        call: &Call,
        object_expr: &Expression,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        let receiver = self.eval(object_expr, env)?;
        if call.optional && matches!(receiver, Value::Null) {
            return Ok(Value::Null);
        }
        let method = call.method.as_deref().unwrap_or("");
        let callee = match &receiver {
            Value::Object(fields) => fields
                .borrow()
                .iter()
                .find(|(k, _)| k == method)
                .map(|(_, v)| v.clone()),
            Value::List(list) => {
                return self.list_method(method, list.clone(), &call.arguments, &call.span, env);
            }
            Value::String(s) => {
                return self.string_method(method, s.clone(), &call.arguments, &call.span, env);
            }
            Value::Set(set) => {
                return self.set_method(method, set.clone(), &call.arguments, &call.span, env);
            }
            _ => None,
        };
        match callee {
            Some(v) => self.call_value(&v, &call.arguments, env),
            None => {
                // A constructed instance knows its struct type: the method
                // resolves on that type's object (inherent methods, a derived
                // `to_string`) with the receiver bound as `self`, so two
                // structs declaring the same method name never cross-fire.
                if let Value::Object(obj) = &receiver
                    && let Some(struct_name) = self.instance_type(obj)
                    && let Some(Value::Object(type_obj)) = env.borrow().get(&struct_name)
                    && let Some(func) = type_obj
                        .borrow()
                        .iter()
                        .find(|(k, _)| k == method)
                        .map(|(_, v)| v.clone())
                {
                    let arg_values = self.eval_args(&call.arguments, env)?;
                    let mut args = Vec::with_capacity(arg_values.len() + 1);
                    args.push(receiver.clone());
                    args.extend(arg_values);
                    return self.call_value_with_values(&func, &args);
                }
                // Inherent impl methods live on the struct's object in the
                // global environment (`impl User { fn method(self, ...) }`), not
                // on instances. Fall back to the matching struct method, binding
                // the receiver as the first (`self`) argument — the scan
                // applies only to instances the runtime cannot place (a plain
                // object literal).
                if let Value::Object(_) = &receiver
                    && let Some(struct_callee) =
                        self.struct_method(method, &receiver, &call.arguments, env)
                {
                    return struct_callee;
                }
                // A plain object — the brace form of a map literal or an
                // untyped map value — answers the Map members once no
                // struct method claimed the name (spec
                // `types/composite-types.md`).
                if let Value::Object(fields) = &receiver
                    && self.instance_type(fields).is_none()
                {
                    return self.map_method(
                        method,
                        fields.clone(),
                        &call.arguments,
                        &call.span,
                        env,
                    );
                }
                // `to_string` is the derived action of `Display`: when no
                // inherent or declared member claimed it, a `Display`
                // implementor answers through `display` at the default
                // `Format` (spec `types/traits.md`, "Member calls").
                if method == "to_string"
                    && call.arguments.is_empty()
                    && let Some(out) = self.call_trait_impl(
                        "Display",
                        "display",
                        &receiver,
                        std::slice::from_ref(&self.default_format()),
                    )
                {
                    return out;
                }
                Err(RunError::err(
                    format!("`{method}` is not a function of {}", receiver.kind_name()),
                    call.span.clone(),
                ))
            }
        }
    }

    /// Fall back to an inherent impl method registered on a `struct` object in
    /// the global environment when it is not found on the receiver itself. The
    /// receiver is passed as the first (`self`) argument.
    pub(super) fn struct_method(
        &self,
        method: &str,
        receiver: &Value,
        call_args: &[CallArg],
        call_env: &Rc<RefCell<Env>>,
    ) -> Option<Result<Value, RunError>> {
        for struct_name in self.struct_names.borrow().iter() {
            // Struct declarations live in the module environment that
            // registered them (`exec_module` binds them in a child of the
            // interpreter root, while `run` uses the root itself), so the
            // lookup follows the calling environment's chain instead of the
            // root alone.
            let struct_obj = match call_env.borrow().get(struct_name) {
                Some(Value::Object(obj)) => obj.clone(),
                _ => continue,
            };
            let Some(func) = struct_obj
                .borrow()
                .iter()
                .find(|(k, _)| k == method)
                .map(|(_, v)| v.clone())
            else {
                continue;
            };
            let arg_values = self.eval_args(call_args, call_env).ok()?;
            let args = std::iter::once(receiver.clone())
                .chain(arg_values)
                .collect::<Vec<_>>();
            return Some(self.call_value_with_values(&func, &args));
        }
        None
    }

    // ──────────────────────────────────────────────────────────────────
    //  Built-in list methods
    // ──────────────────────────────────────────────────────────────────

    /// The methods a `string` receiver carries. `replace` consumes a compiled
    /// `RegExp` and a template or replacement function; the rest reshape or
    /// query the text (`to_uppercase`/`to_lowercase`/`trim`/`pad_start`/
    /// `pad_end`/`repeat`/`split`/`length`) or test it (`contains`/
    /// `starts_with`/`ends_with`). Every member is pure: the receiver is
    /// never mutated, and the reshaping ones yield a new string.
    pub(super) fn string_method(
        &self,
        method: &str,
        receiver: Rc<str>,
        args: &[CallArg],
        span: &std::ops::Range<usize>,
        env: &Rc<RefCell<crate::env::Env>>,
    ) -> Result<Value, RunError> {
        match method {
            "code_at" => {
                expect_args(method, args, 1, 1, span)?;
                let n = self.eval_number_arg(&args[0], env)?;
                let i = non_negative_index(n, span)?;
                let len = receiver.chars().count();
                if i >= len {
                    return Err(RunError::err(
                        format!("index {n} out of bounds for a string of length {len}"),
                        span.clone(),
                    ));
                }
                Ok(Value::Number(
                    receiver.chars().nth(i).map(|c| c as u32).unwrap_or(0) as f64,
                ))
            }
            "slice" => {
                expect_args(method, args, 2, 2, span)?;
                let start = self.eval_number_arg(&args[0], env)?;
                let end = self.eval_number_arg(&args[1], env)?;
                let len = receiver.chars().count();
                let (s, e) = slice_bounds(start, Some(end), len, span)?;
                let out: String = receiver.chars().skip(s).take(e - s).collect();
                Ok(Value::String(out.into()))
            }
            "bytes" => {
                expect_args(method, args, 0, 0, span)?;
                let bytes: Vec<Value> = receiver
                    .as_bytes()
                    .iter()
                    .map(|b| Value::Number(*b as f64))
                    .collect();
                Ok(Value::List(Rc::new(RefCell::new(bytes))))
            }
            "replace" => {
                if args.len() != 2 {
                    return Err(RunError::err(
                        format!(
                            "`replace()` requires exactly 2 arguments (a RegExp and a replacement), found {}",
                            args.len()
                        ),
                        span.clone(),
                    ));
                }
                let pattern = self.eval(&args[0].value, env)?;
                let replacement = self.eval(&args[1].value, env)?;
                let replace_impl = match &pattern {
                    Value::Object(fields) => fields
                        .borrow()
                        .iter()
                        .find(|(k, _)| k == "__replace")
                        .map(|(_, v)| v.clone()),
                    _ => None,
                };
                let Some(replace_impl) = replace_impl else {
                    return Err(RunError::err(
                        "`replace` expects a compiled `RegExp` pattern",
                        span.clone(),
                    ));
                };
                self.call_value_with_values(&replace_impl, &[Value::String(receiver), replacement])
            }
            "split" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        format!(
                            "`split()` requires exactly 1 argument, found {}",
                            args.len()
                        ),
                        span.clone(),
                    ));
                }
                let arg = self.eval(&args[0].value, env)?;
                if let Value::Object(fields) = &arg
                    && let Some(split_fn) = fields
                        .borrow()
                        .iter()
                        .find(|(k, _)| k == "__split")
                        .map(|(_, v)| v.clone())
                {
                    return self.call_value_with_values(&split_fn, &[Value::String(receiver)]);
                }
                let Value::String(sep) = arg else {
                    return Err(RunError::err(
                        "`split()` expects a compiled `RegExp`",
                        args[0].value.span().clone(),
                    ));
                };
                let parts: Vec<Value> = if sep.is_empty() {
                    receiver
                        .chars()
                        .map(|ch| Value::String(ch.to_string().into()))
                        .collect()
                } else {
                    receiver
                        .split(&*sep)
                        .map(|part| Value::String(part.into()))
                        .collect()
                };
                Ok(Value::List(Rc::new(RefCell::new(parts))))
            }
            _ => Err(RunError::err(
                format!("`{method}` is not a method of string"),
                span.clone(),
            )),
        }
    }

    /// `Array::filled<T, const N: int>(value)`: N copies of `value` (spec
    /// `types/composite-types.md`). The runtime carries a fixed array as a
    /// list-shaped value; the static type keeps the length.
    fn array_filled(&self, call: &Call, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
        let args = &call.arguments;
        if args.len() != 1 {
            return Err(RunError::err(
                format!(
                    "`Array::filled` expects exactly 1 argument, found {}",
                    args.len()
                ),
                call.span.clone(),
            ));
        }
        let Some(targs) = &call.type_args else {
            return Err(RunError::err(
                "`Array::filled` requires a written generic-argument list",
                call.span.clone(),
            ));
        };
        let Some(length) = targs.get(1) else {
            return Err(RunError::err(
                "`Array::filled` requires a const length argument",
                call.span.clone(),
            ));
        };
        let n = match length {
            Type::Literal(text) => text.parse::<i64>().map_err(|_| {
                RunError::err(format!("`{text}` is not a const length"), call.span.clone())
            })?,
            Type::Named(name, a) if a.is_empty() => match env.borrow().get(name) {
                Some(Value::Number(n)) => n as i64,
                _ => {
                    return Err(RunError::err(
                        format!("`{name}` is not a const length"),
                        call.span.clone(),
                    ));
                }
            },
            other => {
                return Err(RunError::err(
                    format!("{} is not a const length", other.display()),
                    call.span.clone(),
                ));
            }
        };
        if n < 0 {
            return Err(RunError::err(
                format!("array length must be non-negative, found {n}"),
                call.span.clone(),
            ));
        }
        let value = self.eval(&args[0].value, env)?;
        Ok(Value::List(Rc::new(RefCell::new(vec![value; n as usize]))))
    }

    /// `Map::from_entries(entries)`: build a map from `(K, V)` pairs in list
    /// order, with a later duplicate key replacing the earlier one.
    fn map_from_entries(&self, call: &Call, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
        expect_args("Map::from_entries", &call.arguments, 1, 1, &call.span)?;
        let entries = self.eval(&call.arguments[0].value, env)?;
        let Value::List(entries) = entries else {
            return Err(RunError::err(
                "`Map::from_entries` expects a List of (key, value) pairs",
                call.arguments[0].value.span().clone(),
            ));
        };
        let entries = entries.borrow().clone();
        let mut fields: Vec<(String, Value)> = Vec::with_capacity(entries.len());
        for entry in entries {
            let Value::List(pair) = entry else {
                return Err(RunError::err(
                    "`Map::from_entries` expects each entry to be a (key, value) pair",
                    call.span.clone(),
                ));
            };
            let pair = pair.borrow();
            if pair.len() != 2 {
                return Err(RunError::err(
                    "`Map::from_entries` expects each entry to be a (key, value) pair",
                    call.span.clone(),
                ));
            }
            let key = map_key_string(&pair[0]);
            let value = pair[1].clone();
            match fields.iter_mut().find(|(k, _)| *k == key) {
                Some((_, slot)) => *slot = value,
                None => fields.push((key, value)),
            }
        }
        Ok(Value::Object(Rc::new(RefCell::new(fields))))
    }

    /// `Set::from_list(values)`: build a set holding each distinct value once.
    fn set_from_list(&self, call: &Call, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
        expect_args("Set::from_list", &call.arguments, 1, 1, &call.span)?;
        let values = self.eval(&call.arguments[0].value, env)?;
        let Value::List(values) = values else {
            return Err(RunError::err(
                "`Set::from_list` expects a List",
                call.arguments[0].value.span().clone(),
            ));
        };
        let mut out: Vec<Value> = Vec::new();
        for value in values.borrow().iter().cloned() {
            if !out.iter().any(|seen| equal(seen, &value)) {
                out.push(value);
            }
        }
        Ok(Value::Set(Rc::new(RefCell::new(out))))
    }

    pub(super) fn map_method(
        &self,
        method: &str,
        fields: Rc<RefCell<Vec<(String, Value)>>>,
        args: &[CallArg],
        span: &std::ops::Range<usize>,
        env: &Rc<RefCell<crate::env::Env>>,
    ) -> Result<Value, RunError> {
        match method {
            "get" | "has" | "remove" => {
                expect_args(method, args, 1, 1, span)?;
                let key = map_key_string(&self.eval(&args[0].value, env)?);
                let mut fields = fields.borrow_mut();
                let found = fields.iter().position(|(k, _)| *k == key);
                match method {
                    "get" => Ok(found.map(|i| fields[i].1.clone()).unwrap_or(Value::Null)),
                    "has" => Ok(Value::Boolean(found.is_some())),
                    _ => Ok(found.map(|i| fields.remove(i).1).unwrap_or(Value::Null)),
                }
            }
            "set" => {
                expect_args(method, args, 2, 2, span)?;
                let key = map_key_string(&self.eval(&args[0].value, env)?);
                let value = self.eval(&args[1].value, env)?;
                let mut fields = fields.borrow_mut();
                match fields.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, slot)) => *slot = value,
                    None => fields.push((key, value)),
                }
                Ok(Value::Null)
            }
            "keys" => {
                expect_args(method, args, 0, 0, span)?;
                let keys: Vec<Value> = fields
                    .borrow()
                    .iter()
                    .map(|(k, _)| Value::String(k.clone().into()))
                    .collect();
                Ok(Value::List(Rc::new(RefCell::new(keys))))
            }
            "values" => {
                expect_args(method, args, 0, 0, span)?;
                let values: Vec<Value> = fields.borrow().iter().map(|(_, v)| v.clone()).collect();
                Ok(Value::List(Rc::new(RefCell::new(values))))
            }
            "entries" => {
                expect_args(method, args, 0, 0, span)?;
                let entries: Vec<Value> = fields
                    .borrow()
                    .iter()
                    .map(|(k, v)| {
                        Value::List(Rc::new(RefCell::new(vec![
                            Value::String(k.clone().into()),
                            v.clone(),
                        ])))
                    })
                    .collect();
                Ok(Value::List(Rc::new(RefCell::new(entries))))
            }
            _ => Err(RunError::err(
                format!("`{method}` is not a method of map"),
                span.clone(),
            )),
        }
    }

    pub(super) fn set_method(
        &self,
        method: &str,
        set: Rc<RefCell<Vec<Value>>>,
        args: &[CallArg],
        span: &std::ops::Range<usize>,
        env: &Rc<RefCell<crate::env::Env>>,
    ) -> Result<Value, RunError> {
        match method {
            "has" | "insert" | "remove" => {
                expect_args(method, args, 1, 1, span)?;
                let value = self.eval(&args[0].value, env)?;
                let mut set = set.borrow_mut();
                let found = set.iter().position(|v| equal(v, &value));
                match method {
                    "has" => Ok(Value::Boolean(found.is_some())),
                    "insert" => {
                        if found.is_some() {
                            Ok(Value::Boolean(false))
                        } else {
                            set.push(value);
                            Ok(Value::Boolean(true))
                        }
                    }
                    _ => {
                        if let Some(i) = found {
                            set.remove(i);
                            Ok(Value::Boolean(true))
                        } else {
                            Ok(Value::Boolean(false))
                        }
                    }
                }
            }
            _ => Err(RunError::err(
                format!("`{method}` is not a method of set"),
                span.clone(),
            )),
        }
    }

    pub(super) fn list_method(
        &self,
        method: &str,
        list: std::rc::Rc<RefCell<Vec<Value>>>,
        args: &[CallArg],
        span: &std::ops::Range<usize>,
        env: &Rc<RefCell<crate::env::Env>>,
    ) -> Result<Value, RunError> {
        use std::cmp::Ordering;
        match method {
            "push" => {
                expect_args(method, args, 1, 1, span)?;
                let val = self.eval(&args[0].value, env)?;
                list.borrow_mut().push(val);
                Ok(Value::Null)
            }
            "pop" => {
                expect_args(method, args, 0, 0, span)?;
                Ok(list.borrow_mut().pop().unwrap_or(Value::Null))
            }
            "shift" => {
                expect_args(method, args, 0, 0, span)?;
                let mut l = list.borrow_mut();
                if l.is_empty() {
                    Ok(Value::Null)
                } else {
                    Ok(l.remove(0))
                }
            }
            "unshift" => {
                expect_args(method, args, 1, 1, span)?;
                let val = self.eval(&args[0].value, env)?;
                list.borrow_mut().insert(0, val);
                Ok(Value::Null)
            }
            "splice" => {
                expect_args(method, args, 2, 3, span)?;
                let start = self.eval_number_arg(&args[0], env)?;
                let delete = self.eval_number_arg(&args[1], env)?;
                let items = if args.len() == 3 {
                    match self.eval(&args[2].value, env)? {
                        Value::List(items) => items.borrow().clone(),
                        other => {
                            return Err(RunError::err(
                                format!(
                                    "splice() expects a List of items, found {}",
                                    other.kind_name()
                                ),
                                args[2].value.span().clone(),
                            ));
                        }
                    }
                } else {
                    Vec::new()
                };
                let len = list.borrow().len();
                let start = non_negative_index(start, span)?;
                if start > len {
                    return Err(RunError::err(
                        format!("splice start {start} out of bounds for a list of length {len}"),
                        span.clone(),
                    ));
                }
                let delete = non_negative_index(delete, span)?;
                if delete > len - start {
                    return Err(RunError::err(
                        format!(
                            "splice delete_count {delete} out of bounds at index {start} for a list of length {len}"
                        ),
                        span.clone(),
                    ));
                }
                let removed: Vec<Value> = list
                    .borrow_mut()
                    .splice(start..start + delete, items)
                    .collect();
                Ok(Value::List(Rc::new(RefCell::new(removed))))
            }
            "resize" => {
                expect_args(method, args, 2, 2, span)?;
                let n = self.eval_number_arg(&args[0], env)?;
                let fill = self.eval(&args[1].value, env)?;
                let n = non_negative_index(n, span)?;
                list.borrow_mut().resize(n, fill);
                Ok(Value::Null)
            }
            "truncate" => {
                expect_args(method, args, 1, 1, span)?;
                let n = self.eval_number_arg(&args[0], env)?;
                let n = non_negative_index(n, span)?;
                list.borrow_mut().truncate(n);
                Ok(Value::Null)
            }
            "clear" => {
                expect_args(method, args, 0, 0, span)?;
                list.borrow_mut().clear();
                Ok(Value::Null)
            }
            "slice" => {
                expect_args(method, args, 1, 2, span)?;
                let len = list.borrow().len();
                let start = self.eval_number_arg(&args[0], env)?;
                let end = if args.len() == 2 {
                    Some(self.eval_number_arg(&args[1], env)?)
                } else {
                    None
                };
                let (s, e) = slice_bounds(start, end, len, span)?;
                let out = list.borrow()[s..e].to_vec();
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            "index_of" | "last_index_of" | "includes" => {
                expect_args(method, args, 1, 1, span)?;
                let val = self.eval(&args[0].value, env)?;
                let l = list.borrow();
                match method {
                    "index_of" => Ok(Value::Number(
                        l.iter()
                            .position(|v| equal(v, &val))
                            .map(|i| i as f64)
                            .unwrap_or(-1.0),
                    )),
                    "last_index_of" => Ok(Value::Number(
                        l.iter()
                            .rposition(|v| equal(v, &val))
                            .map(|i| i as f64)
                            .unwrap_or(-1.0),
                    )),
                    _ => Ok(Value::Boolean(l.iter().any(|v| equal(v, &val)))),
                }
            }
            "join" => {
                expect_args(method, args, 0, 1, span)?;
                let sep = if args.is_empty() {
                    ",".to_string()
                } else {
                    match self.eval(&args[0].value, env)? {
                        Value::String(s) => s.to_string(),
                        v => v.format(),
                    }
                };
                let l = list.borrow();
                let parts: Vec<String> = l
                    .iter()
                    .map(|v| self.render_display(v))
                    .collect::<Result<_, _>>()?;
                Ok(Value::String(parts.join(&sep).into()))
            }
            "reverse" => {
                expect_args(method, args, 0, 0, span)?;
                list.borrow_mut().reverse();
                Ok(Value::Null)
            }
            "entries" => {
                expect_args(method, args, 0, 0, span)?;
                let out: Vec<Value> = list
                    .borrow()
                    .iter()
                    .enumerate()
                    .map(|(i, v)| {
                        Value::List(Rc::new(RefCell::new(vec![
                            Value::Number(i as f64),
                            v.clone(),
                        ])))
                    })
                    .collect();
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            "for_each" => {
                expect_args(method, args, 1, 1, span)?;
                let f = self.eval(&args[0].value, env)?;
                let snapshot = list.borrow().clone();
                for item in snapshot {
                    self.call_value_with_values(&f, &[item])?;
                }
                Ok(Value::Null)
            }
            "map" => {
                expect_args(method, args, 1, 1, span)?;
                let f = self.eval(&args[0].value, env)?;
                let snapshot = list.borrow().clone();
                let mut out = Vec::with_capacity(snapshot.len());
                for item in snapshot {
                    out.push(self.call_value_with_values(&f, &[item])?);
                }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            "filter" => {
                expect_args(method, args, 1, 1, span)?;
                let f = self.eval(&args[0].value, env)?;
                let snapshot = list.borrow().clone();
                let mut out = Vec::new();
                for item in snapshot {
                    if is_truthy(&self.call_value_with_values(&f, std::slice::from_ref(&item))?) {
                        out.push(item);
                    }
                }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            "find" => {
                expect_args(method, args, 1, 1, span)?;
                let f = self.eval(&args[0].value, env)?;
                let snapshot = list.borrow().clone();
                for item in snapshot {
                    if is_truthy(&self.call_value_with_values(&f, std::slice::from_ref(&item))?) {
                        return Ok(item);
                    }
                }
                Ok(Value::Null)
            }
            "find_index" => {
                expect_args(method, args, 1, 1, span)?;
                let f = self.eval(&args[0].value, env)?;
                let snapshot = list.borrow().clone();
                for (i, item) in snapshot.into_iter().enumerate() {
                    if is_truthy(&self.call_value_with_values(&f, &[item])?) {
                        return Ok(Value::Number(i as f64));
                    }
                }
                Ok(Value::Number(-1.0))
            }
            "some" => {
                expect_args(method, args, 1, 1, span)?;
                let f = self.eval(&args[0].value, env)?;
                let snapshot = list.borrow().clone();
                for item in snapshot {
                    if is_truthy(&self.call_value_with_values(&f, std::slice::from_ref(&item))?) {
                        return Ok(Value::Boolean(true));
                    }
                }
                Ok(Value::Boolean(false))
            }
            "every" => {
                expect_args(method, args, 1, 1, span)?;
                let f = self.eval(&args[0].value, env)?;
                let snapshot = list.borrow().clone();
                for item in snapshot {
                    if !is_truthy(&self.call_value_with_values(&f, std::slice::from_ref(&item))?) {
                        return Ok(Value::Boolean(false));
                    }
                }
                Ok(Value::Boolean(true))
            }
            "reduce" => {
                expect_args(method, args, 2, 2, span)?;
                let f = self.eval(&args[0].value, env)?;
                let mut acc = self.eval(&args[1].value, env)?;
                let snapshot = list.borrow().clone();
                for item in snapshot {
                    acc = self.call_value_with_values(&f, &[acc, item])?;
                }
                Ok(acc)
            }
            "sort" => {
                expect_args(method, args, 1, 1, span)?;
                let f = self.eval(&args[0].value, env)?;
                let mut failure: Option<RunError> = None;
                list.borrow_mut().sort_by(|a, b| {
                    if failure.is_some() {
                        return Ordering::Equal;
                    }
                    match self.call_value_with_values(&f, &[a.clone(), b.clone()]) {
                        Ok(Value::Number(n)) => {
                            if n < 0.0 {
                                Ordering::Less
                            } else if n > 0.0 {
                                Ordering::Greater
                            } else {
                                Ordering::Equal
                            }
                        }
                        Ok(other) => {
                            failure = Some(RunError::err(
                                format!(
                                    "sort comparator must return a number, found {}",
                                    other.kind_name()
                                ),
                                span.clone(),
                            ));
                            Ordering::Equal
                        }
                        Err(e) => {
                            failure = Some(e);
                            Ordering::Equal
                        }
                    }
                });
                match failure {
                    Some(e) => Err(e),
                    None => Ok(Value::Null),
                }
            }
            _ => Err(RunError::err(
                format!("`{method}` is not a method of list"),
                span.clone(),
            )),
        }
    }
    pub(super) fn eval_number_arg(
        &self,
        arg: &CallArg,
        env: &Rc<RefCell<crate::env::Env>>,
    ) -> Result<f64, RunError> {
        let val = self.eval(&arg.value, env)?;
        match val {
            Value::Number(n) => Ok(n),
            other => Err(RunError::err(
                format!("expected a number, found {}", other.kind_name()),
                arg.value.span().clone(),
            )),
        }
    }

    pub(super) fn enum_construct(
        &self,
        enum_name: &str,
        variant: &str,
        args: &[CallArg],
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        if args.is_empty() {
            // A payload-less variant called with an empty argument list
            // (`Color::Red()`) is the variant value itself.
            return Ok(Value::Enum {
                enum_name: enum_name.to_string(),
                tag: variant.to_string(),
                payload: None,
            });
        }
        if args.iter().any(|a| a.name.is_some()) {
            return Err(RunError::err(
                "named arguments are not supported in enum constructors",
                0..0,
            ));
        }
        let mut values = Vec::new();
        for arg in args {
            values.push(self.eval(&arg.value, env)?);
        }
        let payload = if values.len() == 1 {
            Some(Box::new(values.into_iter().next().expect("one value")))
        } else {
            Some(Box::new(Value::List(Rc::new(RefCell::new(values)))))
        };
        Ok(Value::Enum {
            enum_name: enum_name.to_string(),
            tag: variant.to_string(),
            payload,
        })
    }

    /// Call any callable value: a user function, a closure, or a builtin.
    pub fn call_value(
        &self,
        callee: &Value,
        args: &[CallArg],
        call_env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        match callee {
            Value::Native(native) => {
                // Builtins (`print`, `str`) are called variadically. The JS
                // codegen emits argument *values* in source order and drops any
                // labels (`print(msg: "hi")` compiles to `console.log("hi")`),
                // so the native runtime must ignore names too — erroring here
                // made valid checked programs fail only on the native path.
                let values = self.eval_args(args, call_env)?;
                native(self, &values)
            }
            Value::Function(func) => {
                let depth = self.call_depth.get();
                if depth >= MAX_CALL_DEPTH {
                    return Err(RunError::err(
                        format!("call depth exceeded: recursion limit of {MAX_CALL_DEPTH} reached"),
                        args.first().map(|a| a.value.span().clone()).unwrap_or(0..0),
                    ));
                }
                self.call_depth.set(depth + 1);
                let result = (|| {
                    let bound = self.bind_args(&func.params, args, call_env)?;
                    self.run_function(func, bound)
                })();
                self.call_depth.set(depth);
                result
            }
            other => Err(RunError::err(
                format!("{} is not callable", other.format()),
                0..0,
            )),
        }
    }

    /// Call a callable with pre-evaluated `Value` arguments (bypasses expression
    /// evaluation). Used by built-in higher-order list methods and UI callbacks
    /// that need to pass arguments (e.g. Input `onChange`).
    pub fn call_value_with_values(
        &self,
        callee: &Value,
        values: &[Value],
    ) -> Result<Value, RunError> {
        match callee {
            Value::Native(native) => native(self, values),
            Value::Function(func) => {
                let depth = self.call_depth.get();
                if depth >= MAX_CALL_DEPTH {
                    return Err(RunError::err(
                        format!("call depth exceeded: recursion limit of {MAX_CALL_DEPTH} reached"),
                        0..0,
                    ));
                }
                self.call_depth.set(depth + 1);
                let empty_env = crate::env::Env::root();
                let result = (|| {
                    let mut bound = Vec::with_capacity(func.params.len());
                    for (i, param) in func.params.iter().enumerate() {
                        bound.push(if i < values.len() {
                            Some(values[i].clone())
                        } else {
                            Some(self.default_param(param, &empty_env)?)
                        });
                    }
                    self.run_function(func, bound)
                })();
                self.call_depth.set(depth);
                result
            }
            other => Err(RunError::err(
                format!("{} is not callable", other.format()),
                0..0,
            )),
        }
    }

    pub(super) fn eval_args(
        &self,
        args: &[CallArg],
        env: &Rc<RefCell<Env>>,
    ) -> Result<Vec<Value>, RunError> {
        let mut values = Vec::with_capacity(args.len());
        for arg in args {
            values.push(self.eval(&arg.value, env)?);
        }
        Ok(values)
    }

    /// Evaluate call arguments and reorder them into the callee's declared
    /// parameter order. All-named argument lists are reordered by name.
    /// Omitted parameters come back as `None` — the caller binds what it has
    /// into the callee's scope and evaluates defaults there
    /// ([`bind_callee_args`]), so a default expression sees the callee's own
    /// environment (`fn f(a, b = a)`), never the caller's.
    pub(super) fn bind_args(
        &self,
        params: &[Param],
        args: &[CallArg],
        call_env: &Rc<RefCell<Env>>,
    ) -> Result<Vec<Option<Value>>, RunError> {
        let all_named = !args.is_empty() && args.iter().all(|a| a.name.is_some());
        if all_named {
            let mut by_name: std::collections::HashMap<String, Value> =
                std::collections::HashMap::new();
            for arg in args {
                let name = arg.name.clone().expect("named argument");
                by_name.insert(name, self.eval(&arg.value, call_env)?);
            }
            let mut values = Vec::with_capacity(params.len());
            for param in params {
                values.push(by_name.remove(&param.name));
            }
            return Ok(values);
        }
        let values = self.eval_args(args, call_env)?;
        let mut out = Vec::with_capacity(params.len());
        for i in 0..params.len() {
            out.push(values.get(i).cloned());
        }
        Ok(out)
    }

    /// Arguments for an extern import, mirroring [`Self::bind_args`] plus the
    /// extern-specific default: an omitted parameter (only `Ptr?` may be
    /// omitted, spec ffi.md Calling) binds `null` — extern declarations carry
    /// no default expression. A bare import takes no receiver, so this runs
    /// only on the direct-call path (E0903).
    fn bind_extern_args(
        &self,
        declared: &[String],
        args: &[CallArg],
        env: &Rc<RefCell<Env>>,
    ) -> Result<Vec<Value>, RunError> {
        let all_named = !args.is_empty() && args.iter().all(|a| a.name.is_some());
        if all_named {
            let mut by_name: std::collections::HashMap<&str, Value> =
                std::collections::HashMap::new();
            for arg in args {
                let name = arg.name.as_deref().expect("named argument");
                by_name.insert(name, self.eval(&arg.value, env)?);
            }
            let mut out = Vec::with_capacity(declared.len());
            for name in declared {
                out.push(by_name.remove(name.as_str()).unwrap_or(Value::Null));
            }
            return Ok(out);
        }
        let values = self.eval_args(args, env)?;
        let mut out = Vec::with_capacity(declared.len());
        for i in 0..declared.len() {
            out.push(values.get(i).cloned().unwrap_or(Value::Null));
        }
        Ok(out)
    }

    pub(super) fn default_param(
        &self,
        param: &Param,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        match &param.default {
            Some(expr) => self.eval(expr, env),
            None => Ok(Value::Null),
        }
    }

    /// Bind `args` (in parameter order, `None` for omitted) into the callee
    /// scope, evaluating omitted parameters' default expressions *inside that
    /// scope* — earlier parameters are already defined when a later default
    /// runs, matching the JS codegen (`function f(a, b = a)`).
    pub(super) fn bind_callee_args(
        &self,
        env: &Rc<RefCell<Env>>,
        params: &[Param],
        args: Vec<Option<Value>>,
    ) -> Result<(), RunError> {
        for (param, given) in params.iter().zip(args) {
            let value = match given {
                Some(v) => v,
                None => self.default_param(param, env)?,
            };
            env.borrow_mut().define(&param.name, value, false);
        }
        Ok(())
    }

    /// Evaluate the const arguments a call writes, in declaration order. A
    /// const argument is an integer literal, a `const` binding, or a const
    /// parameter name (spec `types/generics.md`).
    fn eval_const_args(
        &self,
        func: &FunctionValue,
        call: &Call,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Vec<(String, i64)>, RunError> {
        let Some(written) = &call.type_args else {
            return Err(RunError::err(
                format!("`{}` requires a written generic-argument list", call.callee),
                call.span.clone(),
            ));
        };
        let mut out = Vec::new();
        for (i, p) in func.generic_params.iter().enumerate() {
            if p.kind != xulo_core::ast::GenericParamKind::Const {
                continue;
            }
            let Some(arg) = written.get(i) else {
                return Err(RunError::err(
                    format!("missing generic argument for const parameter `{}`", p.name),
                    call.span.clone(),
                ));
            };
            let value = match arg {
                Type::Literal(text) => text.parse::<i64>().map_err(|_| {
                    RunError::err(
                        format!("`{text}` is not a const argument"),
                        call.span.clone(),
                    )
                })?,
                Type::Named(name, args) if args.is_empty() => match env.borrow().get(name) {
                    Some(Value::Number(n)) => n as i64,
                    _ => {
                        return Err(RunError::err(
                            format!("`{name}` is not a const value"),
                            call.span.clone(),
                        ));
                    }
                },
                other => {
                    return Err(RunError::err(
                        format!("{} is not a const argument", other.display()),
                        call.span.clone(),
                    ));
                }
            };
            out.push((p.name.clone(), value));
        }
        Ok(out)
    }

    /// Call a function after binding its const parameters (compile-time values
    /// that the body reads as ordinary immutable `int`s).
    fn call_function_with_consts(
        &self,
        func: &Rc<FunctionValue>,
        args: &[CallArg],
        call_env: &Rc<RefCell<Env>>,
        consts: &[(String, i64)],
    ) -> Result<Value, RunError> {
        let depth = self.call_depth.get();
        if depth >= MAX_CALL_DEPTH {
            return Err(RunError::err(
                format!("call depth exceeded: recursion limit of {MAX_CALL_DEPTH} reached"),
                0..0,
            ));
        }
        self.call_depth.set(depth + 1);
        let result = (|| {
            let bound = self.bind_args(&func.params, args, call_env)?;
            if func.is_async {
                let func = func.clone();
                let consts = consts.to_vec();
                return Ok(self.spawn_async(move |_yielder| {
                    let env = Env::child(&func.closure);
                    with_interp(|interp| {
                        for (name, value) in &consts {
                            env.borrow_mut()
                                .define(name, Value::Number(*value as f64), false);
                        }
                        interp.bind_callee_args(&env, &func.params, bound)?;
                        match interp.run_body(&func.body, &env, func.return_type.is_some()) {
                            Err(RunError::Propagate(v)) => Ok(v),
                            other => other,
                        }
                    })
                }));
            }
            let env = Env::child(&func.closure);
            for (name, value) in consts {
                env.borrow_mut()
                    .define(name, Value::Number(*value as f64), false);
            }
            self.bind_callee_args(&env, &func.params, bound)?;
            let result = if is_component_return(&func.return_type) {
                self.run_component_body(&func.body, &env)
            } else {
                self.run_body(&func.body, &env, func.return_type.is_some())
            };
            match result {
                Err(RunError::Propagate(v)) => Ok(v),
                other => other,
            }
        })();
        self.call_depth.set(depth);
        result
    }

    pub(super) fn run_function(
        &self,
        func: &Rc<FunctionValue>,
        args: Vec<Option<Value>>,
    ) -> Result<Value, RunError> {
        if func.is_async {
            let func = func.clone();
            return Ok(self.spawn_async(move |_yielder| {
                let env = Env::child(&func.closure);
                with_interp(|interp| {
                    interp.bind_callee_args(&env, &func.params, args)?;
                    // The async body's call boundary: a `?` in flight becomes
                    // this function's return value (spec `error-handling.md`).
                    match interp.run_body(&func.body, &env, func.return_type.is_some()) {
                        Err(RunError::Propagate(v)) => Ok(v),
                        other => other,
                    }
                })
            }));
        }
        let env = Env::child(&func.closure);
        self.bind_callee_args(&env, &func.params, args)?;
        let result = if is_component_return(&func.return_type) {
            self.run_component_body(&func.body, &env)
        } else {
            self.run_body(&func.body, &env, func.return_type.is_some())
        };
        // The call boundary: a `?` in flight becomes this function's return
        // value (spec `error-handling.md`) — `Result::Err` returned whole,
        // `null` for an optional operand, exactly as a `return` written at
        // that point would be.
        match result {
            Err(RunError::Propagate(v)) => Ok(v),
            other => other,
        }
    }

    pub(super) fn call_fn(
        &self,
        f: &FnDef,
        args: &[CallArg],
        call_env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        let depth = self.call_depth.get();
        if depth >= MAX_CALL_DEPTH {
            return Err(RunError::err(
                format!("call depth exceeded: recursion limit of {MAX_CALL_DEPTH} reached"),
                0..0,
            ));
        }
        self.call_depth.set(depth + 1);
        let result = (|| {
            let bound = self.bind_args(&f.params, args, call_env)?;
            if f.is_async {
                let params = f.params.clone();
                let body = f.body.clone();
                let return_type = f.return_type.clone();
                let call_env = call_env.clone();
                return Ok(self.spawn_async(move |_yielder| {
                    let env = Env::child(&call_env);
                    with_interp(|interp| {
                        interp.bind_callee_args(&env, &params, bound)?;
                        // The async body's call boundary: a `?` in flight
                        // becomes this function's return value.
                        match interp.run_body(&body, &env, return_type.is_some()) {
                            Err(RunError::Propagate(v)) => Ok(v),
                            other => other,
                        }
                    })
                }));
            }
            let env = Env::child(call_env);
            self.bind_callee_args(&env, &f.params, bound)?;
            if is_component_return(&f.return_type) {
                self.run_component_body(&f.body, &env)
            } else {
                self.run_body(&f.body, &env, f.return_type.is_some())
            }
        })();
        self.call_depth.set(depth);
        // The call boundary: a `?` in flight becomes this function's return
        // value (spec `error-handling.md`).
        match result {
            Err(RunError::Propagate(v)) => Ok(v),
            other => other,
        }
    }

    /// Execute a function body in its call environment, honoring the implicit
    /// return rule: with a declared return type, a trailing expression without
    /// a `;` is the function's value.
    pub(super) fn run_body(
        &self,
        body: &Block,
        env: &Rc<RefCell<Env>>,
        _declared_return: bool,
    ) -> Result<Value, RunError> {
        if let Some(Statement::Expr(last)) = body.statements.last()
            && !last.has_semicolon
        {
            for stmt in &body.statements[..body.statements.len() - 1] {
                match self.exec_stmt(stmt, env)? {
                    Flow::Continue | Flow::LoopContinue => {}
                    Flow::Return(v) => return Ok(v),
                    Flow::Break => return Ok(Value::Null),
                }
            }
            return self.eval(&last.expr, env);
        }
        match self.exec_stmts(&body.statements, env)? {
            Flow::Continue | Flow::LoopContinue => Ok(Value::Null),
            Flow::Return(v) => Ok(v),
            Flow::Break => Ok(Value::Null),
        }
    }

    /// Execute a component function body. Reactive declarations hoist before
    /// the render section runs, and the trailing render block is evaluated as
    /// the component's tree value (the JS output wraps the same block in
    /// `__component(() => ...)`). An explicit `return` beats the implicit
    /// render value; a trailing expression only counts without a `;`, exactly
    /// like the codegen's `return <expr>`.
    pub(super) fn run_component_body(
        &self,
        body: &Block,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        if let Some(last) = body.statements.last() {
            for stmt in &body.statements[..body.statements.len() - 1] {
                match self.exec_stmt(stmt, env)? {
                    Flow::Continue | Flow::LoopContinue => {}
                    Flow::Return(v) => return Ok(v),
                    Flow::Break => return Ok(Value::Null),
                }
            }
            match last {
                Statement::Component(block) => self.eval_component_stmt(block, env),
                Statement::Expr(es) if !es.has_semicolon => self.eval(&es.expr, env),
                other => {
                    self.exec_stmt(other, env)?;
                    Ok(Value::Null)
                }
            }
        } else {
            Ok(Value::Null)
        }
    }
}

/// Check a built-in method call's positional arity (spec
/// `types/composite-types.md`: too few or too many is `E0207`, reported by
/// the semantic pass; this is the runtime guard for the unchecked path).
fn expect_args(
    method: &str,
    args: &[CallArg],
    min: usize,
    max: usize,
    span: &std::ops::Range<usize>,
) -> Result<(), RunError> {
    if args.len() < min || args.len() > max {
        return Err(RunError::err(
            format!(
                "`{method}()` expects {min}..={max} arguments, found {}",
                args.len()
            ),
            span.clone(),
        ));
    }
    Ok(())
}

fn non_negative_index(n: f64, span: &std::ops::Range<usize>) -> Result<usize, RunError> {
    if n.is_nan() || n < 0.0 || n.fract() != 0.0 {
        return Err(RunError::err(
            format!("index must be a non-negative integer, found {n}"),
            span.clone(),
        ));
    }
    Ok(n as usize)
}

/// The `start …= len`, `start <= end` domain of `.slice` (spec
/// `types/composite-types.md`).
fn slice_bounds(
    start: f64,
    end: Option<f64>,
    len: usize,
    span: &std::ops::Range<usize>,
) -> Result<(usize, usize), RunError> {
    let s = non_negative_index(start, span)?;
    let e = match end {
        Some(end) => non_negative_index(end, span)?,
        None => len,
    };
    if s > len || e > len || s > e {
        return Err(RunError::err(
            format!("slice bounds {start}..{e} out of range for a list of length {len}"),
            span.clone(),
        ));
    }
    Ok((s, e))
}

fn map_key_string(value: &Value) -> String {
    match value {
        Value::String(s) => s.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Boolean(b) => b.to_string(),
        other => other.format(),
    }
}
