use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;

use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, Yielder};

use xulo_core::ast::{
    AssignTarget, BinaryOp, BinaryOperator, BindingPattern, Block, Call, ComponentStmt, EffectStmt,
    Expression, FnDef, ForStmt, IfExpr, ImplDecl, LetBinding, Literal, MatchExpr, ObjectField,
    Param, Program, ReturnStmt, Statement, StoreStmt, TryStmt, Type, UiElement, UnaryOperator,
    impl_fn_name,
};
use xulo_core::error::{ErrorKind, XuloError};

use crate::env::Env;
use crate::value::{FunctionValue, Value, equal, is_truthy};

pub use xulo_core::ast::CallArg;

/// Signature of a builtin function (`print`, `str`) or a captured native like a
/// `$` binding's `onChange` setter. A reference-counted trait object so a
/// builtin can close over runtime cells.
pub type NativeFn = Rc<dyn Fn(&Interpreter, &[Value]) -> Result<Value, RunError>>;

/// The outcome of running a statement: keep executing, return from the
/// enclosing function, break out of a loop, or continue to the next loop
/// iteration. Thrown values are carried by [`RunError::Throw`] so a
/// surrounding `try`/`catch` can intercept them.
pub enum Flow {
    Continue,
    Return(Value),
    Break,
    LoopContinue,
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

/// Settles one `send` task: fulfills the handle's promise with the reply
/// value, or rejects it when the message failed or the service stopped.
type ServiceSettle = Rc<dyn Fn(&Interpreter, Result<Value, RunError>)>;

/// One queued message: the deep-copied message value and the settle function
/// of the `send` that enqueued it.
struct ServiceMsg {
    message: Value,
    settle: ServiceSettle,
}

/// A live service started by `spawn.process Type { … }`: its state object
/// (the deep-copied field initializers, mutated in place by `on_message`),
/// the FIFO message queue, and the busy/stopped flags that gate the pump
/// (spec `concurrency.md`).
struct ServiceInstance {
    type_name: String,
    /// The service type's `on_message` function, resolved through the spawn
    /// site's scope chain (a module-level struct lives in the module env, not
    /// the interpreter root).
    on_message: Option<Value>,
    state: Value,
    queue: RefCell<VecDeque<ServiceMsg>>,
    busy: Cell<bool>,
    stopped: Cell<bool>,
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
    /// `Task.timeout` deadlines: `(deadline, combined promise, the value it
    /// settles with)`. `drive` fires due entries once no task is ready and
    /// sleeps to the earliest live one in between; an entry whose promise
    /// settled before its deadline is pruned instead of waited for (the
    /// wrapped task won the race).
    timers: RefCell<Vec<TimerEntry>>,
    /// The task currently executing (the inline-resumed coroutine).
    current_task: Cell<Option<usize>>,
    /// Each task's yielder pointer, stashed by the coroutine on first entry.
    task_yielder: RefCell<Vec<Option<*const Yielder<Control, ()>>>>,
    /// Reusable task slots whose coroutine has completed. `spawn_async` pops
    /// here before growing `tasks`, keeping the vector bounded for programs
    /// that spawn many short-lived async calls.
    task_free: RefCell<Vec<usize>>,
    /// Active call depth (see [`MAX_CALL_DEPTH`]).
    call_depth: Cell<usize>,
    /// Names of `struct` types registered in this interpreter. `method_call`
    /// falls back to these objects (which hold inherent impl methods) when a
    /// method is not found directly on a receiver instance.
    struct_names: RefCell<Vec<String>>,
    /// `@Memo let` value cache keyed by the binding's `name_span.start`: each
    /// site holds `(deps values, cached result)` entries, scanned by value
    /// equality. Live for the whole run (the native runtime has no re-render
    /// loop, so repeated calls with unchanged deps reuse the cached value).
    memo_cache: RefCell<std::collections::HashMap<usize, Vec<MemoEntry>>>,
    /// The render tree the entry `main` produced when it returns a `View`
    /// (see [`Interpreter::take_root_view`]). Kept out of `out` so rendering
    /// hosts can pick it up after execution.
    view: RefCell<Option<Value>>,
    /// The entry module's scope, kept so the UI host can re-invoke `main` to
    /// re-render (see [`Interpreter::rerender_main`]).
    entry_env: RefCell<Option<Rc<RefCell<Env>>>>,
    /// `@State` cells keyed by their binding site (source offset), so a
    /// re-render reuses the cell instead of re-initializing it: state survives
    /// across `rerender_main` calls. Multiple instances of the same component
    /// share a cell (no instance identity is tracked yet).
    state_store: RefCell<std::collections::HashMap<usize, Rc<RefCell<Value>>>>,
    /// Services started by `spawn.process`, kept alive for the whole run:
    /// a service is not bound to the block that started it (spec
    /// `concurrency.md`, service lifetime).
    services: RefCell<Vec<Rc<ServiceInstance>>>,
}

/// A cached `@Memo` result: the dependency values it was computed from, plus
/// the computed value. Scanned by value equality (the entry lists are short).
type MemoEntry = (Vec<Value>, Value);

/// One `Task.timeout` deadline: when it fires, the combined promise it
/// settles, and the value that settles it.
type TimerEntry = (std::time::Instant, Rc<RefCell<crate::value::Promise>>, Value);

/// Maximum nested interpreter calls. Guards against unbounded recursion, which
/// otherwise crashes the process: sync recursion overflows the host stack with
/// a `stack overflow` abort, and async recursion spawns one coroutine stack per
/// level (see [`COROUTINE_STACK_SIZE`]) until allocation fails. Reaching the
/// limit returns a clean runtime error instead. The depth is counted at call
/// time, so an `async` chain that suspends at each `await` is bounded too.
///
/// The limit must leave headroom for *debug* builds on small host stacks: an
/// unoptimized interpreter frame is ~25 KiB, and test/embedding threads default
/// to a 2 MiB stack, so 128 × ~25 KiB stays safely under it.
pub const MAX_CALL_DEPTH: usize = 128;

/// Stack size for each async task's coroutine. A maximal async chain keeps
/// every suspended level's stack alive at once (up to [`MAX_CALL_DEPTH`] × this
/// size), so this is the memory multiplier for deep `await` recursion.
///
/// The two profiles size it differently because unoptimized interpreter frames
/// dominate in debug builds:
/// - Debug keeps the historical 1 MiB: a sync call of ~25 KiB per frame allows a
///   recursion chain of only ~40 levels inside one async body, and shrinking the
///   stack would move that crash boundary into ordinary dev workloads.
/// - Release frames are ~0.3 KiB, so 256 KiB (hundreds of call levels of
///   headroom) is ample while cutting a maximal chain from 128 MiB to 32 MiB.
pub const COROUTINE_STACK_SIZE: usize = if cfg!(debug_assertions) {
    1024 * 1024
} else {
    256 * 1024
};

/// Break the Rc cycle that top-level (and nested) function declarations create:
/// `env -> "f" -> FunctionValue { closure: env }`. The env graph outlives the
/// interpreter otherwise (known issue D11), leaking in REPL/library hosts that
/// create and drop interpreters repeatedly. Clearing the root env's bindings
/// releases the function values — and any other values bound at the top level —
/// so the whole graph is reclaimed. Nested child envs hang off the root through
/// the strong `parent` link and drop together with the values that own them.
impl Drop for Interpreter {
    fn drop(&mut self) {
        self.global.borrow_mut().clear();
    }
}

/// Normalize a combinator operand to a promise: tasks stay as they are,
/// plain values become already-fulfilled promises (spec `Task.resolve`).
fn as_promise(value: &Value) -> Rc<RefCell<crate::value::Promise>> {
    match value {
        Value::Promise(p) => p.clone(),
        other => Rc::new(RefCell::new(crate::value::Promise {
            state: crate::value::PromiseState::Fulfilled(other.clone()),
            awaiters: VecDeque::new(),
        })),
    }
}

/// Settle a shared combinator promise once: the first settle wins, later ones
/// are discarded (spec `error-handling.md`).
fn settle_promise_once(
    interp: &Interpreter,
    promise: &Rc<RefCell<crate::value::Promise>>,
    result: Result<Value, RunError>,
) {
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
}

/// `Task.all([t, u])`: the combined task settles when every input has settled
/// and carries the results in argument order; the first rejection rejects it.
fn native_task_all(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let list = match args.first() {
        Some(Value::List(list)) => list.clone(),
        other => {
            return Err(RunError::err(
                format!(
                    "`Task.all` expects a list of tasks, found {}",
                    other.map(|v| v.kind_name()).unwrap_or("nothing".into())
                ),
                0..0,
            ));
        }
    };
    let promises: Vec<Rc<RefCell<crate::value::Promise>>> =
        list.borrow().iter().map(as_promise).collect();
    Ok(interp.spawn_async(move |_yielder| {
        with_interp(|interp| {
            let mut results = Vec::with_capacity(promises.len());
            for promise in promises {
                results.push(interp.await_promise(promise, 0..0)?);
            }
            Ok(Value::List(Rc::new(RefCell::new(results))))
        })
    }))
}

/// `Task.race([t, u])`: the combined task settles with the first input to
/// settle; the others keep running and their results are discarded.
fn native_task_race(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let list = match args.first() {
        Some(Value::List(list)) => list.clone(),
        other => {
            return Err(RunError::err(
                format!(
                    "`Task.race` expects a list of tasks, found {}",
                    other.map(|v| v.kind_name()).unwrap_or("nothing".into())
                ),
                0..0,
            ));
        }
    };
    let promises: Vec<Rc<RefCell<crate::value::Promise>>> =
        list.borrow().iter().map(as_promise).collect();
    let combined = Rc::new(RefCell::new(crate::value::Promise {
        state: crate::value::PromiseState::Pending,
        awaiters: VecDeque::new(),
    }));
    for promise in promises {
        let combined = combined.clone();
        interp.spawn_async(move |_yielder| {
            with_interp(|interp| {
                // A rejection of one racer settles the combined task rejected
                // only if it is still pending (first settle wins).
                let result = interp.await_promise(promise, 0..0);
                settle_promise_once(interp, &combined, result);
                Ok(Value::Null)
            })
        });
    }
    Ok(Value::Promise(combined))
}

/// `Task.resolve(v)`: an already-fulfilled task holding `v`.
fn native_task_resolve(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let value = args.first().cloned().unwrap_or(Value::Null);
    Ok(Value::Promise(Rc::new(RefCell::new(
        crate::value::Promise {
            state: crate::value::PromiseState::Fulfilled(value),
            awaiters: VecDeque::new(),
        },
    ))))
}

/// The built-in `Error` value (spec `error-handling.md`): an object carrying
/// its one `message` field, so `e.message` reads it back.
fn error_value(message: Rc<str>) -> Value {
    Value::Object(Rc::new(RefCell::new(vec![(
        "message".to_string(),
        Value::String(message),
    )])))
}

/// `Error(message)`: construct the base error type (the semantic pass checks
/// the argument is a string; the runtime formats anything for robustness).
fn native_error(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let message = match args.first() {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.format().into(),
        None => String::new().into(),
    };
    Ok(error_value(message))
}

/// `Result::Ok(value)` as a runtime enum value. `Task.timeout` wraps outcomes
/// in the built-in `Result`, which needs no runtime registration: enum
/// construction and matching are generic over the enum's name.
fn result_ok(value: Value) -> Value {
    Value::Enum {
        enum_name: "Result".to_string(),
        tag: "Ok".to_string(),
        payload: Some(Box::new(value)),
    }
}

/// `Result::Err(error)` as a runtime enum value.
fn result_err(error: Value) -> Value {
    Value::Enum {
        enum_name: "Result".to_string(),
        tag: "Err".to_string(),
        payload: Some(Box::new(error)),
    }
}

/// `Task.timeout(t, ms)`: settles with `Result::Ok(t's result)` when `t`
/// finishes within `ms`, or with `Result::Err(Error("timed out"))` when the
/// deadline elapses first (spec `error-handling.md`). The deadline is a timer
/// entry `drive` fires once nothing else is ready to run; a rejection of `t`
/// propagates as a rejection (the runtime has no separate cancelled state).
///
/// The wrapped task cannot be interrupted — there is no cancellation — so
/// after a timeout it runs on: its late result finds the combined promise
/// already settled and is discarded, and its timer entry is pruned by `drive`
/// instead of being waited for.
fn native_task_timeout(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let Some(first) = args.first() else {
        return Err(RunError::err(
            "`Task.timeout` requires a task argument",
            0..0,
        ));
    };
    let inner = as_promise(first);
    let ms = match args.get(1) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => *n,
        other => {
            return Err(RunError::err(
                format!(
                    "`Task.timeout` expects a millisecond count of type number, found {}",
                    other.map(|v| v.kind_name()).unwrap_or("nothing".into())
                ),
                0..0,
            ));
        }
    };
    // Clamp to the range a deadline addition can hold so an absurd input
    // cannot overflow `Instant + Duration`.
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_millis(ms.min(u32::MAX as f64) as u64);
    let combined = Rc::new(RefCell::new(crate::value::Promise {
        state: crate::value::PromiseState::Pending,
        awaiters: VecDeque::new(),
    }));
    interp.timers.borrow_mut().push((
        deadline,
        combined.clone(),
        result_err(error_value("timed out".into())),
    ));
    let settled = combined.clone();
    interp.spawn_async(move |_yielder| {
        with_interp(|interp| {
            // First settle wins: the waiter's wrap of the task result races
            // the timer, and the loser is discarded.
            let wrapped = match interp.await_promise(inner, 0..0) {
                Ok(v) => Ok(result_ok(v)),
                Err(e) => Err(e),
            };
            settle_promise_once(interp, &settled, wrapped);
            Ok(Value::Null)
        })
    });
    Ok(Value::Promise(combined))
}

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
        Some(v) => Value::String(v.format().into()),
        None => Value::String(String::new().into()),
    })
}

