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
                let args = self.eval_args(&call.arguments, env)?;
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
            return self.call_value(&callee, &call.arguments, env);
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
                            Some(func) => self.call_value(&func, &call.arguments, env),
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
                    None => self.call_value(&callee, &call.arguments, env),
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
            _ => None,
        };
        match callee {
            Some(v) => self.call_value(&v, &call.arguments, env),
            None => {
                // A derived (or hand written) `ToString` on an enum is a
                // standalone mangled function — enum instances carry no object
                // to hang methods on — so `variant.toString()` dispatches
                // through it with the receiver bound as `self`.
                if let Value::Enum { enum_name, .. } = &receiver
                    && method == "toString"
                {
                    let mangled = impl_fn_name("ToString", enum_name, "toString");
                    if let Some(func) = env.borrow().get(&mangled) {
                        let arg_values = self.eval_args(&call.arguments, env)?;
                        let mut args = Vec::with_capacity(arg_values.len() + 1);
                        args.push(receiver.clone());
                        args.extend(arg_values);
                        return self.call_value_with_values(&func, &args);
                    }
                }
                // A constructed instance knows its struct type: the method
                // resolves on that type's object (inherent methods, a derived
                // `toString`) with the receiver bound as `self`, so two
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
            "to_uppercase" => {
                if !args.is_empty() {
                    return Err(RunError::err(
                        format!("`to_uppercase()` takes no arguments, found {}", args.len()),
                        span.clone(),
                    ));
                }
                Ok(Value::String(receiver.to_uppercase().into()))
            }
            "to_lowercase" => {
                if !args.is_empty() {
                    return Err(RunError::err(
                        format!("`to_lowercase()` takes no arguments, found {}", args.len()),
                        span.clone(),
                    ));
                }
                Ok(Value::String(receiver.to_lowercase().into()))
            }
            "trim" => {
                if !args.is_empty() {
                    return Err(RunError::err(
                        format!("`trim()` takes no arguments, found {}", args.len()),
                        span.clone(),
                    ));
                }
                Ok(Value::String(receiver.trim().into()))
            }
            "length" => {
                if !args.is_empty() {
                    return Err(RunError::err(
                        format!("`length()` takes no arguments, found {}", args.len()),
                        span.clone(),
                    ));
                }
                Ok(Value::Number(receiver.chars().count() as f64))
            }
            "contains" | "starts_with" | "ends_with" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        format!(
                            "`{method}()` requires exactly 1 argument, found {}",
                            args.len()
                        ),
                        span.clone(),
                    ));
                }
                let Value::String(needle) = self.eval(&args[0].value, env)? else {
                    return Err(RunError::err(
                        format!("`{method}()` expects a string argument"),
                        args[0].value.span().clone(),
                    ));
                };
                Ok(Value::Boolean(match method {
                    "contains" => receiver.contains(&*needle),
                    "starts_with" => receiver.starts_with(&*needle),
                    _ => receiver.ends_with(&*needle),
                }))
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
                let Value::String(sep) = self.eval(&args[0].value, env)? else {
                    return Err(RunError::err(
                        "`split()` expects a string argument",
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
            "pad_start" | "pad_end" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        format!(
                            "`{method}()` requires exactly 1 argument, found {}",
                            args.len()
                        ),
                        span.clone(),
                    ));
                }
                let width = self.eval_number_arg(&args[0], env)?;
                if width > crate::runtime::MAX_STRING_GROWTH as f64 {
                    return Err(RunError::err(
                        format!(
                            "`{method}()` width must be at most {}, found {width}",
                            crate::runtime::MAX_STRING_GROWTH
                        ),
                        args[0].value.span().clone(),
                    ));
                }
                let width = width as usize;
                let len = receiver.chars().count();
                if len >= width {
                    return Ok(Value::String(receiver));
                }
                let fill = " ".repeat(width - len);
                Ok(Value::String(if method == "pad_start" {
                    format!("{fill}{receiver}").into()
                } else {
                    format!("{receiver}{fill}").into()
                }))
            }
            "repeat" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        format!(
                            "`repeat()` requires exactly 1 argument, found {}",
                            args.len()
                        ),
                        span.clone(),
                    ));
                }
                let n = self.eval_number_arg(&args[0], env)?;
                if n > crate::runtime::MAX_STRING_GROWTH as f64 {
                    return Err(RunError::err(
                        format!(
                            "`repeat()` count must be at most {}, found {n}",
                            crate::runtime::MAX_STRING_GROWTH
                        ),
                        args[0].value.span().clone(),
                    ));
                }
                let n = n as usize;
                Ok(Value::String(receiver.repeat(n).into()))
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
            _ => Err(RunError::err(
                format!("`{method}` is not a function of a string"),
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
        match method {
            // ── basic properties ──
            "isEmpty" => Ok(Value::Boolean(list.borrow().is_empty())),
            "first" => {
                let l = list.borrow();
                Ok(l.first().cloned().unwrap_or(Value::Null))
            }
            "last" => {
                let l = list.borrow();
                Ok(l.last().cloned().unwrap_or(Value::Null))
            }

            // ── safe access & mutation ──
            "get" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "get() requires exactly 1 argument",
                        span.clone(),
                    ));
                }
                let idx = self.eval_number_arg(&args[0], env)?;
                if idx.is_nan() || idx < 0.0 || idx.fract() != 0.0 {
                    return Err(RunError::err(
                        format!("index must be a non-negative integer, found {idx}"),
                        span.clone(),
                    ));
                }
                let i = idx as usize;
                if i >= list.borrow().len() {
                    Ok(Value::Null)
                } else {
                    Ok(list.borrow()[i].clone())
                }
            }
            "set" => {
                if args.len() != 2 {
                    return Err(RunError::err(
                        "set() requires exactly 2 arguments",
                        span.clone(),
                    ));
                }
                let idx = self.eval_number_arg(&args[0], env)?;
                let val = self.eval(&args[1].value, env)?;
                let i = self.list_index_bound(idx, list.borrow().len(), span)?;
                list.borrow_mut()[i] = val;
                Ok(Value::List(list))
            }
            "push" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "push() requires exactly 1 argument",
                        span.clone(),
                    ));
                }
                let val = self.eval(&args[0].value, env)?;
                list.borrow_mut().push(val);
                Ok(Value::List(list))
            }
            "pop" => {
                if !args.is_empty() {
                    return Err(RunError::err("pop() takes no arguments", span.clone()));
                }
                let val = list
                    .borrow_mut()
                    .pop()
                    .ok_or_else(|| RunError::err("pop() called on an empty list", span.clone()))?;
                Ok(val)
            }
            "insert" => {
                if args.len() != 2 {
                    return Err(RunError::err(
                        "insert() requires exactly 2 arguments",
                        span.clone(),
                    ));
                }
                let idx = self.eval_number_arg(&args[0], env)?;
                let val = self.eval(&args[1].value, env)?;
                let i = self.list_index_bound(idx, list.borrow().len(), span)?;
                list.borrow_mut().insert(i, val);
                Ok(Value::List(list))
            }
            "remove" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "remove() requires exactly 1 argument",
                        span.clone(),
                    ));
                }
                let idx = self.eval_number_arg(&args[0], env)?;
                let i = self.list_index_bound(idx, list.borrow().len(), span)?;
                Ok(list.borrow_mut().remove(i))
            }
            "removeValue" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "removeValue() requires exactly 1 argument",
                        span.clone(),
                    ));
                }
                let val = self.eval(&args[0].value, env)?;
                let mut l = list.borrow_mut();
                if let Some(pos) = l.iter().position(|v| equal(v, &val)) {
                    l.remove(pos);
                    Ok(Value::Boolean(true))
                } else {
                    Ok(Value::Boolean(false))
                }
            }

            // ── slicing & views ──
            "slice" => {
                if args.len() > 2 {
                    return Err(RunError::err(
                        "slice() takes at most 2 arguments",
                        span.clone(),
                    ));
                }
                let l = list.borrow();
                let len = l.len() as i64;
                let start = if args.is_empty() {
                    0i64
                } else {
                    self.eval_number_arg(&args[0], env)? as i64
                };
                let end = if args.len() < 2 {
                    len
                } else {
                    self.eval_number_arg(&args[1], env)? as i64
                };
                let s = start.max(0).min(len) as usize;
                let e = end.max(0).min(len) as usize;
                Ok(Value::List(Rc::new(RefCell::new(l[s..e].to_vec()))))
            }
            "indexOf" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "indexOf() requires exactly 1 argument",
                        span.clone(),
                    ));
                }
                let val = self.eval(&args[0].value, env)?;
                let l = list.borrow();
                match l.iter().position(|v| equal(v, &val)) {
                    Some(i) => Ok(Value::Number(i as f64)),
                    None => Ok(Value::Number(-1.0)),
                }
            }
            "contains" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "contains() requires exactly 1 argument",
                        span.clone(),
                    ));
                }
                let val = self.eval(&args[0].value, env)?;
                let l = list.borrow();
                Ok(Value::Boolean(l.iter().any(|v| equal(v, &val))))
            }
            "join" => {
                if args.len() > 1 {
                    return Err(RunError::err(
                        "join() takes at most 1 argument",
                        span.clone(),
                    ));
                }
                let sep = if args.is_empty() {
                    String::new()
                } else {
                    match self.eval(&args[0].value, env)? {
                        Value::String(s) => s.to_string(),
                        v => v.format(),
                    }
                };
                let l = list.borrow();
                let parts: Vec<String> = l.iter().map(|v| v.format()).collect();
                Ok(Value::String(parts.join(&sep).into()))
            }
            "reverse" => {
                if !args.is_empty() {
                    return Err(RunError::err("reverse() takes no arguments", span.clone()));
                }
                list.borrow_mut().reverse();
                Ok(Value::List(list))
            }
            "sort" => {
                if !args.is_empty() {
                    return Err(RunError::err("sort() takes no arguments", span.clone()));
                }
                list.borrow_mut().sort_by(|a, b| match (a, b) {
                    (Value::Number(x), Value::Number(y)) => {
                        x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
                    }
                    (Value::String(x), Value::String(y)) => x.cmp(y),
                    _ => std::cmp::Ordering::Equal,
                });
                Ok(Value::List(list))
            }
            "concat" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "concat() requires exactly 1 argument",
                        span.clone(),
                    ));
                }
                let other = self.eval(&args[0].value, env)?;
                match other {
                    Value::List(other_list) => {
                        let mut out = list.borrow().clone();
                        out.extend(other_list.borrow().iter().cloned());
                        Ok(Value::List(Rc::new(RefCell::new(out))))
                    }
                    _ => Err(RunError::err(
                        format!("concat() expected a list, found {}", other.kind_name()),
                        span.clone(),
                    )),
                }
            }
            "flat" => {
                if !args.is_empty() {
                    return Err(RunError::err("flat() takes no arguments", span.clone()));
                }
                let mut out = Vec::new();
                for item in list.borrow().iter() {
                    match item {
                        Value::List(inner) => {
                            out.extend(inner.borrow().iter().cloned());
                        }
                        other => out.push(other.clone()),
                    }
                }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            "shrink" => {
                if !args.is_empty() {
                    return Err(RunError::err("shrink() takes no arguments", span.clone()));
                }
                list.borrow_mut().shrink_to_fit();
                Ok(Value::List(list))
            }
            "clear" => {
                if !args.is_empty() {
                    return Err(RunError::err("clear() takes no arguments", span.clone()));
                }
                list.borrow_mut().clear();
                Ok(Value::List(list))
            }

            // ── higher-order functions ──
            "map" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "map() requires exactly 1 argument (a function)",
                        span.clone(),
                    ));
                }
                let f_val = self.eval(&args[0].value, env)?;
                let l = list.borrow();
                let mut out = Vec::with_capacity(l.len());
                for (i, item) in l.iter().enumerate() {
                    let result = self
                        .call_value_with_values(&f_val, &[item.clone(), Value::Number(i as f64)])?;
                    out.push(result);
                }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            "filter" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "filter() requires exactly 1 argument (a function)",
                        span.clone(),
                    ));
                }
                let f_val = self.eval(&args[0].value, env)?;
                let l = list.borrow();
                let mut out = Vec::new();
                for item in l.iter() {
                    let keep = self.call_value_with_values(&f_val, std::slice::from_ref(item))?;
                    if is_truthy(&keep) {
                        out.push(item.clone());
                    }
                }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            "reduce" => {
                if args.is_empty() || args.len() > 2 {
                    return Err(RunError::err(
                        "reduce() requires 1 or 2 arguments",
                        span.clone(),
                    ));
                }
                let f_val = self.eval(&args[0].value, env)?;
                let l = list.borrow();
                let has_init = args.len() == 2;
                let mut acc = if has_init {
                    self.eval(&args[1].value, env)?
                } else if l.is_empty() {
                    return Err(RunError::err(
                        "reduce() called on an empty list without initial value",
                        span.clone(),
                    ));
                } else {
                    l[0].clone()
                };
                let start = if has_init { 0 } else { 1 };
                for item in &l[start..] {
                    let new_acc = self.call_value_with_values(&f_val, &[acc, item.clone()])?;
                    acc = new_acc;
                }
                Ok(acc)
            }
            "find" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "find() requires exactly 1 argument (a function)",
                        span.clone(),
                    ));
                }
                let f_val = self.eval(&args[0].value, env)?;
                let l = list.borrow();
                for item in l.iter() {
                    let found = self.call_value_with_values(&f_val, std::slice::from_ref(item))?;
                    if is_truthy(&found) {
                        return Ok(item.clone());
                    }
                }
                Ok(Value::Null)
            }
            "some" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "some() requires exactly 1 argument (a function)",
                        span.clone(),
                    ));
                }
                let f_val = self.eval(&args[0].value, env)?;
                let l = list.borrow();
                for item in l.iter() {
                    let result = self.call_value_with_values(&f_val, std::slice::from_ref(item))?;
                    if is_truthy(&result) {
                        return Ok(Value::Boolean(true));
                    }
                }
                Ok(Value::Boolean(false))
            }
            "every" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "every() requires exactly 1 argument (a function)",
                        span.clone(),
                    ));
                }
                let f_val = self.eval(&args[0].value, env)?;
                let l = list.borrow();
                for item in l.iter() {
                    let result = self.call_value_with_values(&f_val, std::slice::from_ref(item))?;
                    if !is_truthy(&result) {
                        return Ok(Value::Boolean(false));
                    }
                }
                Ok(Value::Boolean(true))
            }
            "forEach" => {
                if args.len() != 1 {
                    return Err(RunError::err(
                        "forEach() requires exactly 1 argument (a function)",
                        span.clone(),
                    ));
                }
                let f_val = self.eval(&args[0].value, env)?;
                let l = list.borrow();
                for (i, item) in l.iter().enumerate() {
                    self.call_value_with_values(&f_val, &[item.clone(), Value::Number(i as f64)])?;
                }
                Ok(Value::Null)
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

    pub(super) fn list_index_bound(
        &self,
        n: f64,
        len: usize,
        span: &std::ops::Range<usize>,
    ) -> Result<usize, RunError> {
        if n.is_nan() || n < 0.0 || n.fract() != 0.0 {
            return Err(RunError::err(
                format!("index must be a non-negative integer, found {n}"),
                span.clone(),
            ));
        }
        let i = n as usize;
        if i > len {
            return Err(RunError::err(
                format!("index {n} out of bounds for a list of length {len}"),
                span.clone(),
            ));
        }
        Ok(i)
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
