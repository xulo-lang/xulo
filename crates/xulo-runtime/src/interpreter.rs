use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, Yielder};

use xulo_core::ast::{
    AssignTarget, BinaryOp, BinaryOperator, Block, Call, Expression, FnDef, ForStmt, IfExpr,
    ImplDecl, Literal, MatchExpr, ObjectField, Param, Program, ReturnStmt, Statement, TryStmt,
    Type, UnaryOperator, impl_fn_name,
};
use xulo_core::error::{ErrorKind, XuloError};

use crate::env::Env;
use crate::value::{FunctionValue, Value, equal, is_truthy};

pub use xulo_core::ast::CallArg;

/// Signature of a builtin function (`print`, `str`).
pub type NativeFn = fn(&Interpreter, &[Value]) -> Result<Value, RunError>;

/// The outcome of running a statement: keep executing, or return from the
/// enclosing function. Thrown values are carried by [`RunError::Throw`] so a
/// surrounding `try`/`catch` can intercept them.
pub enum Flow {
    Continue,
    Return(Value),
}

/// A run-time failure: a fatal error (optionally with a source span) or a value
/// thrown by `throw` that a surrounding `try`/`catch` may handle.
#[derive(Clone)]
pub enum RunError {
    Err(XuloError),
    Throw(Value),
}

impl RunError {
    fn err(message: impl Into<String>, span: impl Into<std::ops::Range<usize>>) -> Self {
        RunError::Err(XuloError::new(ErrorKind::Runtime, message).at(span.into()))
    }
}