/// `assert(condition, message)`: a falsy condition stops the program with
/// `panic(message)` (an uncaught throw of the message string), mirroring
/// `Expression::Panic`. Semantic checking already enforced the
/// `Boolean`/`String` signature; the runtime check is defensive.
fn native_assert(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let condition = args.first().cloned().unwrap_or(Value::Null);
    if is_truthy(&condition) {
        return Ok(Value::Null);
    }
    let message = match args.get(1) {
        Some(Value::String(s)) => Value::String(s.clone()),
        Some(v) => Value::String(v.format().into()),
        None => Value::String("assertion failed".into()),
    };
    Err(RunError::Throw(message))
}

fn native_array_new(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let size = match args.first() {
        Some(Value::Number(n)) => {
            if n.is_nan() || *n < 0.0 || n.fract() != 0.0 {
                return Err(RunError::err(
                    format!("Array.new() size must be a non-negative integer, found {n}"),
                    0..0,
                ));
            }
            *n as usize
        }
        Some(other) => {
            return Err(RunError::err(
                format!(
                    "Array.new() size must be a number, found {}",
                    other.kind_name()
                ),
                0..0,
            ));
        }
        None => {
            return Err(RunError::err(
                "Array.new() requires at least 1 argument",
                0..0,
            ));
        }
    };
    let fill = if args.len() > 1 {
        args[1].clone()
    } else {
        Value::Null
    };
    Ok(Value::List(Rc::new(RefCell::new(vec![fill; size]))))
}

