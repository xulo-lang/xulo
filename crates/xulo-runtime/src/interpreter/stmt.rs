use super::*;

/// Bind a value to a declaration whose annotation names an unsigned 64-bit
/// integer. The numeric tower narrows a `u64` that fits `i64` to `int` for
/// ordinary arithmetic; an explicit annotation must not lose that domain.
pub(super) fn coerce_binding_type(annotation: &Option<Type>, value: Value) -> Value {
    let f32_annotation = matches!(
        annotation,
        Some(Type::Named(name, args)) if args.is_empty() && name == "f32"
    );
    if f32_annotation {
        return match value {
            Value::Float(n) => Value::Float32(n as f32),
            Value::Float32(n) => Value::Float32(n),
            Value::Int(n) => Value::Float32(n as f32),
            Value::UInt(n) => Value::Float32(n as f32),
            other => other,
        };
    }
    let unsigned = matches!(
        annotation,
        Some(Type::Named(name, args))
            if args.is_empty() && matches!(name.as_str(), "u64" | "usize")
    );
    if unsigned
        && let Value::Int(n) = value
        && n >= 0
    {
        return Value::UInt(n as u64);
    }
    value
}

impl Interpreter {
    pub(super) fn exec_stmt(
        &self,
        stmt: &Statement,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        self.bump_step(&(0..0))?;
        match stmt {
            Statement::Fn(f) => {
                self.register_fn(f, env);
                Ok(Flow::Continue)
            }
            // A `#[test] fn` body is inert (spec testing.md): the identifier
            // binds no value and no execution reaches the body, so the
            // declaration is a no-op at run time.
            Statement::Test(_) => Ok(Flow::Continue),
            Statement::Let(binding) => {
                let value = match &binding.value {
                    Some(expr) => self.eval(expr, env)?,
                    None => Value::Null,
                };
                // An explicit `u64` annotation keeps the unsigned slot even
                // when the value fits `i64`; otherwise `x + 1` would narrow
                // back to `int` and overflow at `i64::MAX`
                // (spec `types/primitive-types.md`).
                let value = coerce_binding_type(&binding.type_annotation, value);
                // `Array<T, N>` shares the list representation; an annotated
                // array is registered so later bindings copy it (R8). An object
                // literal annotated with a struct type is a struct instance,
                // not a `Map`.
                let value = if let Some(Type::Named(name, _)) = &binding.type_annotation {
                    if name == "Array" {
                        if let Value::List(list) = &value {
                            self.mark_value_list(list);
                        }
                    } else if self.struct_names.borrow().iter().any(|known| known == name)
                        && let Value::Object(object) = &value
                    {
                        self.register_instance(object, name);
                    }
                    value
                } else {
                    value
                };
                // `let x = e` binds by value/reference rule: aggregates in the
                // value category get their own container.
                let value = self.value_bind_copy(value);
                // Tuple destructuring: `let (a, b, c) = expr`
                if let Some(tuple_names) = &binding.tuple_names {
                    match value {
                        Value::List(list) => {
                            let l = list.borrow();
                            if l.len() != tuple_names.len() {
                                return Err(RunError::err(
                                    format!(
                                        "tuple destructuring expected {} elements, found {}",
                                        tuple_names.len(),
                                        l.len()
                                    ),
                                    binding.name_span.clone(),
                                ));
                            }
                            for (name, item) in tuple_names.iter().zip(l.iter()) {
                                let item = self.value_bind_copy(item.clone());
                                env.borrow_mut().define(name, item, binding.is_mutable);
                            }
                        }
                        other => {
                            return Err(RunError::err(
                                format!(
                                    "cannot destructure {} as a tuple (expected a list)",
                                    other.kind_name()
                                ),
                                binding.name_span.clone(),
                            ));
                        }
                    }
                // Object destructuring: `let { name, age: user_age } = expr`
                } else if let Some(fields) = &binding.object_destructuring {
                    match value {
                        Value::Object(obj) => {
                            let o = obj.borrow();
                            for (field_name, alias) in fields {
                                let bind_name = alias.as_deref().unwrap_or(field_name);
                                let val = o
                                    .iter()
                                    .find(|(k, _)| k == field_name)
                                    .map(|(_, v)| v.clone())
                                    .unwrap_or(Value::Null);
                                let val = self.value_bind_copy(val);
                                env.borrow_mut().define(bind_name, val, binding.is_mutable);
                            }
                        }
                        other => {
                            return Err(RunError::err(
                                format!(
                                    "cannot destructure {} as an object (expected an object)",
                                    other.kind_name()
                                ),
                                binding.name_span.clone(),
                            ));
                        }
                    }
                } else {
                    env.borrow_mut()
                        .define(&binding.name, value, binding.is_mutable);
                }
                Ok(Flow::Continue)
            }
            Statement::Return(r) => {
                let value = match &r.value {
                    Some(expr) => self.eval(expr, env)?,
                    None => Value::Null,
                };
                Ok(Flow::Return(value))
            }
            Statement::For(f) => self.exec_for(f, env),
            Statement::While(w) => {
                loop {
                    let cond = self.eval(&w.condition, env)?;
                    if !is_truthy(&cond) {
                        break;
                    }
                    match self.exec_block(&w.body, env)? {
                        Flow::Continue | Flow::LoopContinue => {}
                        Flow::Break => break,
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Continue)
            }
            Statement::Assign(assign) => self.exec_assign(assign, env),
            Statement::TypeAlias(_) => Ok(Flow::Continue),
            Statement::Enum(e) => {
                // Register the enum's runtime value so `pub use { Color }` and
                // `pub enum` both resolve it for importers (construction
                // `Color::Red` is name-based and unaffected), and record the
                // name so `Color::Red` stays variant construction rather than
                // a namespace read.
                if !self.enum_names.borrow().iter().any(|n| n == &e.name) {
                    self.enum_names.borrow_mut().push(e.name.clone());
                }
                env.borrow_mut().define(&e.name, enum_value(e), false);
                Ok(Flow::Continue)
            }
            Statement::Expr(es) => {
                // `if` in statement position runs its branches as statements so
                // a `return` inside a branch returns from the enclosing
                // function (matching the JS codegen, which emits a real `if`
                // statement). A trailing no-semicolon `if` used as a function's
                // implicit return still takes the value path (`run_body` evals
                // it directly, never through here).
                if let Expression::If(if_expr) = &es.expr {
                    self.exec_if_stmt(if_expr, env)
                } else {
                    self.eval(&es.expr, env)?;
                    Ok(Flow::Continue)
                }
            }
            Statement::Block(block) => self.exec_block(block, env),
            Statement::Try(t) => self.exec_try(t, env),
            Statement::Throw(expr) => {
                let value = self.eval(expr, env)?;
                Err(RunError::Throw(value))
            }
            // `mod` claims/nests a module (its body runs as a separate
            // module, earlier in the structural init order) and `use` only
            // moves names — the CLI binds both before statements run.
            Statement::Mod(_) | Statement::Use(_) => Ok(Flow::Continue),
            Statement::Export(export) => self.exec_export(&export.item, env),
            Statement::Trait(_) => Ok(Flow::Continue),
            Statement::Impl(imp) => {
                self.register_impl(imp, env);
                Ok(Flow::Continue)
            }
            Statement::Break => Ok(Flow::Break),
            Statement::Continue => Ok(Flow::LoopContinue),
            Statement::Defer(d) => {
                // Register only; the operand runs when this block is left
                // (`exec_block`/`run_body`/`block_value` drain the scope's
                // queue in reverse at exit).
                env.borrow().push_defer(d.body.clone());
                Ok(Flow::Continue)
            }
            Statement::Struct(_) => {
                // Struct name was already registered in the first pass of
                // run()/setup_module(); skip re-registration in exec_stmt to
                // avoid wiping out inherent impl methods added in that pass.
                Ok(Flow::Continue)
            }
            // Extern imports are registered with their native call bridge
            // before any statement runs (spec ffi.md); nothing to exec here.
            Statement::ExternBlock(_) => Ok(Flow::Continue),
        }
    }

    pub(super) fn exec_export(
        &self,
        item: &xulo_core::ast::ExportItem,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        match item {
            xulo_core::ast::ExportItem::Fn(f) => {
                self.register_fn(f, env);
                Ok(Flow::Continue)
            }
            xulo_core::ast::ExportItem::Let(binding) => {
                self.exec_stmt(&Statement::Let(binding.clone()), env)
            }
            // `pub enum` reaches the same registration a bare `enum` does:
            // env value for importers, and the name for `E::V` construction.
            xulo_core::ast::ExportItem::Enum(e) => self.exec_stmt(&Statement::Enum(e.clone()), env),
            xulo_core::ast::ExportItem::Type(_)
            | xulo_core::ast::ExportItem::Struct(_)
            | xulo_core::ast::ExportItem::Trait(_)
            | xulo_core::ast::ExportItem::Names(_) => Ok(Flow::Continue),
        }
    }

    /// Run `defer` operands registered in `env`, last registered first, and
    /// then return `result` unchanged. A normal exit and a `?` propagation
    /// both leave the block, so both drain; a panic (`Throw`) or fatal runtime
    /// error abandons every defer whose operand has not begun (spec
    /// `statements/return-and-block.md`).
    pub(super) fn leave_block<T>(
        &self,
        env: &Rc<RefCell<Env>>,
        result: Result<T, RunError>,
    ) -> Result<T, RunError> {
        match result {
            Ok(value) => {
                self.run_defers(env)?;
                Ok(value)
            }
            Err(RunError::Propagate(value)) => {
                self.run_defers(env)?;
                Err(RunError::Propagate(value))
            }
            Err(e) => Err(e),
        }
    }

    fn run_defers(&self, env: &Rc<RefCell<Env>>) -> Result<(), RunError> {
        let defers = env.borrow().take_defers();
        for body in defers.into_iter().rev() {
            match body {
                xulo_core::ast::DeferBody::Block(block) => {
                    self.exec_block(&block, env)?;
                }
                xulo_core::ast::DeferBody::Expr(expr) => {
                    self.eval(&expr, env)?;
                }
            }
        }
        Ok(())
    }

    pub(super) fn exec_block(
        &self,
        block: &Block,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        let child = Env::child(env);
        self.leave_block(&child, self.exec_stmts(&block.statements, &child))
    }

    pub(super) fn exec_stmts(
        &self,
        stmts: &[Statement],
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        for stmt in stmts {
            match self.exec_stmt(stmt, env)? {
                Flow::Continue => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Continue)
    }

    pub(super) fn exec_for(&self, f: &ForStmt, env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
        // Recycle one iteration scope across rounds when no closure can observe
        // it (P3). A fresh scope per round is mandatory whenever the body
        // declares or captures a function (see `block_has_closure`): recycled
        // environments would make every closure read the *final* loop variable.
        let recycle = !block_has_closure(&f.body);
        if let Expression::Range(r) = &f.iterable {
            let mut cursor = self.range_cursor(r, env)?;
            if recycle {
                let child = Env::child(env);
                while let Some(value) = cursor.next() {
                    child
                        .borrow_mut()
                        .reset(&f.iter_var, self.value_bind_copy(value), true);
                    match self.leave_block(&child, self.exec_stmts(&f.body.statements, &child))? {
                        Flow::Continue | Flow::LoopContinue => {}
                        Flow::Break => break,
                        other => return Ok(other),
                    }
                }
            } else {
                while let Some(value) = cursor.next() {
                    let child = Env::child(env);
                    child
                        .borrow_mut()
                        .define(&f.iter_var, self.value_bind_copy(value), true);
                    match self.exec_block(&f.body, &child)? {
                        Flow::Continue | Flow::LoopContinue => {}
                        Flow::Break => break,
                        other => return Ok(other),
                    }
                }
            }
            Ok(Flow::Continue)
        } else {
            let iterable = self.eval(&f.iterable, env)?;
            // A `Map` iterates its keys in insertion order and a `Set` its
            // elements (spec `types/composite-types.md`); both are snapshotted,
            // while a `List` keeps its live-borrow iteration below.
            let iterable = match iterable {
                Value::Object(fields) => Value::List(Rc::new(RefCell::new(
                    fields
                        .borrow()
                        .iter()
                        .map(|(k, _)| Value::String(k.clone().into()))
                        .collect(),
                ))),
                Value::Set(set) => Value::List(Rc::new(RefCell::new(set.borrow().clone()))),
                other => other,
            };
            let list = match iterable {
                Value::List(list) => list,
                other => {
                    return Err(RunError::err(
                        format!(
                            "for loop must iterate over a `List`, `Map`, or `Set`, found {}",
                            other.kind_name()
                        ),
                        f.iterable.span().clone(),
                    ));
                }
            };
            // Iterate *live*, like JS `for...of`: the length and each element
            // are re-read on every step, so mutations in the body (appending,
            // replacing, shortening) are visible to later iterations. A
            // snapshot used to diverge (`xs[1] = 99` inside the body read the
            // old value here but the new one in JS).
            let mut i = 0usize;
            if recycle {
                let child = Env::child(env);
                loop {
                    let item = {
                        let list = list.borrow();
                        if i >= list.len() {
                            break;
                        }
                        list[i].clone()
                    };
                    child
                        .borrow_mut()
                        .reset(&f.iter_var, self.value_bind_copy(item), true);
                    match self.leave_block(&child, self.exec_stmts(&f.body.statements, &child))? {
                        Flow::Continue | Flow::LoopContinue => {}
                        Flow::Break => break,
                        other => return Ok(other),
                    }
                    i += 1;
                }
            } else {
                loop {
                    let item = {
                        let list = list.borrow();
                        if i >= list.len() {
                            break;
                        }
                        list[i].clone()
                    };
                    let child = Env::child(env);
                    child
                        .borrow_mut()
                        .define(&f.iter_var, self.value_bind_copy(item), true);
                    match self.exec_block(&f.body, &child)? {
                        Flow::Continue | Flow::LoopContinue => {}
                        Flow::Break => break,
                        other => return Ok(other),
                    }
                    i += 1;
                }
            }
            Ok(Flow::Continue)
        }
    }

    /// Whether a loop body could let a closure observe the loop's iteration scope.
    /// `for` creates a fresh environment per round so an `fn` in the body captures
    /// *that round's* loop variable (JS `let` semantics). When the body contains no
    /// closures the fresh scope is unobservable, and one recycled environment
    /// (reset to just the loop variable each round) is equivalent — that is what
    /// `exec_for` uses to avoid allocating per iteration (P3). Any `fn` — a
    /// declaration, an anonymous expression, a nested loop/if body, or an
    /// `impl` method — forces the per-round path instead.
    pub(super) fn exec_assign(
        &self,
        a: &xulo_core::ast::AssignStmt,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        self.exec_assign_value(a, env)?;
        Ok(Flow::Continue)
    }

    /// Run an assignment and return the value it assigned, so a chained
    /// right-hand side (`a = b = c`) can feed the outer target.
    pub(super) fn exec_assign_value(
        &self,
        a: &xulo_core::ast::AssignStmt,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        let value = match &a.value {
            AssignValue::Expr(expr) => self.eval(expr, env)?,
            AssignValue::Assign(inner) => self.exec_assign_value(inner, env)?,
        };
        // Assignment follows the same value/reference rule as `let` (R8).
        let value = self.value_bind_copy(value);
        let result = value.clone();
        match &a.target {
            AssignTarget::Name(name) => {
                // Check if variable is mutable before assignment.
                if !env.borrow().is_mutable(name) {
                    return Err(RunError::err(
                        format!("cannot assign to immutable variable `{name}`"),
                        a.span.clone(),
                    ));
                }
                if !env.borrow_mut().assign(name, value) {
                    return Err(RunError::err(
                        format!("undefined variable `{name}` cannot be assigned"),
                        a.span.clone(),
                    ));
                }
            }
            AssignTarget::Member(object, property) => {
                let obj = self.eval(object, env)?;
                match obj {
                    // `p.0 = v` on a tuple (a list at run time).
                    Value::List(list) if property.parse::<usize>().is_ok() => {
                        let index = property.parse::<usize>().expect("checked");
                        let mut list = list.borrow_mut();
                        if index < list.len() {
                            list[index] = value;
                        } else {
                            return Err(RunError::err(
                                format!("position {index} does not exist in the tuple"),
                                a.span.clone(),
                            ));
                        }
                    }
                    Value::Object(fields) => {
                        let field_ty = self.instance_type(&fields).and_then(|name| {
                            let table = self.struct_fields.borrow();
                            table.get(&name).and_then(|decl| {
                                decl.fields
                                    .iter()
                                    .find(|(fname, _, _)| fname == property)
                                    .and_then(|(_, _, ty)| ty.clone())
                            })
                        });
                        let value = super::stmt::coerce_binding_type(&field_ty, value);
                        let mut fields = fields.borrow_mut();
                        if let Some((_, slot)) = fields.iter_mut().find(|(k, _)| k == property) {
                            *slot = value;
                        } else {
                            fields.push((property.clone(), value));
                        }
                    }
                    other => {
                        return Err(RunError::err(
                            format!("cannot assign member `{property}` of {}", other.kind_name()),
                            a.span.clone(),
                        ));
                    }
                }
            }
            AssignTarget::Index(object, index) => {
                let obj = self.eval(object, env)?;
                let idx = self.eval(index, env)?;
                match (obj, &idx) {
                    (Value::List(list), _) if idx.num().is_some() => {
                        let i = list_index(idx.num().expect("checked"), &a.span)?;
                        let mut list = list.borrow_mut();
                        if i < list.len() {
                            list[i] = value;
                        } else {
                            return Err(RunError::err(
                                format!(
                                    "index {i} out of bounds for a list of length {}",
                                    list.len()
                                ),
                                a.span.clone(),
                            ));
                        }
                    }
                    (Value::Object(fields), Value::String(key)) => {
                        let mut fields = fields.borrow_mut();
                        if let Some((_, slot)) =
                            fields.iter_mut().find(|(k, _)| k.as_str() == key.as_ref())
                        {
                            *slot = value;
                        } else {
                            fields.push((key.to_string(), value));
                        }
                    }
                    (other, i) => {
                        return Err(RunError::err(
                            format!(
                                "cannot assign into {} by index {}",
                                other.kind_name(),
                                i.format()
                            ),
                            a.span.clone(),
                        ));
                    }
                }
            }
        }
        Ok(result)
    }

    pub(super) fn exec_try(&self, t: &TryStmt, env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
        match self.exec_block(&t.try_block, env) {
            Ok(flow) => Ok(flow),
            Err(RunError::Throw(value)) => {
                let child = Env::child(env);
                child.borrow_mut().define(&t.catch_var, value, true);
                self.exec_block(&t.catch_block, &child)
            }
            Err(e) => Err(e),
        }
    }

    /// Run an `if` in statement position: execute the chosen branch as a
    /// statement block, propagating an early `return` out of the function.
    pub(super) fn exec_if_stmt(
        &self,
        if_expr: &IfExpr,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        let cond = self.eval(&if_expr.condition, env)?;
        if is_truthy(&cond) {
            self.exec_block(&if_expr.then_branch, env)
        } else if let Some(else_branch) = &if_expr.else_branch {
            self.exec_block(else_branch, env)
        } else {
            Ok(Flow::Continue)
        }
    }
}
