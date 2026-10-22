use super::*;

impl Interpreter {
    pub(super) fn eval(
        &self,
        expr: &Expression,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        match expr {
            Expression::Literal { value, .. } => self.eval_literal(value, env),
            Expression::Identifier { name, span } => match env.borrow().get(name) {
                // A `@State` read unwraps to the cell's value (`__signal().get()`);
                // a shadowing plain binding in an inner scope resolves first, so
                // loop variables and block locals read the raw value.
                Some(Value::Signal(cell)) => Ok(cell.borrow().clone()),
                Some(v) => Ok(v),
                None => Err(RunError::err(
                    format!("undefined variable `{name}`"),
                    span.clone(),
                )),
            },
            Expression::BinaryOp(bin) => self.eval_binary(bin, env),
            Expression::Unary(un) => {
                let operand = self.eval(&un.operand, env)?;
                match un.operator {
                    UnaryOperator::Not => Ok(Value::Boolean(!is_truthy(&operand))),
                    UnaryOperator::Neg => match operand {
                        Value::Number(n) => Ok(Value::Number(-n)),
                        other => Err(RunError::err(
                            format!("cannot negate {}", other.kind_name()),
                            un.span.clone(),
                        )),
                    },
                    UnaryOperator::BitNot => match operand {
                        Value::Number(n) => Ok(Value::Number((!(n as i64)) as f64)),
                        other => Err(RunError::err(
                            format!("cannot apply `~` to {}", other.kind_name()),
                            un.span.clone(),
                        )),
                    },
                    // `move x` yields the value (ownership transfer is a no-op
                    // in the interpreter); `copy x` deep-copies mutable cells.
                    UnaryOperator::Move => Ok(operand),
                    UnaryOperator::Copy => Ok(deep_copy_value(&operand)),
                }
            }
            Expression::Call(call) => self.call(call, env),
            Expression::EnumRef(r) => {
                // The `Task` combinator namespace (spec
                // `builtins/intrinsic-functions.md`).
                if r.enum_name == "Task"
                    && let Some(value) = self.builtin_ns_member("Task", &r.variant)
                {
                    return Ok(value);
                }
                // A module-namespace member in value position (`ns::CONST`,
                // `std::math::PI`): the head is not a declared enum and names
                // an object in scope — walk to it and read the member.
                if !self.enum_names.borrow().iter().any(|n| n == &r.enum_name) {
                    let parts: Vec<&str> = r.enum_name.split("::").collect();
                    let mut current = env.borrow().get(parts[0]);
                    for seg in &parts[1..] {
                        let next = match &current {
                            Some(Value::Object(obj)) => obj
                                .borrow()
                                .iter()
                                .find(|(k, _)| k == seg)
                                .map(|(_, v)| v.clone()),
                            _ => None,
                        };
                        if next.is_none() {
                            current = None;
                            break;
                        }
                        current = next;
                    }
                    if let Some(Value::Object(ns)) = current {
                        let found = ns
                            .borrow()
                            .iter()
                            .find(|(k, _)| k == &r.variant)
                            .map(|(_, v)| v.clone());
                        return match found {
                            Some(value) => Ok(value),
                            None => Err(RunError::err(
                                format!("`{}` has no member `{}`", r.enum_name, r.variant),
                                r.span.clone(),
                            )),
                        };
                    }
                }
                Ok(Value::Enum {
                    enum_name: r.enum_name.clone(),
                    tag: r.variant.clone(),
                    payload: None,
                })
            }
            Expression::If(if_expr) => self.eval_if(if_expr, env),
            Expression::Ternary(tr) => {
                let cond = self.eval(&tr.condition, env)?;
                if is_truthy(&cond) {
                    self.eval(&tr.then_value, env)
                } else {
                    self.eval(&tr.else_value, env)
                }
            }
            Expression::Match(m) => self.eval_match(m, env),
            Expression::Member(m) => self.eval_member(m, env),
            Expression::Index(idx) => {
                let obj = self.eval(&idx.object, env)?;
                let index = self.eval(&idx.index, env)?;
                match (obj, &index) {
                    (Value::List(list), Value::Number(n)) => {
                        let i = list_index(*n, &idx.span)?;
                        let list = list.borrow();
                        if i < list.len() {
                            Ok(list[i].clone())
                        } else {
                            Err(RunError::err(
                                format!(
                                    "index {n} out of bounds for a list of length {}",
                                    list.len()
                                ),
                                idx.span.clone(),
                            ))
                        }
                    }
                    (Value::String(s), Value::Number(n)) => {
                        // `"abc"[1]` reads a character, matching the JS path
                        // (which returns the string's element).
                        let i = list_index(*n, &idx.span)?;
                        match s.chars().nth(i) {
                            Some(c) => Ok(Value::String(c.to_string().into())),
                            None => Err(RunError::err(
                                format!(
                                    "index {n} out of bounds for a string of length {}",
                                    s.chars().count()
                                ),
                                idx.span.clone(),
                            )),
                        }
                    }
                    (Value::Object(fields), Value::String(key)) => {
                        let fields = fields.borrow();
                        match fields.iter().find(|(k, _)| k.as_str() == key.as_ref()) {
                            Some((_, v)) => Ok(v.clone()),
                            None => Ok(Value::Null),
                        }
                    }
                    (other, i) => Err(RunError::err(
                        format!("cannot index {} with {}", other.kind_name(), i.format()),
                        idx.span.clone(),
                    )),
                }
            }
            Expression::Nullish(n) => {
                let left = self.eval(&n.left, env)?;
                if matches!(left, Value::Null) {
                    self.eval(&n.right, env)
                } else {
                    Ok(left)
                }
            }
            Expression::Range(r) => {
                let start = self.eval_number(&r.start, env)?;
                let end = self.eval_number(&r.end, env)?;
                let mut out = Vec::new();
                let mut i = start;
                while self.range_has_next(i, end, r.end_inclusive) {
                    out.push(Value::Number(i));
                    i += 1.0;
                }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            Expression::Await { expr, span } => {
                let operand = self.eval(expr, env)?;
                let promise = match operand {
                    Value::Promise(p) => p,
                    other => Rc::new(RefCell::new(crate::value::Promise {
                        state: crate::value::PromiseState::Fulfilled(other),
                        awaiters: VecDeque::new(),
                    })),
                };
                self.await_promise(promise, span.clone())
            }
            Expression::FnExpr(f) => Ok(Value::Function(Rc::new(FunctionValue {
                params: f.params.clone(),
                body: f.body.clone(),
                return_type: f.return_type.clone(),
                is_async: f.is_async,
                closure: env.clone(),
            }))),
            Expression::Spread { span, .. } => Err(RunError::err(
                "`...` spread is only allowed inside list or object literals",
                span.clone(),
            )),
            Expression::CallValue(cv) => {
                let callee = self.eval(&cv.callee, env)?;
                self.call_value(&callee, &cv.arguments, env)
            }
            Expression::Component(c) => self.eval_component_stmt(c, env),
            Expression::Tuple { items, .. } => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item, env)?);
                }
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            Expression::TypedMap(tm) => {
                let mut fields = Vec::with_capacity(tm.entries.len());
                for entry in &tm.entries {
                    let key = self.eval(&entry.key, env)?;
                    let key_str = match &key {
                        Value::String(s) => s.to_string(),
                        Value::Number(n) => n.to_string(),
                        Value::Boolean(b) => b.to_string(),
                        other => other.format(),
                    };
                    let value = self.eval(&entry.value, env)?;
                    fields.push((key_str, value));
                }
                Ok(Value::Object(Rc::new(RefCell::new(fields))))
            }
            Expression::Is(is) => {
                let value = self.eval(&is.expr, env)?;
                let matched = type_matches_value(&is.type_, &value);
                Ok(Value::Boolean(if is.negated { !matched } else { matched }))
            }
            Expression::Panic { expr, span: _ } => {
                let value = self.eval(expr, env)?;
                Err(RunError::Throw(value))
            }
            Expression::Propagate { expr, .. } => {
                let value = self.eval(expr, env)?;
                match value {
                    // `Result::Err` leaves the function carrying the whole
                    // `Result`; `Result::Ok` unwraps to its payload; `null`
                    // leaves the function with `null`; anything else (an
                    // optional's value) passes through (spec
                    // `error-handling.md`). The boundary that collects the
                    // failure value is the enclosing function's call.
                    Value::Enum {
                        enum_name,
                        tag,
                        payload,
                    } if enum_name == "Result" && tag == "Err" => {
                        Err(RunError::Propagate(Value::Enum {
                            enum_name,
                            tag,
                            payload,
                        }))
                    }
                    Value::Enum {
                        enum_name: _,
                        tag,
                        payload,
                    } if tag == "Ok" => Ok(payload.map(|p| *p).unwrap_or(Value::Null)),
                    Value::Null => Err(RunError::Propagate(Value::Null)),
                    other => Ok(other),
                }
            }
            Expression::Arrow(arrow) => {
                let body = match &arrow.body {
                    xulo_core::ast::ArrowBody::Expr(e) => Block {
                        statements: vec![Statement::Expr(xulo_core::ast::ExprStmt {
                            expr: e.clone(),
                            has_semicolon: false,
                            span: e.span().clone(),
                        })],
                    },
                    xulo_core::ast::ArrowBody::Block(b) => b.clone(),
                };
                Ok(Value::Function(Rc::new(FunctionValue {
                    params: arrow.params.clone(),
                    body,
                    return_type: arrow.return_type.clone(),
                    is_async: arrow.is_async,
                    closure: env.clone(),
                })))
            }
            Expression::Spawn(spawn) => match &spawn.body {
                xulo_core::ast::SpawnBody::Async(block) => {
                    let block = block.clone();
                    let spawn_env = env.clone();
                    Ok(self.spawn_async(move |_yielder| {
                        with_interp(|interp| interp.run_body(&block, &spawn_env, false))
                    }))
                }
                xulo_core::ast::SpawnBody::Service { type_name, fields } => {
                    // The field initializers construct the service's state;
                    // the value is deep-copied into the service, which never
                    // shares storage with its spawner (spec concurrency.md).
                    let mut out = Vec::with_capacity(fields.len());
                    for field in fields {
                        out.push((field.name.clone(), self.eval(&field.value, env)?));
                    }
                    let state = deep_copy_value(&Value::Object(Rc::new(RefCell::new(out))));
                    // Resolve `on_message` now: the struct object lives in the
                    // module scope that lexically contains the spawn.
                    let on_message = env.borrow().get(type_name).and_then(|v| match v {
                        Value::Object(fields) => fields
                            .borrow()
                            .iter()
                            .find(|(k, _)| k == "on_message")
                            .map(|(_, v)| v.clone()),
                        _ => None,
                    });
                    let service = Rc::new(ServiceInstance {
                        type_name: type_name.clone(),
                        on_message,
                        state: state.clone(),
                        queue: RefCell::new(VecDeque::new()),
                        busy: Cell::new(false),
                        stopped: Cell::new(false),
                    });
                    self.services.borrow_mut().push(service.clone());

                    let send_service = service.clone();
                    let send = Value::Native(Rc::new(move |interp, args| {
                        if send_service.stopped.get() {
                            return Err(RunError::err(
                                format!(
                                    "service `{}` has stopped; cannot send",
                                    send_service.type_name
                                ),
                                0..0,
                            ));
                        }
                        let message = args.first().cloned().unwrap_or(Value::Null);
                        // The message is a value: it is deep-copied in transit
                        // (an `move x` operand is already an owned transfer at
                        // the language level; the single-threaded runtime keeps
                        // the copy, which is observationally equivalent here).
                        let message = deep_copy_value(&message);
                        let promise = Rc::new(RefCell::new(crate::value::Promise {
                            state: crate::value::PromiseState::Pending,
                            awaiters: VecDeque::new(),
                        }));
                        let settle: ServiceSettle = {
                            let promise = promise.clone();
                            Rc::new(move |interp, result| {
                                let mut p = promise.borrow_mut();
                                if !matches!(p.state, crate::value::PromiseState::Pending) {
                                    return;
                                }
                                p.state = match &result {
                                    Ok(v) => crate::value::PromiseState::Fulfilled(v.clone()),
                                    Err(e) => crate::value::PromiseState::Rejected(e.clone()),
                                };
                                let awaiters = std::mem::take(&mut p.awaiters);
                                drop(p);
                                for awaiter in awaiters {
                                    interp
                                        .ready
                                        .borrow_mut()
                                        .push_back((awaiter, Control::Resume(result.clone())));
                                }
                            })
                        };
                        send_service
                            .queue
                            .borrow_mut()
                            .push_back(ServiceMsg { message, settle });
                        interp.service_pump(&send_service);
                        Ok(Value::Promise(promise))
                    }));

                    let stop_service = service.clone();
                    let stop = Value::Native(Rc::new(move |_interp, _args| {
                        if !stop_service.stopped.replace(true) {
                            // Nothing further is queued; messages already
                            // waiting never run, so their sends settle as
                            // cancelled.
                            let queued = std::mem::take(&mut *stop_service.queue.borrow_mut());
                            for msg in queued {
                                (msg.settle)(
                                    _interp,
                                    Err(RunError::err(
                                        format!(
                                            "service `{}` stopped before handling the message",
                                            stop_service.type_name
                                        ),
                                        0..0,
                                    )),
                                );
                            }
                        }
                        Ok(Value::Null)
                    }));

                    let handle = Value::Object(Rc::new(RefCell::new(vec![
                        ("send".to_string(), send),
                        ("stop".to_string(), stop),
                    ])));
                    // Start the first queued message (if any raced ahead);
                    // normally the queue is empty here and this is a no-op.
                    self.service_pump(&service);
                    Ok(handle)
                }
            },
            Expression::Lock(lock) => self.block_value(&lock.body, env),
            Expression::Binding { name, span } => {
                // `$name` two-way binding, mirroring the JS shape
                // `{ value: ...get(), onChange: v => ...set(v) }`. For a signal the
                // onChange writes the cell; for a plain binding it is a no-op.
                match env.borrow().get(name) {
                    Some(Value::Signal(cell)) => {
                        let setter_cell = cell.clone();
                        let setter = Rc::new(
                            move |_interp: &Interpreter,
                                  args: &[Value]|
                                  -> Result<Value, RunError> {
                                let next = args.first().cloned().unwrap_or(Value::Null);
                                *setter_cell.borrow_mut() = next;
                                Ok(Value::Null)
                            },
                        );
                        Ok(Value::Object(Rc::new(RefCell::new(vec![
                            ("value".to_string(), cell.borrow().clone()),
                            ("onChange".to_string(), Value::Native(setter)),
                        ]))))
                    }
                    Some(v) => Ok(Value::Object(Rc::new(RefCell::new(vec![
                        ("value".to_string(), v),
                        (
                            "onChange".to_string(),
                            Value::Native(Rc::new(|_interp: &Interpreter, _args: &[Value]| {
                                Ok(Value::Null)
                            })),
                        ),
                    ])))),
                    None => Err(RunError::err(
                        format!("undefined variable `{name}`"),
                        span.clone(),
                    )),
                }
            }
        }
    }

    pub(super) fn eval_literal(
        &self,
        lit: &Literal,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        Ok(match lit {
            Literal::String(s) => Value::String(Rc::from(s.as_str())),
            Literal::Number(n) => Value::Number(*n),
            Literal::Boolean(b) => Value::Boolean(*b),
            Literal::Null => Value::Null,
            Literal::List(items) => {
                let mut out = Vec::new();
                for item in items {
                    match item {
                        Expression::Spread { expr, span } => {
                            let spread = self.eval(expr, env)?;
                            match spread {
                                Value::List(list) => {
                                    out.extend(list.borrow().iter().cloned());
                                }
                                other => {
                                    return Err(RunError::err(
                                        format!("cannot spread {} into a list", other.kind_name()),
                                        span.clone(),
                                    ));
                                }
                            }
                        }
                        other => out.push(self.eval(other, env)?),
                    }
                }
                Value::List(Rc::new(RefCell::new(out)))
            }
            Literal::Object(fields) => {
                let mut out: Vec<(String, Value)> = Vec::new();
                for field in fields {
                    match field {
                        ObjectField::Field { name, value } => {
                            let v = self.eval(value, env)?;
                            out.push((name.clone(), v));
                        }
                        ObjectField::Spread { value } => {
                            let spread = self.eval(value, env)?;
                            match spread {
                                Value::Object(obj) => {
                                    out.extend(obj.borrow().iter().cloned());
                                }
                                other => {
                                    return Err(RunError::err(
                                        format!(
                                            "cannot spread {} into an object",
                                            other.kind_name()
                                        ),
                                        0..0,
                                    ));
                                }
                            }
                        }
                    }
                }
                Value::Object(Rc::new(RefCell::new(out)))
            }
        })
    }

    pub(super) fn eval_binary(
        &self,
        bin: &BinaryOp,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        let left = self.eval(&bin.left, env)?;
        match bin.operator {
            BinaryOperator::And => {
                if !is_truthy(&left) {
                    return Ok(Value::Boolean(false));
                }
                let right = self.eval(&bin.right, env)?;
                Ok(Value::Boolean(is_truthy(&right)))
            }
            BinaryOperator::Or => {
                if is_truthy(&left) {
                    return Ok(Value::Boolean(true));
                }
                let right = self.eval(&bin.right, env)?;
                Ok(Value::Boolean(is_truthy(&right)))
            }
            _ => {
                let right = self.eval(&bin.right, env)?;
                match bin.operator {
                    BinaryOperator::Add => self.add(left, right, bin),
                    BinaryOperator::Sub => self.arith(left, right, bin, "`-`", |a, b| a - b),
                    BinaryOperator::Mul => self.arith(left, right, bin, "`*`", |a, b| a * b),
                    BinaryOperator::Div => self.arith(left, right, bin, "`/`", |a, b| a / b),
                    BinaryOperator::Mod => self.arith(left, right, bin, "`%`", |a, b| a % b),
                    BinaryOperator::Pow => self.arith(left, right, bin, "`**`", |a, b| a.powf(b)),
                    BinaryOperator::BitAnd => {
                        self.arith(left, right, bin, "`&`", |a, b| (a as i64 & b as i64) as f64)
                    }
                    BinaryOperator::BitOr => {
                        self.arith(left, right, bin, "`|`", |a, b| (a as i64 | b as i64) as f64)
                    }
                    BinaryOperator::Xor => {
                        self.arith(left, right, bin, "`^`", |a, b| (a as i64 ^ b as i64) as f64)
                    }
                    BinaryOperator::Shl => self.arith(left, right, bin, "`<<`", |a, b| {
                        ((a as i64) << (b as i64)) as f64
                    }),
                    BinaryOperator::Shr => self.arith(left, right, bin, "`>>`", |a, b| {
                        ((a as i64) >> (b as i64)) as f64
                    }),
                    BinaryOperator::Eq => Ok(Value::Boolean(equal(&left, &right))),
                    BinaryOperator::Neq => Ok(Value::Boolean(!equal(&left, &right))),
                    BinaryOperator::Lt => self.compare(left, right, bin, "<"),
                    BinaryOperator::Gt => self.compare(left, right, bin, ">"),
                    BinaryOperator::Lte => self.compare(left, right, bin, "<="),
                    BinaryOperator::Gte => self.compare(left, right, bin, ">="),
                    BinaryOperator::And | BinaryOperator::Or => unreachable!(),
                }
            }
        }
    }

    pub(super) fn add(&self, left: Value, right: Value, bin: &BinaryOp) -> Result<Value, RunError> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
            (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{a}{b}").into())),
            (Value::List(a), Value::List(b)) => {
                let mut out = a.borrow().clone();
                out.extend(b.borrow().iter().cloned());
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            (Value::String(a), Value::Number(b)) => Ok(Value::String(
                format!("{a}{}", crate::value::format_number(b)).into(),
            )),
            (Value::Number(a), Value::String(b)) => Ok(Value::String(
                format!("{}{b}", crate::value::format_number(a)).into(),
            )),
            (a, b) => Err(RunError::err(
                format!(
                    "cannot apply `+` to {} and {}",
                    a.kind_name(),
                    b.kind_name()
                ),
                bin.span.clone(),
            )),
        }
    }

    pub(super) fn arith(
        &self,
        left: Value,
        right: Value,
        bin: &BinaryOp,
        op: &str,
        f: fn(f64, f64) -> f64,
    ) -> Result<Value, RunError> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(f(a, b))),
            (a, b) => Err(RunError::err(
                format!(
                    "cannot apply {op} to {} and {}",
                    a.kind_name(),
                    b.kind_name()
                ),
                bin.span.clone(),
            )),
        }
    }

    pub(super) fn compare(
        &self,
        left: Value,
        right: Value,
        bin: &BinaryOp,
        op: &str,
    ) -> Result<Value, RunError> {
        let result = match (&left, &right) {
            (Value::Number(a), Value::Number(b)) => match op {
                "<" => a < b,
                ">" => a > b,
                "<=" => a <= b,
                _ => a >= b,
            },
            (Value::String(a), Value::String(b)) => match op {
                "<" => a < b,
                ">" => a > b,
                "<=" => a <= b,
                _ => a >= b,
            },
            _ => {
                return Err(RunError::err(
                    format!(
                        "cannot compare {} with {}",
                        left.kind_name(),
                        right.kind_name()
                    ),
                    bin.span.clone(),
                ));
            }
        };
        Ok(Value::Boolean(result))
    }

    pub(super) fn eval_number(
        &self,
        expr: &Expression,
        env: &Rc<RefCell<Env>>,
    ) -> Result<f64, RunError> {
        match self.eval(expr, env)? {
            Value::Number(n) => Ok(n),
            other => Err(RunError::err(
                format!("expected a `number`, found {}", other.kind_name()),
                expr.span().clone(),
            )),
        }
    }

    /// Whether `i` is still inside a range with upper bound `end`:
    /// `..<` is half-open (i < end), `...` is closed (i <= end).
    pub(super) fn range_has_next(&self, i: f64, end: f64, end_inclusive: bool) -> bool {
        if end_inclusive { i <= end } else { i < end }
    }

    pub(super) fn eval_if(
        &self,
        if_expr: &IfExpr,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        let cond = self.eval(&if_expr.condition, env)?;
        if is_truthy(&cond) {
            self.block_value(&if_expr.then_branch, env)
        } else if let Some(else_branch) = &if_expr.else_branch {
            self.block_value(else_branch, env)
        } else {
            Ok(Value::Null)
        }
    }

    /// A block in value position: a trailing expression (or `return`) is the
    /// block's value, otherwise the block evaluates to `null`.
    pub(super) fn block_value(
        &self,
        block: &Block,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        let child = Env::child(env);
        let (prefix, tail) = match block.statements.last() {
            Some(Statement::Expr(e)) if !e.has_semicolon => (
                &block.statements[..block.statements.len() - 1],
                Tail::Expr(&e.expr),
            ),
            Some(Statement::Return(r)) => (
                &block.statements[..block.statements.len() - 1],
                Tail::Return(r),
            ),
            _ => {
                return match self.exec_stmts(&block.statements, &child)? {
                    Flow::Continue | Flow::LoopContinue => Ok(Value::Null),
                    Flow::Return(v) => Ok(v),
                    Flow::Break => Ok(Value::Null),
                };
            }
        };
        if let Some(v) = self.exec_prefix(prefix, &child)? {
            // An explicit `return` before the trailing expression short-circuits
            // the block's value (the JS IIFE does the same).
            return Ok(v);
        }
        match tail {
            Tail::Expr(expr) => self.eval(expr, &child),
            Tail::Return(r) => match &r.value {
                Some(expr) => self.eval(expr, &child),
                None => Ok(Value::Null),
            },
        }
    }

    /// Execute every statement before a block's trailing expression, unwinding
    /// on an explicit `return`: the returned value short-circuits the block's
    /// value (matching the JS codegen, whose value-position `if`/`match` arms
    /// are IIFEs where `return` exits the arm). Returns `Some(value)` when a
    /// statement returned early.
    pub(super) fn exec_prefix(
        &self,
        stmts: &[Statement],
        env: &Rc<RefCell<Env>>,
    ) -> Result<Option<Value>, RunError> {
        for stmt in stmts {
            match self.exec_stmt(stmt, env)? {
                Flow::Continue | Flow::LoopContinue => {}
                Flow::Return(v) => return Ok(Some(v)),
                Flow::Break => return Ok(None),
            }
        }
        Ok(None)
    }

    pub(super) fn eval_member(
        &self,
        m: &xulo_core::ast::MemberAccess,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        let object = self.eval(&m.object, env)?;
        if m.optional && matches!(object, Value::Null) {
            return Ok(Value::Null);
        }
        match &object {
            Value::Object(fields) => {
                let fields = fields.borrow();
                match fields.iter().find(|(k, _)| k == &m.property) {
                    Some((_, v)) => Ok(v.clone()),
                    None => Ok(Value::Null),
                }
            }
            Value::List(list) if m.property == "length" => {
                Ok(Value::Number(list.borrow().len() as f64))
            }
            Value::List(list) if m.property == "capacity" => {
                Ok(Value::Number(list.borrow().capacity() as f64))
            }
            Value::String(s) if m.property == "length" => {
                // JS `.length` counts UTF-16 code units, not Unicode scalars
                // (`"😀".length` is 2 there); mirror it.
                Ok(Value::Number(s.encode_utf16().count() as f64))
            }
            other => Err(RunError::err(
                format!(
                    "cannot access member `{}` of {}",
                    m.property,
                    other.kind_name()
                ),
                m.span.clone(),
            )),
        }
    }

    pub(super) fn eval_match(
        &self,
        m: &MatchExpr,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        let scrutinee = self.eval(&m.value, env)?;
        for arm in &m.arms {
            let arm_env = Env::child(env);
            if !pattern_matches(&arm.pattern, &scrutinee, &arm_env)? {
                continue;
            }
            if let Some(guard) = &arm.guard {
                let guard_value = self.eval(guard, &arm_env)?;
                if !is_truthy(&guard_value) {
                    continue;
                }
            }
            return self.eval_match_body(&arm.value, &arm_env);
        }
        Err(RunError::err("non-exhaustive match", m.span.clone()))
    }

    /// Evaluate a match arm body in the arm's scope.
    pub(super) fn eval_match_body(
        &self,
        body: &xulo_core::ast::MatchBody,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        match body {
            xulo_core::ast::MatchBody::Expr(e) => self.eval(e, env),
            xulo_core::ast::MatchBody::Block(b) => self.block_value(b, env),
        }
    }
}
