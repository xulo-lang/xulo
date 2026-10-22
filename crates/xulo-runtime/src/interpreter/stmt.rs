use super::*;

impl Interpreter {
    pub(super) fn exec_stmt(
        &self,
        stmt: &Statement,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        match stmt {
            Statement::Fn(f) => {
                self.register_fn(f, env);
                Ok(Flow::Continue)
            }
            // A `#Test fn` body is inert (spec testing.md): the identifier
            // binds no value and no execution reaches the body, so the
            // declaration is a no-op at run time.
            Statement::Test(_) => Ok(Flow::Continue),
            Statement::Let(binding) => {
                let value = if binding.memo {
                    self.memo_value(binding, env)?
                } else {
                    match &binding.value {
                        Some(expr) => self.eval(expr, env)?,
                        None => Value::Null,
                    }
                };
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
                                env.borrow_mut()
                                    .define(name, item.clone(), binding.is_mutable);
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
            Statement::State(s) => self.exec_state(&s.binding, env),
            Statement::Store(s) => self.exec_store(s, env),
            Statement::Effect(e) => self.exec_effect(e, env),
            Statement::Environment(_) => Err(RunError::err(
                "@Environment is not supported in the native runtime (no provider mechanism)",
                0..0,
            )),
            Statement::Component(c) => {
                // In a plain body a component block renders into the tree and
                // its value is only used when it is a component function's
                // trailing statement (see `run_component_body`).
                self.eval_component_stmt(c, env)?;
                Ok(Flow::Continue)
            }
            Statement::Trait(_) => Ok(Flow::Continue),
            Statement::Impl(imp) => {
                self.register_impl(imp, env);
                Ok(Flow::Continue)
            }
            Statement::Break => Ok(Flow::Break),
            Statement::Continue => Ok(Flow::LoopContinue),
            Statement::Defer(d) => {
                // TODO(defer): real defer runs at scope exit; until then the
                // body executes immediately (parsed/checked, not yet queued).
                match &d.body {
                    xulo_core::ast::DeferBody::Block(b) => {
                        self.exec_block(b, env)?;
                    }
                    xulo_core::ast::DeferBody::Expr(e) => {
                        self.eval(e, env)?;
                    }
                }
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

    /// `@Memo let x = expr` value memoization: evaluate the deps, and reuse the
    /// cached result for this binding site when the deps are equal to a prior
    /// evaluation (`@Memo([])` / bare `@Memo` always reuse). The cache lives
    /// for the whole run and is a computation optimization only.
    pub(super) fn memo_value(
        &self,
        binding: &LetBinding,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        let deps: Vec<Value> = binding
            .memo_deps
            .iter()
            .flatten()
            .map(|dep| self.eval(dep, env))
            .collect::<Result<_, _>>()?;
        let key = binding.name_span.start;
        let mut cache = self.memo_cache.borrow_mut();
        let entries = cache.entry(key).or_default();
        for (cached_deps, value) in entries.iter() {
            if cached_deps.len() == deps.len()
                && cached_deps.iter().zip(&deps).all(|(a, b)| equal(a, b))
            {
                return Ok(value.clone());
            }
        }
        let expr = binding
            .value
            .as_ref()
            .expect("checked: memo has an initializer");
        let value = self.eval(expr, env)?;
        entries.push((deps, value.clone()));
        Ok(value)
    }

    /// `@State let x = v` defines a reactive cell (the JS output is
    /// `const x = __signal(v)`); writes later rewrite into the cell. A missing
    /// initializer starts as `null`, mirroring `__signal(undefined)`.
    ///
    /// Cells are kept in [`Interpreter::state_store`] keyed by binding site, so
    /// a re-render (see [`Interpreter::rerender_main`]) reuses the existing
    /// cell and the initializer is not re-run — `@State` values survive across
    /// renders.
    pub(super) fn exec_state(
        &self,
        binding: &LetBinding,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        let key = binding.name_span.start;
        let cell: Rc<RefCell<Value>> = if let Some(existing) = self.state_store.borrow().get(&key) {
            existing.clone()
        } else {
            let value = match &binding.value {
                Some(value) => self.eval(value, env)?,
                None => Value::Null,
            };
            let cell = Rc::new(RefCell::new(value));
            self.state_store.borrow_mut().insert(key, cell.clone());
            cell
        };
        env.borrow_mut()
            .define(&binding.name, Value::Signal(cell), binding.is_mutable);
        Ok(Flow::Continue)
    }

    /// `@Store { a, b } = expr` is a plain value binding in the codegen
    /// (`const { a, b } = expr;`): a single name binds the whole value and a
    /// destructure reads the object members. No signal is created.
    pub(super) fn exec_store(
        &self,
        store: &StoreStmt,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        let value = self.eval(&store.value, env)?;
        match &store.pattern {
            BindingPattern::Ident(name) => {
                env.borrow_mut().define(name, value, false);
            }
            BindingPattern::Destructure(fields) => {
                for (key, alias) in fields {
                    let member = match &value {
                        Value::Object(fields_map) => fields_map
                            .borrow()
                            .iter()
                            .find(|(k, _)| k == key)
                            .map(|(_, v)| v.clone())
                            .unwrap_or(Value::Null),
                        _ => Value::Null,
                    };
                    let binding_name = alias.clone().unwrap_or_else(|| key.clone());
                    env.borrow_mut().define(&binding_name, member, false);
                }
            }
        }
        Ok(Flow::Continue)
    }

    /// `@Effect` runs its closure once with the current environment. The JS
    /// path registers an effect that runs on render; natively there is no
    /// subscription model, so the body executes exactly once (documented).
    pub(super) fn exec_effect(
        &self,
        effect: &EffectStmt,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        let closure = &effect.closure;
        let func = Value::Function(Rc::new(FunctionValue {
            params: closure.params.clone(),
            body: closure.body.clone(),
            return_type: closure.return_type.clone(),
            is_async: closure.is_async,
            closure: env.clone(),
        }));
        self.call_value(&func, &[], env)?;
        Ok(Flow::Continue)
    }

    /// A component block inside a component body (`Button { "hi" }`) builds a
    /// tree node in the render array. The value is only *used* when the block
    /// is the trailing statement of a component function (see
    /// `run_component_body`), which is the implicit-return contract.
    pub(super) fn eval_component_stmt(
        &self,
        c: &ComponentStmt,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        self.local_component_call(&c.name, &c.args, &c.children, env)
    }

    /// Evaluate a single UI element into a renderable value.
    pub(super) fn eval_ui_element(
        &self,
        el: &UiElement,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        match el {
            UiElement::Component(c) => self.eval_component_stmt(c, env),
            UiElement::Text(text) => Ok(Value::String(Rc::from(text.as_str()))),
            UiElement::Expr(expr) => self.eval(expr, env),
            UiElement::If {
                condition,
                then_branch,
                else_branch,
            } => {
                if is_truthy(&self.eval(condition, env)?) {
                    self.ui_children_value(Some(then_branch), env)
                } else if let Some(els) = else_branch {
                    self.ui_children_value(Some(els), env)
                } else {
                    Ok(Value::Null)
                }
            }
            UiElement::For {
                iter_var,
                iterable,
                body,
                ..
            } => {
                let iterable = self.eval(iterable, env)?;
                let items = match iterable {
                    Value::List(list) => list,
                    Value::Signal(cell) => match cell.borrow().clone() {
                        Value::List(list) => list,
                        other => {
                            return Err(RunError::err(
                                format!(
                                    "for loop must iterate over a `List`, found {}",
                                    other.kind_name()
                                ),
                                0..0,
                            ));
                        }
                    },
                    other => {
                        return Err(RunError::err(
                            format!(
                                "for loop must iterate over a `List`, found {}",
                                other.kind_name()
                            ),
                            0..0,
                        ));
                    }
                };
                let child = Env::child(env);
                let mut acc = Vec::with_capacity(items.borrow().len());
                for v in items.borrow().iter().cloned() {
                    child.borrow_mut().define(iter_var, v, true);
                    acc.push(self.ui_children_value(Some(body), &child)?);
                }
                Ok(Value::List(Rc::new(RefCell::new(acc))))
            }
            UiElement::Group(elements) => self.ui_children_value(Some(elements), env),
        }
    }

    /// Render `children` into an array. A single element stays scalar in the
    /// JS output (`<View>{x}</View>` with one child emits a bare expression);
    /// multiple elements are collected into a list so `for`/`if` blocks can
    /// spread into it. An empty children body stays `null` (the default-slot
    /// shape), matching the codegen's `children` prop.
    pub(super) fn ui_children_value(
        &self,
        children: Option<&[UiElement]>,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        match children {
            None | Some([]) => Ok(Value::Null),
            Some([single]) => self.eval_ui_element(single, env),
            Some(multi) => {
                let mut acc = Vec::with_capacity(multi.len());
                for el in multi {
                    acc.push(self.eval_ui_element(el, env)?);
                }
                Ok(Value::List(Rc::new(RefCell::new(acc))))
            }
        }
    }

    /// Resolve a component by name. A known local component function is called
    /// with the same slot-routing as the codegen (`local_component_call`): named
    /// args reordered into parameter order, leftover positional args appended,
    /// and the rendered `children` list routed to a parameter named `children`
    /// (dropped when the function declares none). Anything else — `@xulo/ui`
    /// placeholders, unresolved names — becomes an opaque structured node; the
    /// headless runtime never mounts or renders it to a screen.
    pub(super) fn local_component_call(
        &self,
        name: &str,
        args: &[CallArg],
        children: &[UiElement],
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        if let Some(Value::Function(func)) = env.borrow().get(name) {
            let params: Vec<String> = func.params.iter().map(|p| p.name.clone()).collect();
            let mut slots: Vec<Option<Value>> = vec![None; params.len()];
            let mut extras = Vec::new();
            for (i, arg) in args.iter().enumerate() {
                let value = self.eval(&arg.value, env)?;
                match &arg.name {
                    Some(named) => {
                        if let Some(idx) = params.iter().position(|p| p == named) {
                            slots[idx] = Some(value);
                        } else {
                            extras.push(value);
                        }
                    }
                    None => {
                        if i < slots.len() {
                            slots[i] = Some(value);
                        } else {
                            extras.push(value);
                        }
                    }
                }
            }
            if let Some(idx) = params.iter().position(|p| p == "children") {
                slots[idx] = Some(self.ui_children_value(Some(children), env)?);
            }
            let mut bound = slots;
            bound.extend(extras.into_iter().map(Some));
            let depth = self.call_depth.get();
            if depth >= MAX_CALL_DEPTH {
                return Err(RunError::err(
                    format!("call depth exceeded: recursion limit of {MAX_CALL_DEPTH} reached"),
                    0..0,
                ));
            }
            self.call_depth.set(depth + 1);
            let result = self.run_function(&func, bound);
            self.call_depth.set(depth);
            return result;
        }
        let mut props = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let key = match &arg.name {
                Some(name) => name.clone(),
                // Numeric string keys mirror the codegen's `"0": v` props.
                None => i.to_string(),
            };
            props.push((key, self.eval(&arg.value, env)?));
        }
        let rendered = self.ui_children_value(Some(children), env)?;
        if !equal(&rendered, &Value::Null) {
            props.push(("children".to_string(), rendered));
        }
        Ok(Value::Object(Rc::new(RefCell::new(vec![
            ("name".to_string(), Value::String(Rc::from(name))),
            (
                "props".to_string(),
                Value::Object(Rc::new(RefCell::new(props))),
            ),
        ]))))
    }

    pub(super) fn exec_block(
        &self,
        block: &Block,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        let child = Env::child(env);
        self.exec_stmts(&block.statements, &child)
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
            let start = self.eval_number(&r.start, env)?;
            let end = self.eval_number(&r.end, env)?;
            let mut i = start;
            if recycle {
                let child = Env::child(env);
                while self.range_has_next(i, end, r.end_inclusive) {
                    child
                        .borrow_mut()
                        .reset(&f.iter_var, Value::Number(i), true);
                    match self.exec_stmts(&f.body.statements, &child)? {
                        Flow::Continue | Flow::LoopContinue => {}
                        Flow::Break => break,
                        other => return Ok(other),
                    }
                    i += 1.0;
                }
            } else {
                while self.range_has_next(i, end, r.end_inclusive) {
                    let child = Env::child(env);
                    child
                        .borrow_mut()
                        .define(&f.iter_var, Value::Number(i), true);
                    match self.exec_block(&f.body, &child)? {
                        Flow::Continue | Flow::LoopContinue => {}
                        Flow::Break => break,
                        other => return Ok(other),
                    }
                    i += 1.0;
                }
            }
            Ok(Flow::Continue)
        } else {
            let iterable = self.eval(&f.iterable, env)?;
            let list = match iterable {
                Value::List(list) => list,
                other => {
                    return Err(RunError::err(
                        format!(
                            "for loop must iterate over a `List`, found {}",
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
                    child.borrow_mut().reset(&f.iter_var, item, true);
                    match self.exec_stmts(&f.body.statements, &child)? {
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
                    child.borrow_mut().define(&f.iter_var, item, true);
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
    /// declaration, an anonymous expression, a nested loop/if body, an `impl`
    /// method, or a UI element — forces the per-round path instead.
    pub(super) fn exec_assign(
        &self,
        a: &xulo_core::ast::AssignStmt,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        let value = self.eval(&a.value, env)?;
        match &a.target {
            AssignTarget::Name(name) => {
                // A `@State` name holds a signal cell: assignment rewrites into
                // the cell (JS emits `name.set(v)`), and a shadowing plain
                // binding in an inner scope takes a plain assignment.
                if let Some(Value::Signal(cell)) = env.borrow().get(name) {
                    *cell.borrow_mut() = value;
                } else {
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
            }
            AssignTarget::Member(object, property) => {
                let obj = self.eval(object, env)?;
                match obj {
                    Value::Object(fields) => {
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
                    (Value::List(list), Value::Number(n)) => {
                        let i = list_index(*n, &a.span)?;
                        let mut list = list.borrow_mut();
                        if i < list.len() {
                            list[i] = value;
                        } else {
                            return Err(RunError::err(
                                format!(
                                    "index {n} out of bounds for a list of length {}",
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
        Ok(Flow::Continue)
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