fn native_array_fill(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let n = match args.first() {
        Some(Value::Number(n)) => {
            if n.is_nan() || *n < 0.0 || n.fract() != 0.0 {
                return Err(RunError::err(
                    format!("Array.fill() count must be a non-negative integer, found {n}"),
                    0..0,
                ));
            }
            *n as usize
        }
        Some(other) => {
            return Err(RunError::err(
                format!(
                    "Array.fill() count must be a number, found {}",
                    other.kind_name()
                ),
                0..0,
            ));
        }
        None => {
            return Err(RunError::err("Array.fill() requires 2 arguments", 0..0));
        }
    };
    let fill = args.get(1).cloned().unwrap_or(Value::Null);
    Ok(Value::List(Rc::new(RefCell::new(vec![fill; n]))))
}

fn native_array_from(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    match args.first() {
        Some(Value::List(list)) => Ok(Value::List(Rc::new(RefCell::new(list.borrow().clone())))),
        Some(other) => Err(RunError::err(
            format!("Array.clone() expected a list, found {}", other.kind_name()),
            0..0,
        )),
        None => Err(RunError::err("Array.clone() requires 1 argument", 0..0)),
    }
}

fn block_has_closure(block: &Block) -> bool {
    block.statements.iter().any(stmt_has_closure)
}

fn stmt_has_closure(stmt: &Statement) -> bool {
    match stmt {
        Statement::Fn(_) => true,
        // A `test fn` body is inert: nothing can reach it, so its closures
        // never exist at run time.
        Statement::Test(_) => false,
        Statement::Let(l) => {
            l.value.as_ref().is_some_and(expr_has_closure)
                || l.memo_deps.iter().flatten().any(expr_has_closure)
        }
        Statement::Return(r) => r.value.as_ref().is_some_and(expr_has_closure),
        Statement::For(f) => expr_has_closure(&f.iterable) || block_has_closure(&f.body),
        Statement::While(w) => expr_has_closure(&w.condition) || block_has_closure(&w.body),
        Statement::Assign(a) => {
            let target = match &a.target {
                AssignTarget::Name(_) => false,
                AssignTarget::Member(object, _) => expr_has_closure(object),
                AssignTarget::Index(object, index) => {
                    expr_has_closure(object) || expr_has_closure(index)
                }
            };
            target || expr_has_closure(&a.value)
        }
        Statement::TypeAlias(_)
        | Statement::Enum(_)
        | Statement::Struct(_)
        | Statement::Import(_)
        | Statement::Environment(_)
        | Statement::Trait(_)
        | Statement::Break
        | Statement::Continue => false,
        Statement::Expr(e) => expr_has_closure(&e.expr),
        Statement::Block(b) => block_has_closure(b),
        Statement::Defer(d) => match &d.body {
            xulo_core::ast::DeferBody::Block(b) => block_has_closure(b),
            xulo_core::ast::DeferBody::Expr(e) => expr_has_closure(e),
        },
        Statement::Try(t) => block_has_closure(&t.try_block) || block_has_closure(&t.catch_block),
        Statement::Throw(e) => expr_has_closure(e),
        Statement::Export(e) => match &e.item {
            xulo_core::ast::ExportItem::Fn(_) => true,
            xulo_core::ast::ExportItem::Let(l) => l.value.as_ref().is_some_and(expr_has_closure),
            xulo_core::ast::ExportItem::Enum(_)
            | xulo_core::ast::ExportItem::Type(_)
            | xulo_core::ast::ExportItem::Struct(_)
            | xulo_core::ast::ExportItem::Trait(_)
            | xulo_core::ast::ExportItem::Names(_) => false,
        },
        Statement::State(s) => s.binding.value.as_ref().is_some_and(expr_has_closure),
        Statement::Store(s) => expr_has_closure(&s.value),
        Statement::Effect(_) => true,
        Statement::Component(c) => {
            c.args.iter().any(|a| expr_has_closure(&a.value))
                || c.children.iter().any(ui_has_closure)
        }
        Statement::Impl(_) => true,
    }
}