/// What a block in value position evaluates to: a trailing expression or a
/// trailing `return`.
enum Tail<'a> {
    Expr(&'a Expression),
    Return(&'a ReturnStmt),
}

/// The runtime exports a module's `export` statements produce.
pub struct ModuleExports {
    pub bindings: Vec<(String, Value)>,
    pub default: Option<Value>,
}

/// The control message passed into an async coroutine. The first resume is
/// `Start`; every later resume delivers the result of whatever the coroutine
/// last awaited (or suspended on).
enum Control {
    Start,
    Resume(Result<Value, RunError>),
}

/// One suspended async call. Its promise is shared with every `await`er; the
/// coroutine runs the function body and suspends at each `await`.
struct Task {
    coro: Coroutine<Control, (), Result<Value, RunError>, DefaultStack>,
    promise: Rc<RefCell<crate::value::Promise>>,
}

// The interpreter an async coroutine's closure runs against. Set (to `self`)
// around every `resume`, so the `'static` coroutine closures can reach the
// shared `Interpreter` without capturing a reference. Single-threaded, so the
// raw pointer is always valid while a closure is executing.
thread_local! {
    static CURRENT_INTERP: Cell<Option<*const Interpreter>> = const { Cell::new(None) };
}

/// The tree-walking interpreter. Collects `print` output into `out`; the caller
/// renders it (the CLI prints it to stdout).
pub struct Interpreter {
    out: RefCell<Vec<String>>,
    global: Rc<RefCell<Env>>,
    /// Async tasks by id; `None` once a task has completed.
    tasks: RefCell<Vec<Option<Task>>>,
    /// Tasks to resume next, FIFO (JS microtask order).
    ready: RefCell<VecDeque<(usize, Control)>>,
    /// The task currently executing (the inline-resumed coroutine).
    current_task: Cell<Option<usize>>,
    /// Each task's yielder pointer, stashed by the coroutine on first entry.
    task_yielder: RefCell<Vec<Option<*const Yielder<Control, ()>>>>,
    /// Active call depth (see [`MAX_CALL_DEPTH`]).
    call_depth: Cell<usize>,
}

/// Maximum nested interpreter calls. Guards against unbounded recursion, which
/// otherwise crashes the process: sync recursion overflows the host stack with
/// a `stack overflow` abort, and async recursion spawns one 1 MiB coroutine
/// stack per level until allocation fails. Reaching the limit returns a clean
/// runtime error instead. The depth is counted at call time, so an `async`
/// chain that suspends at each `await` is bounded too.
///
/// The limit must leave headroom for *debug* builds on small host stacks: an
/// unoptimized interpreter frame can be several KiB, and test/embedding
/// threads default to a 2 MiB stack, so 128 × ~8 KiB stays safely under it.
pub const MAX_CALL_DEPTH: usize = 128;

fn native_print(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let line = args
        .iter()
        .map(|v| v.format())
        .collect::<Vec<_>>()
        .join(" ");
    interp.out.borrow_mut().push(line);
    Ok(Value::Null)
}

fn native_str(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(match args.first() {
        Some(v) => Value::String(v.format()),
        None => Value::String(String::new()),
    })
}

impl Interpreter {
    /// A fresh interpreter with `print`/`str` builtins registered in the global
    /// scope (user declarations of the same name shadow them).
    pub fn new() -> Self {
        let global = Env::root();
        global
            .borrow_mut()
            .define("print", Value::Native(native_print));
        global.borrow_mut().define("str", Value::Native(native_str));
        Interpreter {
            out: RefCell::new(Vec::new()),
            global,
            tasks: RefCell::new(Vec::new()),
            ready: RefCell::new(VecDeque::new()),
            current_task: Cell::new(None),
            task_yielder: RefCell::new(Vec::new()),
            call_depth: Cell::new(0),
        }
    }

    /// Run a whole program: register functions/enums, execute top-level
    /// statements, then invoke `main` if one is declared. Returns the lines
    /// printed by `print`.
    pub fn run(&self, program: &Program) -> Result<Vec<String>, XuloError> {
        let global = self.global.clone();
        for statement in &program.statements {
            match statement {
                Statement::Fn(f) => self.register_fn(f, &global),
                Statement::Impl(imp) => self.register_impl(imp, &global),
                Statement::Export(export) => {
                    if let xulo_core::ast::ExportItem::Fn(f) = &export.item {
                        self.register_fn(f, &global);
                    }
                }
                _ => {}
            }
        }
        for statement in &program.statements {
            match self.exec_stmt(statement, &global) {
                Ok(_) => {}
                Err(RunError::Err(e)) => return Err(e),
                Err(RunError::Throw(v)) => {
                    return Err(self.uncaught(&v));
                }
            }
        }
        if let Some(main) = main_fn(program) {
            self.run_main(main, &global)?;
        } else {
            // No `main` (e.g. a script of top-level statements): still drain the
            // async task queue, or fire-and-forget `work()` calls would park
            // forever on their first `await` (the JS path drains microtasks).
            self.drive();
        }
        Ok(self.out.take())
    }

    /// Invoke `main`, draining the async task queue afterward. A rejected async
    /// `main` surfaces like any other uncaught error.
    fn run_main(&self, main: &FnDef, global: &Rc<RefCell<Env>>) -> Result<(), XuloError> {
        if is_component_return(&main.return_type) {
            return Err(XuloError::new(
                ErrorKind::Runtime,
                "UI components are not supported in the native runtime",
            ));
        }
        let result = self.call_fn(main, &[], global);
        self.drive();
        match result {
            Ok(Value::Promise(promise)) => {
                let state = promise.borrow();
                match &state.state {
                    crate::value::PromiseState::Fulfilled(_) => {}
                    crate::value::PromiseState::Rejected(e) => {
                        return Err(match e {
                            RunError::Err(e) => e.clone(),
                            RunError::Throw(v) => self.uncaught(v),
                        });
                    }
                    crate::value::PromiseState::Pending => {
                        return Err(XuloError::new(
                            ErrorKind::Runtime,
                            "async `main` did not complete",
                        ));
                    }
                }
                Ok(())
            }
            Ok(_) => Ok(()),
            Err(RunError::Err(e)) => Err(e),
            Err(RunError::Throw(v)) => Err(self.uncaught(&v)),
        }
    }

    /// Execute one module with its (already resolved) import bindings bound in
    /// its own scope. A module's own `import` statements are skipped — the
    /// caller pre-binds them so inter-module wiring stays at the CLI level.
    pub fn exec_module(
        &self,
        program: &Program,
        imports: &[(String, Value)],
        run_main: bool,
    ) -> Result<ModuleExports, RunError> {
        let global = Env::child(&self.global);
        for (name, value) in imports {
            global.borrow_mut().define(name, value.clone());
        }
        for statement in &program.statements {
            match statement {
                Statement::Fn(f) => self.register_fn(f, &global),
                Statement::Impl(imp) => self.register_impl(imp, &global),
                Statement::Export(export) => self.register_export_fns(&export.item, &global),
                _ => {}
            }
        }
        for statement in &program.statements {
            if matches!(statement, Statement::Import(_)) {
                continue;
            }
            match self.exec_stmt(statement, &global) {
                Ok(_) => {}
                Err(e) => return Err(e),
            }
        }
        if run_main && let Some(main) = main_fn(program) {
            self.run_main(main, &global).map_err(RunError::Err)?;
        } else {
            // A module without `main` (any dependency module, or a no-main
            // entry) still drains its async tasks, matching JS module
            // semantics where fire-and-forget async work completes.
            self.drive();
        }
        let mut bindings = Vec::new();
        let mut default = None;
        for statement in &program.statements {
            if let Statement::Export(export) = statement {
                collect_exports(&export.item, &global, &mut bindings, &mut default);
            }
        }
        Ok(ModuleExports { bindings, default })
    }

    /// Take the collected `print` lines so far (e.g. across executed modules).
    pub fn take_output(&self) -> Vec<String> {
        self.out.take()
    }

    fn register_export_fns(&self, item: &xulo_core::ast::ExportItem, env: &Rc<RefCell<Env>>) {
        match item {
            xulo_core::ast::ExportItem::Fn(f) => self.register_fn(f, env),
            xulo_core::ast::ExportItem::Default(inner) => self.register_export_fns(inner, env),
            _ => {}
        }
    }

    fn uncaught(&self, value: &Value) -> XuloError {
        XuloError::new(
            ErrorKind::Runtime,
            format!("uncaught exception: {}", value.format()),
        )
    }

    /// Start an async function body as a coroutine task and inline-resume it
    /// once, so its synchronous prefix runs at the call site exactly as in JS.
    /// Returns the task's promise.
    fn spawn_async(
        &self,
        body: impl FnOnce(&Yielder<Control, ()>) -> Result<Value, RunError> + 'static,
    ) -> Value {
        let promise = Rc::new(RefCell::new(crate::value::Promise {
            state: crate::value::PromiseState::Pending,
            awaiters: VecDeque::new(),
        }));
        let promise_for_task = promise.clone();
        let interp = self as *const Interpreter;
        let coro = Coroutine::new(
            move |yielder: &Yielder<Control, ()>, _start: Control| -> Result<Value, RunError> {
                // SAFETY: `resume_task` sets CURRENT_INTERP to this interpreter
                // around every resume; the coroutine only runs inside one.
                let interp = unsafe { &*interp };
                let id = interp
                    .current_task
                    .get()
                    .expect("a task id is set before an async coroutine runs");
                interp.task_yielder.borrow_mut()[id] = Some(yielder as *const Yielder<Control, ()>);
                body(yielder)
            },
        );
        let id = {
            let mut tasks = self.tasks.borrow_mut();
            tasks.push(Some(Task {
                coro,
                promise: promise_for_task,
            }));
            self.task_yielder.borrow_mut().push(None);
            tasks.len() - 1
        };
        self.resume_task(id, Control::Start);
        Value::Promise(promise)
    }

    /// Resume `id`'s coroutine with `control`. A `Yield` parks the task back in
    /// `tasks`; a `Return` settles its promise and enqueues every awaiter (FIFO)
    /// for the next drive pass. `current_task`/`CURRENT_INTERP` are saved and
    /// restored so nested spawns from inside a coroutine work.
    fn resume_task(&self, id: usize, control: Control) {
        let prev = self.current_task.replace(Some(id));
        let prev_interp = CURRENT_INTERP.with(|c| c.replace(Some(self as *const Interpreter)));
        let mut task = self.tasks.borrow_mut()[id].take().expect("resumable task");
        let result = task.coro.resume(control);
        CURRENT_INTERP.with(|c| c.set(prev_interp));
        self.current_task.set(prev);
        match result {
            corosensei::CoroutineResult::Yield(()) => {
                self.tasks.borrow_mut()[id] = Some(task);
            }
            corosensei::CoroutineResult::Return(ret) => {
                let mut promise = task.promise.borrow_mut();
                promise.state = match &ret {
                    Ok(v) => crate::value::PromiseState::Fulfilled(v.clone()),
                    Err(e) => crate::value::PromiseState::Rejected(e.clone()),
                };
                let awaiters = std::mem::take(&mut promise.awaiters);
                drop(promise);
                for awaiter in awaiters {
                    self.ready
                        .borrow_mut()
                        .push_back((awaiter, Control::Resume(ret.clone())));
                }
            }
        }
    }

    /// Drain the ready queue (JS microtasks): resume each queued task in FIFO
    /// order until none is ready to run.
    fn drive(&self) {
        loop {
            let next = self.ready.borrow_mut().pop_front();
            let Some((id, control)) = next else { break };
            self.resume_task(id, control);
        }
    }

    fn current_yielder(&self, id: usize) -> *const Yielder<Control, ()> {
        self.task_yielder.borrow()[id].expect("an awaiting task has a yielder")
    }

    fn register_fn(&self, f: &FnDef, env: &Rc<RefCell<Env>>) {
        let func = Value::Function(Rc::new(FunctionValue {
            params: f.params.clone(),
            body: f.body.clone(),
            return_type: f.return_type.clone(),
            is_async: f.is_async,
            closure: env.clone(),
        }));
        env.borrow_mut().define(&f.name, func);
    }

    /// Register every method of an `impl` block under its mangled name
    /// (`impl_{Trait}_{Type}_{method}`), the same name `Trait::method` calls
    /// are annotated with. Registered up front like `fn` declarations, so
    /// dispatch calls are defined regardless of statement order. The name is
    /// defined on the shared root env — not the caller's module env — so a
    /// dependent module can dispatch to an `impl` declared in another module;
    /// the function value keeps `closure` as its own module's env so its body
    /// still resolves that module's locals.
    fn register_impl(&self, imp: &ImplDecl, closure: &Rc<RefCell<Env>>) {
        for method in &imp.methods {
            let func = Value::Function(Rc::new(FunctionValue {
                params: method.params.clone(),
                body: method.body.clone(),
                return_type: method.return_type.clone(),
                is_async: method.is_async,
                closure: closure.clone(),
            }));
            self.global.borrow_mut().define(
                &impl_fn_name(&imp.trait_name, &imp.type_name, &method.name),
                func,
            );
        }
    }

    fn exec_stmt(&self, stmt: &Statement, env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
        match stmt {
            Statement::Fn(f) => {
                self.register_fn(f, env);
                Ok(Flow::Continue)
            }
            Statement::Let(binding) => {
                let value = match &binding.value {
                    Some(expr) => self.eval(expr, env)?,
                    None => Value::Null,
                };
                env.borrow_mut().define(&binding.name, value);
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
                        Flow::Continue => {}
                        other => return Ok(other),
                    }
                }
                Ok(Flow::Continue)
            }
            Statement::Assign(assign) => self.exec_assign(assign, env),
            Statement::TypeAlias(_) => Ok(Flow::Continue),
            Statement::Enum(e) => {
                // Register the enum's runtime value so `export { Color }` and
                // `export enum` both resolve it for importers (construction
                // `Color::Red` is name-based and unaffected).
                env.borrow_mut().define(&e.name, enum_value(e));
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
            Statement::Import(imp) => {
                if imp.type_only {
                    Ok(Flow::Continue)
                } else {
                    Err(RunError::err(
                        "imports are not supported in the native runtime",
                        0..0,
                    ))
                }
            }
            Statement::Export(export) => self.exec_export(&export.item, env),
            Statement::State(_)
            | Statement::Store(_)
            | Statement::Effect(_)
            | Statement::Environment(_) => Err(RunError::err(
                "reactive state (`@State`/`@Store`/`@Effect`/`@Environment`) is not supported in the native runtime",
                0..0,
            )),
            Statement::Component(_) => Err(RunError::err(
                "UI components are not supported in the native runtime",
                0..0,
            )),
            Statement::Trait(_) => Ok(Flow::Continue),
            Statement::Impl(imp) => {
                self.register_impl(imp, env);
                Ok(Flow::Continue)
            }
        }
    }

    fn exec_export(
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
            xulo_core::ast::ExportItem::Enum(_)
            | xulo_core::ast::ExportItem::Type(_)
            | xulo_core::ast::ExportItem::Trait(_)
            | xulo_core::ast::ExportItem::Names(_) => Ok(Flow::Continue),
            xulo_core::ast::ExportItem::Default(inner) => self.exec_export(inner, env),
        }
    }

    fn exec_block(&self, block: &Block, env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
        let child = Env::child(env);
        self.exec_stmts(&block.statements, &child)
    }

    fn exec_stmts(&self, stmts: &[Statement], env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
        for stmt in stmts {
            match self.exec_stmt(stmt, env)? {
                Flow::Continue => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Continue)
    }

    fn exec_for(&self, f: &ForStmt, env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
        if let Expression::Range(r) = &f.iterable {
            let start = self.eval_number(&r.start, env)?;
            let end = self.eval_number(&r.end, env)?;
            let mut i = start;
            while i < end {
                let child = Env::child(env);
                child.borrow_mut().define(&f.iter_var, Value::Number(i));
                match self.exec_block(&f.body, &child)? {
                    Flow::Continue => {}
                    other => return Ok(other),
                }
                i += 1.0;
            }
            Ok(Flow::Continue)
        } else {
            let iterable = self.eval(&f.iterable, env)?;
            let list = match iterable {
                Value::List(list) => list,
                other => {
                    return Err(RunError::err(
                        format!(
                            "for loop must iterate over a `list`, found {}",
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
            loop {
                let item = {
                    let list = list.borrow();
                    if i >= list.len() {
                        break;
                    }
                    list[i].clone()
                };
                let child = Env::child(env);
                child.borrow_mut().define(&f.iter_var, item);
                match self.exec_block(&f.body, &child)? {
                    Flow::Continue => {}
                    other => return Ok(other),
                }
                i += 1;
            }
            Ok(Flow::Continue)
        }
    }

    fn exec_assign(
        &self,
        a: &xulo_core::ast::AssignStmt,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Flow, RunError> {
        let value = self.eval(&a.value, env)?;
        match &a.target {
            AssignTarget::Name(name) => {
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
                        if let Some((_, slot)) = fields.iter_mut().find(|(k, _)| k == key) {
                            *slot = value;
                        } else {
                            fields.push((key.clone(), value));
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

    fn exec_try(&self, t: &TryStmt, env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
        match self.exec_block(&t.try_block, env) {
            Ok(flow) => Ok(flow),
            Err(RunError::Throw(value)) => {
                let child = Env::child(env);
                child.borrow_mut().define(&t.catch_var, value);
                self.exec_block(&t.catch_block, &child)
            }
            Err(e) => Err(e),
        }
    }

    fn eval(&self, expr: &Expression, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
        match expr {
            Expression::Literal { value, .. } => self.eval_literal(value, env),
            Expression::Identifier { name, span } => match env.borrow().get(name) {
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
                }
            }
            Expression::Call(call) => self.call(call, env),
            Expression::EnumRef(r) => Ok(Value::Enum {
                enum_name: r.enum_name.clone(),
                tag: r.variant.clone(),
                payload: None,
            }),
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
                            Some(c) => Ok(Value::String(c.to_string())),
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
                        match fields.iter().find(|(k, _)| k == key) {
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
                while i < end {
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
                let task = self.current_task.get().ok_or_else(|| {
                    RunError::err(
                        "`await` may only be used inside an `async` function",
                        span.clone(),
                    )
                })?;
                let settled = {
                    let promise = promise.borrow();
                    match &promise.state {
                        crate::value::PromiseState::Pending => None,
                        crate::value::PromiseState::Fulfilled(v) => Some(Ok(v.clone())),
                        crate::value::PromiseState::Rejected(e) => Some(Err(e.clone())),
                    }
                };
                if let Some(result) = settled {
                    // Awaiting an already-settled promise defers to a microtask
                    // (matching JS), so re-queue ourselves.
                    self.ready
                        .borrow_mut()
                        .push_back((task, Control::Resume(result)));
                } else {
                    promise.borrow_mut().awaiters.push_back(task);
                }
                let yielder = self.current_yielder(task);
                let next = unsafe { (*yielder).suspend(()) };
                match next {
                    Control::Resume(result) => result,
                    Control::Start => {
                        unreachable!("a suspended coroutine cannot be resumed with Start")
                    }
                }
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
            Expression::Binding { name, span } => Err(RunError::err(
                format!("`$` binding `${name}` is not supported in the native runtime"),
                span.clone(),
            )),
        }
    }

    fn eval_literal(&self, lit: &Literal, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
        Ok(match lit {
            Literal::String(s) => Value::String(s.clone()),
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

    fn eval_binary(&self, bin: &BinaryOp, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
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

    fn add(&self, left: Value, right: Value, bin: &BinaryOp) -> Result<Value, RunError> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => Ok(Value::Number(a + b)),
            (Value::String(a), Value::String(b)) => Ok(Value::String(format!("{a}{b}"))),
            (Value::List(a), Value::List(b)) => {
                let mut out = a.borrow().clone();
                out.extend(b.borrow().iter().cloned());
                Ok(Value::List(Rc::new(RefCell::new(out))))
            }
            (Value::String(a), Value::Number(b)) => Ok(Value::String(format!(
                "{a}{}",
                crate::value::format_number(b)
            ))),
            (Value::Number(a), Value::String(b)) => Ok(Value::String(format!(
                "{}{b}",
                crate::value::format_number(a)
            ))),
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

    fn arith(
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

    fn compare(
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

    fn eval_number(&self, expr: &Expression, env: &Rc<RefCell<Env>>) -> Result<f64, RunError> {
        match self.eval(expr, env)? {
            Value::Number(n) => Ok(n),
            other => Err(RunError::err(
                format!("expected a `number`, found {}", other.kind_name()),
                expr.span().clone(),
            )),
        }
    }

    fn eval_if(&self, if_expr: &IfExpr, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
        let cond = self.eval(&if_expr.condition, env)?;
        if is_truthy(&cond) {
            self.block_value(&if_expr.then_branch, env)
        } else if let Some(else_branch) = &if_expr.else_branch {
            self.block_value(else_branch, env)
        } else {
            Ok(Value::Null)
        }
    }

    /// Run an `if` in statement position: execute the chosen branch as a
    /// statement block, propagating an early `return` out of the function.
    fn exec_if_stmt(&self, if_expr: &IfExpr, env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
        let cond = self.eval(&if_expr.condition, env)?;
        if is_truthy(&cond) {
            self.exec_block(&if_expr.then_branch, env)
        } else if let Some(else_branch) = &if_expr.else_branch {
            self.exec_block(else_branch, env)
        } else {
            Ok(Flow::Continue)
        }
    }

    /// A block in value position: a trailing expression (or `return`) is the
    /// block's value, otherwise the block evaluates to `null`.
    fn block_value(&self, block: &Block, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
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
                    Flow::Continue => Ok(Value::Null),
                    Flow::Return(v) => Ok(v),
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
    fn exec_prefix(
        &self,
        stmts: &[Statement],
        env: &Rc<RefCell<Env>>,
    ) -> Result<Option<Value>, RunError> {
        for stmt in stmts {
            match self.exec_stmt(stmt, env)? {
                Flow::Continue => {}
                Flow::Return(v) => return Ok(Some(v)),
            }
        }
        Ok(None)
    }

    fn eval_member(
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

    fn eval_match(&self, m: &MatchExpr, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
        let scrutinee = self.eval(&m.value, env)?;
        for arm in &m.arms {
            match &arm.pattern {
                xulo_core::ast::MatchPattern::Wildcard => {
                    return self.eval(&arm.value, env);
                }
                xulo_core::ast::MatchPattern::Literal(lit) => {
                    if literal_matches(lit, &scrutinee) {
                        return self.eval(&arm.value, env);
                    }
                }
                xulo_core::ast::MatchPattern::Enum(r) => {
                    if enum_ref_matches(&r.enum_name, &r.variant, &scrutinee) {
                        return self.eval(&arm.value, env);
                    }
                }
                xulo_core::ast::MatchPattern::EnumPayload {
                    variant,
                    bindings,
                    span,
                    ..
                } => {
                    if let Value::Enum { tag, payload, .. } = &scrutinee
                        && tag == variant
                        && let Some(payload) = payload
                    {
                        let child = Env::child(env);
                        bind_enum_payload(&child, bindings, payload, span)?;
                        return self.eval(&arm.value, &child);
                    }
                }
            }
        }
        Err(RunError::err("non-exhaustive match", m.span.clone()))
    }

    fn call(&self, call: &Call, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
        // Trait dispatch: call the mangled `impl_{Trait}_{Type}_{method}` free
        // function with the receiver as its first argument.
        if let Some(impl_name) = &call.trait_impl {
            let callee = env.borrow().get(impl_name).ok_or_else(|| {
                RunError::err(
                    format!("undefined impl function `{impl_name}`"),
                    call.span.clone(),
                )
            })?;
            return self.call_value(&callee, &call.arguments, env);
        }
        if let Some((enum_name, variant)) = call.enum_parts() {
            return self.enum_construct(enum_name, variant, &call.arguments, env);
        }
        if let Some(object) = &call.object {
            return self.method_call(call, object, env);
        }
        match env.borrow().get(&call.callee) {
            Some(callee) => self.call_value(&callee, &call.arguments, env),
            None => Err(RunError::err(
                format!("undefined function `{}`", call.callee),
                call.span.clone(),
            )),
        }
    }

    fn method_call(
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
            _ => None,
        };
        match callee {
            Some(v) => self.call_value(&v, &call.arguments, env),
            None => Err(RunError::err(
                format!("`{method}` is not a function of {}", receiver.kind_name()),
                call.span.clone(),
            )),
        }
    }

    fn enum_construct(
        &self,
        enum_name: &str,
        variant: &str,
        args: &[CallArg],
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        if args.is_empty() {
            return Err(RunError::err(
                format!("enum constructor `{enum_name}::{variant}` requires at least one argument"),
                0..0,
            ));
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
                if args.iter().any(|a| a.name.is_some()) {
                    return Err(RunError::err(
                        "named arguments are not supported for builtin functions",
                        0..0,
                    ));
                }
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

    fn eval_args(&self, args: &[CallArg], env: &Rc<RefCell<Env>>) -> Result<Vec<Value>, RunError> {
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
    fn bind_args(
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

    fn default_param(&self, param: &Param, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
        match &param.default {
            Some(expr) => self.eval(expr, env),
            None => Ok(Value::Null),
        }
    }

    /// Bind `args` (in parameter order, `None` for omitted) into the callee
    /// scope, evaluating omitted parameters' default expressions *inside that
    /// scope* — earlier parameters are already defined when a later default
    /// runs, matching the JS codegen (`function f(a, b = a)`).
    fn bind_callee_args(
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
            env.borrow_mut().define(&param.name, value);
        }
        Ok(())
    }

    fn run_function(
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
                    interp.run_body(&func.body, &env, func.return_type.is_some())
                })
            }));
        }
        let env = Env::child(&func.closure);
        self.bind_callee_args(&env, &func.params, args)?;
        self.run_body(&func.body, &env, func.return_type.is_some())
    }

    fn call_fn(
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
                        interp.run_body(&body, &env, return_type.is_some())
                    })
                }));
            }
            let env = Env::child(call_env);
            self.bind_callee_args(&env, &f.params, bound)?;
            self.run_body(&f.body, &env, f.return_type.is_some())
        })();
        self.call_depth.set(depth);
        result
    }

    /// Execute a function body in its call environment, honoring the implicit
    /// return rule: with a declared return type, a trailing expression without
    /// a `;` is the function's value.
    fn run_body(
        &self,
        body: &Block,
        env: &Rc<RefCell<Env>>,
        declared_return: bool,
    ) -> Result<Value, RunError> {
        if declared_return
            && let Some(Statement::Expr(last)) = body.statements.last()
            && !last.has_semicolon
        {
            for stmt in &body.statements[..body.statements.len() - 1] {
                match self.exec_stmt(stmt, env)? {
                    Flow::Continue => {}
                    Flow::Return(v) => return Ok(v),
                }
            }
            return self.eval(&last.expr, env);
        }
        match self.exec_stmts(&body.statements, env)? {
            Flow::Continue => Ok(Value::Null),
            Flow::Return(v) => Ok(v),
        }
    }
}

impl Default for Interpreter {
    fn default() -> Self {
        Self::new()
    }
}

/// Run `f` against the interpreter a coroutine is currently executing in.
/// The `'static` coroutine closures cannot capture `&Interpreter`, so they reach
/// it through `CURRENT_INTERP` (set around every `resume`).
fn with_interp<R>(f: impl FnOnce(&Interpreter) -> R) -> R {
    CURRENT_INTERP.with(|c| {
        let ptr = c
            .get()
            .expect("interpreter pointer is set while a coroutine runs");
        // SAFETY: CURRENT_INTERP points at a live interpreter and is cleared
        // when no coroutine is executing; the pointer stays valid for the whole
        // resume, and execution is single-threaded.
        f(unsafe { &*ptr })
    })
}

/// Whether a pattern literal (including a `"Enum.Variant"` string) matches a
/// value, mirroring the JS codegen: `__m === "Enum.Variant"` or
/// `__m.tag === "Variant"`.
fn literal_matches(lit: &Literal, value: &Value) -> bool {
    match lit {
        Literal::String(s) => match value {
            Value::Enum { enum_name, tag, .. } => format!("{enum_name}.{tag}") == *s || *tag == *s,
            Value::String(v) => v == s,
            _ => false,
        },
        Literal::Number(n) => matches!(value, Value::Number(v) if v == n),
        Literal::Boolean(b) => matches!(value, Value::Boolean(v) if v == b),
        Literal::Null => matches!(value, Value::Null),
        Literal::List(_) | Literal::Object(_) => false,
    }
}

/// Convert a list/string index value into a `usize`, rejecting fractional,
/// negative, and NaN indices. Silently truncating (`-1.0 as usize` is 0 in
/// Rust) used to read the wrong element — the JS path yields `undefined` for
/// those, so the native runtime must not guess either.
fn list_index(n: f64, span: &std::ops::Range<usize>) -> Result<usize, RunError> {
    if n.is_nan() || n < 0.0 || n.fract() != 0.0 {
        return Err(RunError::err(
            format!("index must be a non-negative integer, found {n}"),
            span.clone(),
        ));
    }
    Ok(n as usize)
}

/// Whether an enum-pattern matches: the JS codegen accepts both a
/// `"Enum.Variant"` string and any value whose tag is the variant.
fn enum_ref_matches(enum_name: &str, variant: &str, value: &Value) -> bool {
    match value {
        Value::Enum { tag, .. } => tag == variant,
        Value::String(s) => s == &format!("{enum_name}.{variant}") || s == variant,
        _ => false,
    }
}

/// Bind a matched enum payload into `env`: a single binding receives the value
/// itself; several bindings receive the elements of a list payload.
fn bind_enum_payload(
    env: &Rc<RefCell<Env>>,
    bindings: &[String],
    payload: &Value,
    span: &std::ops::Range<usize>,
) -> Result<(), RunError> {
    if bindings.len() == 1 {
        let name = &bindings[0];
        if name != "_" {
            env.borrow_mut().define(name, payload.clone());
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
        env.borrow_mut().define(name, value);
    }
    Ok(())
}

/// Find the program's `main`, including `export fn main` and
/// `export default fn main`.
fn main_fn(program: &Program) -> Option<&FnDef> {
    program.statements.iter().find_map(|s| match s {
        Statement::Fn(f) if f.name == "main" => Some(f),
        Statement::Export(export) => match &export.item {
            xulo_core::ast::ExportItem::Fn(f) if f.name == "main" => Some(f),
            xulo_core::ast::ExportItem::Default(inner) => match inner.as_ref() {
                xulo_core::ast::ExportItem::Fn(f) if f.name == "main" => Some(f),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    })
}

fn is_component_return(ret: &Option<Type>) -> bool {
    matches!(ret, Some(Type::Named(n)) if n == "Component")
}

/// The runtime value behind an export item (functions and variables; types and
/// enums carry no runtime binding, and bare name lists re-export the first one).
fn export_value(item: &xulo_core::ast::ExportItem, env: &Rc<RefCell<Env>>) -> Option<Value> {
    match item {
        xulo_core::ast::ExportItem::Fn(f) => env.borrow().get(&f.name),
        xulo_core::ast::ExportItem::Let(b) => env.borrow().get(&b.name),
        xulo_core::ast::ExportItem::Names(names) => names.first().and_then(|n| env.borrow().get(n)),
        _ => None,
    }
}

/// Gather a module's exports: named bindings plus an optional default, from the
/// bindings already defined by running its statements.
fn collect_exports(
    item: &xulo_core::ast::ExportItem,
    env: &Rc<RefCell<Env>>,
    bindings: &mut Vec<(String, Value)>,
    default: &mut Option<Value>,
) {
    match item {
        xulo_core::ast::ExportItem::Fn(f) => {
            if let Some(v) = env.borrow().get(&f.name) {
                bindings.push((f.name.clone(), v));
            }
        }
        xulo_core::ast::ExportItem::Let(b) => {
            if let Some(v) = env.borrow().get(&b.name) {
                bindings.push((b.name.clone(), v));
            }
        }
        xulo_core::ast::ExportItem::Names(names) => {
            for name in names {
                if let Some(v) = env.borrow().get(name) {
                    bindings.push((name.clone(), v));
                }
            }
        }
        xulo_core::ast::ExportItem::Type(_) | xulo_core::ast::ExportItem::Trait(_) => {}
        xulo_core::ast::ExportItem::Enum(e) => {
            bindings.push((e.name.clone(), enum_value(e)));
        }
        xulo_core::ast::ExportItem::Default(inner) => {
            *default = export_value(inner, env);
        }
    }
}

/// The runtime value of an enum, mirroring the JS shape — an object of
/// `"Enum.Variant"` strings — so member access on the value (`Theme.Dark`)
/// and printing behave alike. Enum *construction* (`Theme::Dark`) is
/// name-based and does not look this value up.
fn enum_value(e: &xulo_core::ast::EnumDef) -> Value {
    let fields = e
        .variants
        .iter()
        .map(|v| {
            (
                v.name.clone(),
                Value::String(format!("{}.{}", e.name, v.name)),
            )
        })
        .collect();
    Value::Object(Rc::new(RefCell::new(fields)))
}