fn expr_has_closure(expr: &Expression) -> bool {
    match expr {
        Expression::Literal { value, .. } => match value {
            Literal::List(items) => items.iter().any(expr_has_closure),
            Literal::Object(fields) => fields.iter().any(|field| match field {
                ObjectField::Field { value, .. } => expr_has_closure(value),
                ObjectField::Spread { value } => expr_has_closure(value),
            }),
            Literal::String(_) | Literal::Number(_) | Literal::Boolean(_) | Literal::Null => false,
        },
        Expression::Identifier { .. } | Expression::EnumRef(_) | Expression::Binding { .. } => {
            false
        }
        Expression::BinaryOp(b) => expr_has_closure(&b.left) || expr_has_closure(&b.right),
        Expression::Unary(u) => expr_has_closure(&u.operand),
        Expression::Call(c) => {
            c.object.as_deref().is_some_and(expr_has_closure)
                || c.arguments.iter().any(|a| expr_has_closure(&a.value))
        }
        Expression::If(e) => {
            expr_has_closure(&e.condition)
                || block_has_closure(&e.then_branch)
                || e.else_branch.as_ref().is_some_and(block_has_closure)
        }
        Expression::Ternary(t) => {
            expr_has_closure(&t.condition)
                || expr_has_closure(&t.then_value)
                || expr_has_closure(&t.else_value)
        }
        Expression::Match(m) => {
            expr_has_closure(&m.value)
                || m.arms.iter().any(|arm| {
                    arm.guard.as_ref().is_some_and(expr_has_closure)
                        || match_body_has_closure(&arm.value)
                })
        }
        Expression::Member(m) => expr_has_closure(&m.object),
        Expression::Index(i) => expr_has_closure(&i.object) || expr_has_closure(&i.index),
        Expression::Nullish(n) => expr_has_closure(&n.left) || expr_has_closure(&n.right),
        Expression::Range(r) => expr_has_closure(&r.start) || expr_has_closure(&r.end),
        Expression::Await { expr, .. } => expr_has_closure(expr),
        Expression::FnExpr(_) => true,
        Expression::Spread { expr, .. } => expr_has_closure(expr),
        Expression::CallValue(c) => {
            expr_has_closure(&c.callee) || c.arguments.iter().any(|a| expr_has_closure(&a.value))
        }
        Expression::Component(c) => {
            c.args.iter().any(|a| expr_has_closure(&a.value))
                || c.children.iter().any(ui_has_closure)
        }
        Expression::Tuple { items, .. } => items.iter().any(expr_has_closure),
        Expression::TypedMap(tm) => tm
            .entries
            .iter()
            .any(|e| expr_has_closure(&e.key) || expr_has_closure(&e.value)),
        Expression::Is(is) => expr_has_closure(&is.expr),
        Expression::Panic { expr, .. } => expr_has_closure(expr),
        Expression::Propagate { expr, .. } => expr_has_closure(expr),
        Expression::Arrow(_) => true,
        Expression::Spawn(s) => match &s.body {
            xulo_core::ast::SpawnBody::Async(b) => block_has_closure(b),
            xulo_core::ast::SpawnBody::Service { fields, .. } => {
                fields.iter().any(|f| expr_has_closure(&f.value))
            }
        },
        Expression::Lock(l) => block_has_closure(&l.body),
    }
}

fn match_body_has_closure(body: &xulo_core::ast::MatchBody) -> bool {
    match body {
        xulo_core::ast::MatchBody::Expr(e) => expr_has_closure(e),
        xulo_core::ast::MatchBody::Block(b) => block_has_closure(b),
    }
}

fn ui_has_closure(el: &xulo_core::ast::UiElement) -> bool {
    match el {
        xulo_core::ast::UiElement::Component(c) => {
            c.args.iter().any(|a| expr_has_closure(&a.value))
                || c.children.iter().any(ui_has_closure)
        }
        xulo_core::ast::UiElement::Text(_) => false,
        xulo_core::ast::UiElement::Expr(e) => expr_has_closure(e),
        xulo_core::ast::UiElement::If {
            condition,
            then_branch,
            else_branch,
        } => {
            expr_has_closure(condition)
                || then_branch.iter().any(ui_has_closure)
                || else_branch
                    .as_ref()
                    .is_some_and(|branch| branch.iter().any(ui_has_closure))
        }
        xulo_core::ast::UiElement::For { iterable, body, .. } => {
            expr_has_closure(iterable) || body.iter().any(ui_has_closure)
        }
        xulo_core::ast::UiElement::Group(children) => children.iter().any(ui_has_closure),
    }
}

impl Interpreter {
    /// A fresh interpreter with `print`/`str` builtins registered in the global
    /// scope (user declarations of the same name shadow them).
    pub fn new() -> Self {
        let global = Env::root();
        global
            .borrow_mut()
            .define("print", Value::Native(Rc::new(native_print)), false);
        global
            .borrow_mut()
            .define("println", Value::Native(Rc::new(native_print)), false);
        global
            .borrow_mut()
            .define("str", Value::Native(Rc::new(native_str)), false);
        // The fourth intrinsic (spec builtins/intrinsic-functions.md):
        // `assert(condition, message)` — false stops the program with
        // `panic(message)`.
        global
            .borrow_mut()
            .define("assert", Value::Native(Rc::new(native_assert)), false);
        // `Error(message)`: the built-in base error type (spec
        // `error-handling.md`), constructed like any other builtin call.
        global
            .borrow_mut()
            .define("Error", Value::Native(Rc::new(native_error)), false);
        // Array global: Array.new(size, fill?), Array.fill(n, v), Array.clone(iter)
        let array_methods = vec![
            (
                "create".to_string(),
                Value::Native(Rc::new(native_array_new)),
            ),
            (
                "fill".to_string(),
                Value::Native(Rc::new(native_array_fill)),
            ),
            (
                "clone".to_string(),
                Value::Native(Rc::new(native_array_from)),
            ),
        ];
        global.borrow_mut().define(
            "Array",
            Value::Object(Rc::new(RefCell::new(array_methods))),
            false,
        );
        // The `Task` combinator namespace (spec `builtins/intrinsic-functions.md`).
        let task_methods = vec![
            (
                "all".to_string(),
                Value::Native(Rc::new(native_task_all)),
            ),
            (
                "race".to_string(),
                Value::Native(Rc::new(native_task_race)),
            ),
            (
                "resolve".to_string(),
                Value::Native(Rc::new(native_task_resolve)),
            ),
            (
                "timeout".to_string(),
                Value::Native(Rc::new(native_task_timeout)),
            ),
        ];
        global.borrow_mut().define(
            "Task",
            Value::Object(Rc::new(RefCell::new(task_methods))),
            false,
        );
        Interpreter {
            out: RefCell::new(Vec::new()),
            global,
            tasks: RefCell::new(Vec::new()),
            ready: RefCell::new(VecDeque::new()),
            timers: RefCell::new(Vec::new()),
            current_task: Cell::new(None),
            task_yielder: RefCell::new(Vec::new()),
            task_free: RefCell::new(Vec::new()),
            call_depth: Cell::new(0),
            memo_cache: RefCell::new(std::collections::HashMap::new()),
            struct_names: RefCell::new(Vec::new()),
            view: RefCell::new(None),
            entry_env: RefCell::new(None),
            state_store: RefCell::new(std::collections::HashMap::new()),
            services: RefCell::new(Vec::new()),
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
                Statement::Struct(s) => {
                    self.struct_names.borrow_mut().push(s.name.clone());
                    global.borrow_mut().define(
                        &s.name,
                        Value::Object(Rc::new(RefCell::new(Vec::new()))),
                        false,
                    );
                }
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
            *self.entry_env.borrow_mut() = Some(global.clone());
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
    /// `main` surfaces like any other uncaught error. A `View` `main` runs
    /// headlessly: its render value is produced and dropped (the JS path hands
    /// it to `__xulo_mount`, which needs an external runtime the native one
    /// does not have).
    ///
    /// When `main` declares a `View` return type the produced render value is
    /// kept in `self.view`, retrievable via [`Interpreter::take_root_view`], so
    /// a rendering host (the UI framework) can pick it up after execution.
    fn run_main(&self, main: &FnDef, global: &Rc<RefCell<Env>>) -> Result<(), XuloError> {
        let is_view = matches!(
            &main.return_type,
            Some(Type::Named(name, _)) if name == "View"
        );
        let result = self.call_fn(main, &[], global);
        self.drive();
        match result {
            Ok(Value::Promise(promise)) => {
                let state = promise.borrow();
                match &state.state {
                    crate::value::PromiseState::Fulfilled(v) => {
                        if is_view {
                            *self.view.borrow_mut() = Some(v.clone());
                        }
                        Ok(())
                    }
                    crate::value::PromiseState::Rejected(e) => Err(match e {
                        RunError::Err(e) => e.clone(),
                        RunError::Throw(v) => self.uncaught(v),
                    }),
                    crate::value::PromiseState::Pending => Err(XuloError::new(
                        ErrorKind::Runtime,
                        "async `main` did not complete",
                    )),
                }
            }
            Ok(view) => {
                if is_view {
                    *self.view.borrow_mut() = Some(view);
                }
                Ok(())
            }
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
        let global = self.setup_module(program, imports)?;
        if run_main && let Some(main) = main_fn(program) {
            // Remember the entry scope so the UI host can re-invoke `main`
            // later to re-render (see [`Interpreter::rerender_main`]).
            *self.entry_env.borrow_mut() = Some(global.clone());
            self.run_main(main, &global).map_err(RunError::Err)?;
        } else {
            // A module without `main` (any dependency module, or a no-main
            // entry) still drains its async tasks, matching JS module
            // semantics where fire-and-forget async work completes.
            self.drive();
        }
        let mut bindings = Vec::new();
        for statement in &program.statements {
            if let Statement::Export(export) = statement {
                collect_exports(&export.item, &global, &mut bindings);
            }
        }
        Ok(ModuleExports { bindings })
    }

    /// Bind a module's imports, register its declarations, and execute its
    /// top-level statements (skipping `import`s, which the caller pre-binds).
    /// Shared by [`Interpreter::exec_module`] and [`Interpreter::run_test`].
    fn setup_module(
        &self,
        program: &Program,
        imports: &[(String, Value)],
    ) -> Result<Rc<RefCell<Env>>, RunError> {
        let global = Env::child(&self.global);
        for (name, value) in imports {
            global.borrow_mut().define(name, value.clone(), false);
        }
        for statement in &program.statements {
            match statement {
                Statement::Fn(f) => self.register_fn(f, &global),
                Statement::Impl(imp) => self.register_impl(imp, &global),
                Statement::Struct(s) => {
                    self.struct_names.borrow_mut().push(s.name.clone());
                    global.borrow_mut().define(
                        &s.name,
                        Value::Object(Rc::new(RefCell::new(Vec::new()))),
                        false,
                    );
                }
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
        Ok(global)
    }

    /// Execute a module exactly like [`Interpreter::exec_module`] with
    /// `run_main: false`, then invoke one `test fn` body. The test body sees
    /// the module the same way a function body does (spec testing.md).
    ///
    /// Pass = the body completes; an async test passes when its task settles
    /// fulfilled. Fail = the body panics/throws/`assert`s or the task settles
    /// rejected. Returns the lines the test printed.
    pub fn run_test(
        &self,
        program: &Program,
        imports: &[(String, Value)],
        test: &FnDef,
    ) -> Result<Vec<String>, XuloError> {
        let global = self.setup_module(program, imports).map_err(|e| match e {
            RunError::Err(e) => e,
            RunError::Throw(v) => self.uncaught(&v),
        })?;
        *self.entry_env.borrow_mut() = Some(global.clone());
        let result = self.call_fn(test, &[], &global);
        self.drive();
        match result {
            Ok(Value::Promise(promise)) => {
                let state = promise.borrow();
                match &state.state {
                    crate::value::PromiseState::Fulfilled(_) => Ok(self.out.take()),
                    crate::value::PromiseState::Rejected(e) => Err(match e {
                        RunError::Err(e) => e.clone(),
                        RunError::Throw(v) => self.uncaught(v),
                    }),
                    crate::value::PromiseState::Pending => Err(XuloError::new(
                        ErrorKind::Runtime,
                        format!("async test `{}` did not complete", test.name),
                    )),
                }
            }
            Ok(_) => Ok(self.out.take()),
            Err(RunError::Err(e)) => Err(e),
            Err(RunError::Throw(v)) => Err(self.uncaught(&v)),
        }
    }

    /// Take the collected `print` lines so far (e.g. across executed modules).
    pub fn take_output(&self) -> Vec<String> {
        self.out.take()
    }

    /// Take the render tree the entry `main` produced, if it declared a `View`
    /// return type. A non-`View` `main` (or a program without `main`) leaves
    /// the slot empty. The value is a render tree of opaque `{ name, props }`
    /// nodes; the UI framework converts it into a widget tree.
    pub fn take_root_view(&self) -> Option<Value> {
        self.view.take()
    }

    /// Invoke a callable value (a UI callback such as `Button.onClick`) with
    /// no arguments. State cells the callback writes to are shared with the
    /// component that declared them, so a following
    /// [`Interpreter::rerender_main`] observes the update.
    pub fn invoke(&self, callee: &Value) -> Result<Value, RunError> {
        self.call_value_with_values(callee, &[])
    }

    /// Re-invoke the entry `main` to rebuild the render tree, reusing the
    /// `@State` cells created by the previous run (see [`Interpreter::exec_state`]).
    /// Returns a copy of the freshly rendered tree (which stays in the
    /// interpreter for [`Interpreter::take_root_view`]); `None` when the
    /// program has no `View` `main` to re-render.
    pub fn rerender_main(&self, program: &Program) -> Result<Option<Value>, RunError> {
        let main = match main_fn(program) {
            Some(main) => main,
            None => return Ok(None),
        };
        let entry_env = self
            .entry_env
            .borrow()
            .clone()
            .ok_or_else(|| RunError::err("entry module was not executed", 0..0))?;
        self.run_main(main, &entry_env).map_err(RunError::Err)?;
        Ok(self.view.borrow().clone())
    }

    /// Append a line to the interpreter's output buffer. Used by native
    /// builtins that produce output other than through [`Interpreter::run`]
    /// (e.g. the REPL's colorizing echo hook).
    pub fn push_output(&self, line: String) {
        self.out.borrow_mut().push(line);
    }

    /// The shared root environment backing all top-level bindings (`print`,
    /// `str`, and every `fn`/`impl` registered via [`Interpreter::run`]).
    ///
    /// Exposed so embedders and tests can observe the root scope — for example,
    /// tracking its strong reference count across `drop` to diagnose the Rc
    /// cycle described in the runtime's memory tests.
    pub fn root_env(&self) -> Rc<RefCell<Env>> {
        self.global.clone()
    }

    /// Current length of the internal task-slot vector (live + completed slots).
    ///
    /// Hidden from public API docs; used by memory tests to assert that
    /// completed task slots are recycled rather than grown unboundedly.
    #[doc(hidden)]
    pub fn debug_task_slot_count(&self) -> usize {
        self.tasks.borrow().len()
    }

    fn register_export_fns(&self, item: &xulo_core::ast::ExportItem, env: &Rc<RefCell<Env>>) {
        if let xulo_core::ast::ExportItem::Fn(f) = item {
            self.register_fn(f, env);
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
    /// Suspend the current task until `promise` settles, resolving to its
    /// value (or failing with its rejection). Shared by `await` expressions
    /// and the service pump's `on_message` chaining.
    fn await_promise(
        &self,
        promise: Rc<RefCell<crate::value::Promise>>,
        span: std::ops::Range<usize>,
    ) -> Result<Value, RunError> {
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

    /// Run one message through the service's `on_message`: bind `self` to
    /// the service state and `reply` to a cell the pump reads back.
    fn invoke_on_message(
        &self,
        func: &Rc<FunctionValue>,
        state: Value,
        message: Value,
        reply_cell: &Rc<RefCell<Option<Value>>>,
    ) -> Result<Value, RunError> {
        let reply = {
            let cell = Rc::clone(reply_cell);
            Value::Native(Rc::new(move |_interp, args| {
                let v = args.first().cloned().unwrap_or(Value::Null);
                *cell.borrow_mut() = Some(v);
                Ok(Value::Null)
            }))
        };
        let bound: Vec<Option<Value>> = func
            .params
            .iter()
            .map(|param| {
                if param.name == "self" {
                    Some(state.clone())
                } else {
                    Some(message.clone())
                }
            })
            .collect();
        if func.is_async {
            let func = func.clone();
            return Ok(self.spawn_async(move |_yielder| {
                with_interp(|interp| {
                    let env = Env::child(&func.closure);
                    interp.bind_callee_args(&env, &func.params, bound)?;
                    env.borrow_mut().define("reply", reply, false);
                    interp.run_body(&func.body, &env, func.return_type.is_some())
                })
            }));
        }
        let env = Env::child(&func.closure);
        self.bind_callee_args(&env, &func.params, bound)?;
        env.borrow_mut().define("reply", reply, false);
        self.run_body(&func.body, &env, func.return_type.is_some())
    }

    /// Settle one message's send task, mark the service idle, and take the
    /// next queued message (arrival order, one at a time).
    fn finish_message(
        &self,
        service: &Rc<ServiceInstance>,
        settle: &ServiceSettle,
        reply_cell: &Rc<RefCell<Option<Value>>>,
        result: Result<Value, RunError>,
    ) {
        let outcome = result.map(|_| reply_cell.borrow().clone().unwrap_or(Value::Null));
        settle(self, outcome);
        service.busy.set(false);
        self.service_pump(service);
    }

    /// Process queued messages while the service is idle: one `on_message`
    /// runs to completion (through every `await`) before the next begins.
    fn service_pump(&self, service: &Rc<ServiceInstance>) {
        while !service.stopped.get() && !service.busy.get() {
            let Some(msg) = service.queue.borrow_mut().pop_front() else {
                break;
            };
            service.busy.set(true);
            let reply_cell = Rc::new(RefCell::new(None));
            let func = service.on_message.clone();
            let Some(Value::Function(func)) = func else {
                (msg.settle)(
                    self,
                    Err(RunError::err(
                        format!(
                            "service `{}` declares no `on_message`",
                            service.type_name
                        ),
                        0..0,
                    )),
                );
                service.busy.set(false);
                continue;
            };
            match self.invoke_on_message(&func, service.state.clone(), msg.message, &reply_cell) {
                Ok(Value::Promise(p)) => {
                    // An async `on_message` holds the service busy until it
                    // settles; the next message waits (spec concurrency.md).
                    let service = service.clone();
                    let settle = msg.settle;
                    self.spawn_async(move |_yielder| {
                        with_interp(|interp| {
                            let result = interp.await_promise(p, 0..0);
                            interp.finish_message(&service, &settle, &reply_cell, result);
                            Ok(Value::Null)
                        })
                    });
                    return;
                }
                Ok(_) => {
                    self.finish_message(service, &msg.settle, &reply_cell, Ok(Value::Null));
                }
                Err(e) => {
                    // A failing message settles its send as rejected; the
                    // service keeps handling what is queued.
                    (msg.settle)(self, Err(e));
                    service.busy.set(false);
                }
            }
        }
    }

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
        let coro = Coroutine::with_stack(
            DefaultStack::new(COROUTINE_STACK_SIZE).expect("failed to allocate async task stack"),
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
            let mut free = self.task_free.borrow_mut();
            let mut yielder_slots = self.task_yielder.borrow_mut();
            if let Some(id) = free.pop() {
                // Reuse a slot left by a completed task. Its yielder pointer is
                // stale (the coroutine stack is gone), so reset it to `None`.
                yielder_slots[id] = None;
                tasks[id] = Some(Task {
                    coro,
                    promise: promise_for_task,
                });
                id
            } else {
                tasks.push(Some(Task {
                    coro,
                    promise: promise_for_task,
                }));
                yielder_slots.push(None);
                tasks.len() - 1
            }
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
        // SAFETY invariant: a coroutine only ever runs inside this method, so
        // while it is executing the interpreter is necessarily alive. Dropping
        // an `Interpreter` drops every task it owns; nothing resumes them
        // afterward. The raw pointer captured by `spawn_async` cannot dangle.
        debug_assert_eq!(
            CURRENT_INTERP.with(|c| c.get()),
            Some(self as *const Interpreter)
        );
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
                // Free the slot for reuse. A returned task is never enqueued in
                // `ready` again (only suspended tasks are), so handing the id
                // to the next `spawn_async` is safe.
                self.task_free.borrow_mut().push(id);
                // Only trim on the *outermost* resume. A nested resume (a task
                // spawned while another coroutine runs) finds every running
                // coroutine's slot "taken" (`None`), so trimming there would
                // truncate live slots. Reuse (`task_free`) keeps the vector
                // bounded meanwhile.
                if prev.is_none() {
                    self.trim_completed_slots();
                }
            }
        }
    }

    /// Shrink `tasks`/`task_yielder` once every trailing slot is a completed
    /// one, and drop free-list ids that are now out of bounds.
    fn trim_completed_slots(&self) {
        let mut tasks = self.tasks.borrow_mut();
        if tasks.last().is_some_and(Option::is_none) {
            let mut len = tasks.len();
            while len > 0 && tasks[len - 1].is_none() {
                len -= 1;
            }
            tasks.truncate(len);
            drop(tasks);
            self.task_yielder.borrow_mut().truncate(len);
            self.task_free.borrow_mut().retain(|&id| id < len);
        }
    }

    /// Drain the ready queue (JS microtasks): resume each queued task in FIFO
    /// order until none is ready to run. With no task ready, the timer half of
    /// the loop takes over: fire deadlines that have passed, otherwise sleep
    /// until the next one (`Task.timeout` deadlines), and end when neither
    /// has work left.
    fn drive(&self) {
        loop {
            let next = self.ready.borrow_mut().pop_front();
            let Some((id, control)) = next else {
                if self.fire_due_timers() {
                    continue;
                }
                if !self.sleep_to_next_timer() {
                    break;
                }
                continue;
            };
            self.resume_task(id, control);
        }
    }

    /// Settle every timer whose deadline has passed. Returns whether any
    /// fired: a settle wakes awaiters, so the caller re-checks the ready queue.
    fn fire_due_timers(&self) -> bool {
        let now = std::time::Instant::now();
        let mut due = Vec::new();
        {
            let mut timers = self.timers.borrow_mut();
            let mut i = 0;
            while i < timers.len() {
                if timers[i].0 <= now {
                    due.push(timers.swap_remove(i));
                } else {
                    i += 1;
                }
            }
        }
        if due.is_empty() {
            return false;
        }
        for (_, promise, value) in due {
            // Settle-once: a promise the waiter already settled is a no-op.
            settle_promise_once(self, &promise, Ok(value));
        }
        true
    }

    /// Wait until the earliest pending deadline, returning `false` when there
    /// is none left to wait for (nothing else will wake the loop). Timers
    /// whose promise settled before their deadline are dropped here — the
    /// wrapped task won the race — so waiting for them cannot stall a program
    /// that is otherwise done.
    fn sleep_to_next_timer(&self) -> bool {
        let next = {
            let mut timers = self.timers.borrow_mut();
            timers.retain(|(_, promise, _)| {
                matches!(
                    promise.borrow().state,
                    crate::value::PromiseState::Pending
                )
            });
            timers.iter().map(|(deadline, _, _)| *deadline).min()
        };
        let Some(next) = next else {
            return false;
        };
        let wait = next.saturating_duration_since(std::time::Instant::now());
        if !wait.is_zero() {
            std::thread::sleep(wait);
        }
        true
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
    fn register_impl(&self, imp: &ImplDecl, closure: &Rc<RefCell<Env>>) {
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
                    obj.push((
                        format!("{}::{}", imp.trait_name, method.name),
                        func.clone(),
                    ));
                }
            }
        }
    }

    fn exec_stmt(&self, stmt: &Statement, env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
        match stmt {
            Statement::Fn(f) => {
                self.register_fn(f, env);
                Ok(Flow::Continue)
            }
            // A `test fn` body is inert (spec testing.md): the identifier
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
                // `Color::Red` is name-based and unaffected).
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
            Statement::Import(imp) => {
                if imp.type_only {
                    Ok(Flow::Continue)
                } else {
                    // Local imports are bound into the module scope by
                    // `exec_module` before statements run; any remaining import
                    // is an external package. The headless runtime has no
                    // external values, so bind the names to `null` placeholders
                    // — UI components still resolve by name (they build the
                    // props-object shape), and expression uses of a missing
                    // external value fail only if a program computes with it.
                    for name in import_binding_names(&imp.spec) {
                        if env.borrow().get(&name).is_none() {
                            env.borrow_mut().define(&name, Value::Null, false);
                        }
                    }
                    Ok(Flow::Continue)
                }
            }
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
            | xulo_core::ast::ExportItem::Struct(_)
            | xulo_core::ast::ExportItem::Trait(_)
            | xulo_core::ast::ExportItem::Names(_) => Ok(Flow::Continue),
        }
    }

    /// `@Memo let x = expr` value memoization: evaluate the deps, and reuse the
    /// cached result for this binding site when the deps are equal to a prior
    /// evaluation (`@Memo([])` / bare `@Memo` always reuse). The cache lives
    /// for the whole run and is a computation optimization only.
    fn memo_value(&self, binding: &LetBinding, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
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
    fn exec_state(&self, binding: &LetBinding, env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
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
    fn exec_store(&self, store: &StoreStmt, env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
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
    fn exec_effect(&self, effect: &EffectStmt, env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
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
    fn eval_component_stmt(
        &self,
        c: &ComponentStmt,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        self.local_component_call(&c.name, &c.args, &c.children, env)
    }

    /// Evaluate a single UI element into a renderable value.
    fn eval_ui_element(&self, el: &UiElement, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
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
                                    "for loop must iterate over a `list`, found {}",
                                    other.kind_name()
                                ),
                                0..0,
                            ));
                        }
                    },
                    other => {
                        return Err(RunError::err(
                            format!(
                                "for loop must iterate over a `list`, found {}",
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
    fn ui_children_value(
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
    fn local_component_call(
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
    fn exec_assign(
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

    fn exec_try(&self, t: &TryStmt, env: &Rc<RefCell<Env>>) -> Result<Flow, RunError> {
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

    fn eval(&self, expr: &Expression, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
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
            Expression::Propagate { expr, .. } => self.eval(expr, env),
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
                                if !matches!(
                                    p.state,
                                    crate::value::PromiseState::Pending
                                ) {
                                    return;
                                }
                                p.state = match &result {
                                    Ok(v) => crate::value::PromiseState::Fulfilled(v.clone()),
                                    Err(e) => crate::value::PromiseState::Rejected(e.clone()),
                                };
                                let awaiters = std::mem::take(&mut p.awaiters);
                                drop(p);
                                for awaiter in awaiters {
                                    interp.ready.borrow_mut().push_back((
                                        awaiter,
                                        Control::Resume(result.clone()),
                                    ));
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

    fn eval_literal(&self, lit: &Literal, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
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

    fn add(&self, left: Value, right: Value, bin: &BinaryOp) -> Result<Value, RunError> {
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

    /// Whether `i` is still inside a range with upper bound `end`:
    /// `..<` is half-open (i < end), `...` is closed (i <= end).
    fn range_has_next(&self, i: f64, end: f64, end_inclusive: bool) -> bool {
        if end_inclusive { i <= end } else { i < end }
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
    fn exec_prefix(
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

    fn eval_match(&self, m: &MatchExpr, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
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
    fn eval_match_body(
        &self,
        body: &xulo_core::ast::MatchBody,
        env: &Rc<RefCell<Env>>,
    ) -> Result<Value, RunError> {
        match body {
            xulo_core::ast::MatchBody::Expr(e) => self.eval(e, env),
            xulo_core::ast::MatchBody::Block(b) => self.block_value(b, env),
        }
    }

    fn call(&self, call: &Call, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
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
                    RunError::err(format!("malformed dynamic dispatch `{impl_name}`"), call.span.clone())
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
        if let Some((enum_name, variant)) = call.enum_parts() {
            return self.enum_construct(enum_name, variant, &call.arguments, env);
        }
        if let Some(object) = &call.object {
            return self.method_call(call, object, env);
        }
        let callee = env.borrow().get(&call.callee);
        match callee {
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
            Value::List(list) => {
                return self.list_method(method, list.clone(), &call.arguments, &call.span, env);
            }
            _ => None,
        };
        match callee {
            Some(v) => self.call_value(&v, &call.arguments, env),
            None => {
                // Inherent impl methods live on the struct's object in the
                // global environment (`impl User { fn method(self, ...) }`), not
                // on instances. Fall back to the matching struct method, binding
                // the receiver as the first (`self`) argument.
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
    fn struct_method(
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

    fn list_method(
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

    fn eval_number_arg(
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

    fn list_index_bound(
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
            env.borrow_mut().define(&param.name, value, false);
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
        if is_component_return(&func.return_type) {
            self.run_component_body(&func.body, &env)
        } else {
            self.run_body(&func.body, &env, func.return_type.is_some())
        }
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
            if is_component_return(&f.return_type) {
                self.run_component_body(&f.body, &env)
            } else {
                self.run_body(&f.body, &env, f.return_type.is_some())
            }
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
    fn run_component_body(&self, body: &Block, env: &Rc<RefCell<Env>>) -> Result<Value, RunError> {
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
            Value::String(v) => v.as_ref() == s.as_str(),
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
        Value::String(s) => {
            let full = format!("{enum_name}.{variant}");
            s.as_ref() == full.as_str() || s.as_ref() == variant
        }
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
fn pattern_matches(
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
        } => match scrutinee {
            Value::Number(n) => {
                if *end_inclusive {
                    n >= start && n <= end
                } else {
                    n >= start && n < end
                }
            }
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
fn bind_struct_payload(
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
fn type_matches_value(type_: &Type, value: &Value) -> bool {
    match type_ {
        Type::String => matches!(value, Value::String(_)),
        Type::Number => matches!(value, Value::Number(_)),
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
fn value_is_named_type(value: &Value, type_name: &str) -> bool {
    match type_name {
        "number" | "int" | "float" => matches!(value, Value::Number(_)),
        "string" => matches!(value, Value::String(_)),
        "boolean" => matches!(value, Value::Boolean(_)),
        "null" => matches!(value, Value::Null),
        "list" | "List" => matches!(value, Value::List(_)),
        "object" | "Object" | "Map" => matches!(value, Value::Object(_)),
        "any" | "Any" | "unknown" | "Unknown" => true,
        _ => match value {
            Value::Enum { enum_name, .. } => enum_name == type_name,
            _ => false,
        },
    }
}

/// Deep-copy mutable containers so `copy x` detaches from the original.
fn deep_copy_value(value: &Value) -> Value {
    match value {
        Value::List(list) => {
            let items: Vec<Value> = list.borrow().iter().map(deep_copy_value).collect();
            Value::List(Rc::new(RefCell::new(items)))
        }
        Value::Object(fields) => {
            let items: Vec<(String, Value)> = fields
                .borrow()
                .iter()
                .map(|(k, v)| (k.clone(), deep_copy_value(v)))
                .collect();
            Value::Object(Rc::new(RefCell::new(items)))
        }
        other => other.clone(),
    }
}

/// Find the program's `main`, including `pub fn main`.
fn main_fn(program: &Program) -> Option<&FnDef> {
    program.statements.iter().find_map(|s| match s {
        Statement::Fn(f) if f.name == "main" => Some(f),
        Statement::Export(export) => match &export.item {
            xulo_core::ast::ExportItem::Fn(f) if f.name == "main" => Some(f),
            _ => None,
        },
        _ => None,
    })
}

fn is_component_return(ret: &Option<Type>) -> bool {
    matches!(ret, Some(Type::Named(n, _)) if n == "View")
}

/// The binding names an `import` statement introduces (namespace or named
/// bindings with their aliases); a bare side-effect import binds nothing.
fn import_binding_names(spec: &xulo_core::ast::ImportSpec) -> Vec<String> {
    match spec {
        xulo_core::ast::ImportSpec::Namespace(ns) => vec![ns.clone()],
        xulo_core::ast::ImportSpec::Named(bindings) => bindings
            .iter()
            .map(|(name, alias)| alias.clone().unwrap_or_else(|| name.clone()))
            .collect(),
        xulo_core::ast::ImportSpec::Bare => Vec::new(),
    }
}

/// Gather a module's exports: named bindings, from the bindings already
/// defined by running its statements.
fn collect_exports(
    item: &xulo_core::ast::ExportItem,
    env: &Rc<RefCell<Env>>,
    bindings: &mut Vec<(String, Value)>,
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
        xulo_core::ast::ExportItem::Type(_)
        | xulo_core::ast::ExportItem::Trait(_)
        | xulo_core::ast::ExportItem::Struct(_) => {}
        xulo_core::ast::ExportItem::Enum(e) => {
            bindings.push((e.name.clone(), enum_value(e)));
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
                Value::String(format!("{}.{}", e.name, v.name).into()),
            )
        })
        .collect();
    Value::Object(Rc::new(RefCell::new(fields)))
}
