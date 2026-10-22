use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

use corosensei::stack::DefaultStack;
use corosensei::{Coroutine, Yielder};

use xulo_core::ast::{
    AssignTarget, Attribute, BinaryOp, BinaryOperator, BindingPattern, Block, Call, ComponentStmt,
    EffectStmt, EnumDef, Expression, FnDef, ForStmt, IfExpr, ImplDecl, LetBinding, Literal,
    MatchExpr, ObjectField, Param, Program, ReturnStmt, Statement, StoreStmt, StructDef, TryStmt,
    Type, UiElement, UnaryOperator, impl_fn_name,
};
use xulo_core::error::{ErrorKind, XuloError};

use crate::env::Env;
use crate::value::{FunctionValue, Value, equal, is_truthy};

pub use xulo_core::ast::CallArg;

mod call;
mod closure;
mod decl;
mod expr;
mod ffi;
mod native;
mod pattern;
mod stmt;
mod task;

// 自由函数/类型在各子模块定义；这里再引入 interpreter 的命名空间，
// 让所有子模块经 `use super::*;` 与原单文件时代一样按裸名解析。
use closure::*;
use decl::*;
use native::*;
use pattern::*;
use task::*;

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

/// A run-time failure: a fatal error (optionally with a source span), a value
/// thrown by `throw` that a surrounding `try`/`catch` may handle, or the
/// failure value of a `?` in flight — [`RunError::Propagate`] carries it to
/// the enclosing function's call boundary, which returns it as that
/// function's value (spec `error-handling.md`). A `try`/`catch` does not
/// intercept propagation.
#[derive(Clone)]
pub enum RunError {
    Err(XuloError),
    Throw(Value),
    Propagate(Value),
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
    env: Rc<RefCell<Env>>,
}

impl ModuleExports {
    /// Re-bind an import placeholder in this module's scope once the target
    /// module has executed. Legal `use` cycles are filled two-phase
    /// (spec/import-export.md): the placeholder holds an empty object until
    /// the loop owner patches it in with the real binding.
    pub fn patch_import(&self, local: &str, value: Value) {
        self.env.borrow_mut().define(local, value, false);
    }
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
    /// `Task::timeout` deadlines: `(deadline, combined promise, the value it
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
    /// Names of every `enum` declared by a program executed in this
    /// interpreter (execution registers them as it goes). A `head::member`
    /// call whose head is here constructs a variant; any other head that
    /// names an object in scope reads as a namespace member
    /// (`use mod as ns; ns::f()` — spec `expressions/path-and-access.md`).
    enum_names: RefCell<Vec<String>>,
    /// Declared field names and default expressions per `struct`, with the
    /// environment defaults evaluate in — the metadata behind
    /// `User(name: …, age: …)` construction (spec `types/composite-types.md`).
    struct_fields: RefCell<std::collections::HashMap<String, StructCtorDecl>>,
    /// Declared parameter names per registered extern import (spec ffi.md
    /// Calling): named arguments reorder by these, and an omitted (only
    /// `Ptr?` may be) parameter binds `null`, as for any declared `fn`.
    /// Keyed by the local binding name — the callee's bare name, which is
    /// the only call position an import has (E0903).
    extern_params: RefCell<std::collections::HashMap<String, Vec<String>>>,
    /// Instances built by a struct constructor, keyed by their field map's
    /// identity: `u.to_string()` must find *that* struct's methods, not the
    /// first registered struct declaring a method of the same name. The weak
    /// handle makes a recycled pointer evict a stale entry instead of claiming
    /// a type it no longer has.
    instance_types: InstanceTypeMap,
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

/// Pointer-identity side table for constructor-built instances: object ptr ->
/// (weak handle to the defining module's bindings, struct name).
type InstanceTypeMap =
    RefCell<std::collections::HashMap<usize, (Weak<RefCell<Vec<(String, Value)>>>, String)>>;

/// One `Task::timeout` deadline: when it fires, the combined promise it
/// settles, and the value that settles it.
type TimerEntry = (
    std::time::Instant,
    Rc<RefCell<crate::value::Promise>>,
    Value,
);

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
            ("all".to_string(), Value::Native(Rc::new(native_task_all))),
            ("race".to_string(), Value::Native(Rc::new(native_task_race))),
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
        // The `std::math` namespace of constants and functions (spec
        // `builtins/intrinsic-functions.md`): reached as `std::math::PI` /
        // `std::math::max(a, b)`, never as a value of its own.
        let math_members: Vec<(String, Value)> = vec![
            ("PI".to_string(), Value::Number(xulo_std::math::PI)),
            ("E".to_string(), Value::Number(xulo_std::math::E)),
            ("TAU".to_string(), Value::Number(xulo_std::math::TAU)),
            (
                "INFINITY".to_string(),
                Value::Number(xulo_std::math::INFINITY),
            ),
            ("NAN".to_string(), Value::Number(xulo_std::math::NAN)),
            ("abs".to_string(), Value::Native(Rc::new(native_math_abs))),
            ("min".to_string(), Value::Native(Rc::new(native_math_min))),
            ("max".to_string(), Value::Native(Rc::new(native_math_max))),
            ("sqrt".to_string(), Value::Native(Rc::new(native_math_sqrt))),
            ("cbrt".to_string(), Value::Native(Rc::new(native_math_cbrt))),
            ("pow".to_string(), Value::Native(Rc::new(native_math_pow))),
            (
                "floor".to_string(),
                Value::Native(Rc::new(native_math_floor)),
            ),
            ("ceil".to_string(), Value::Native(Rc::new(native_math_ceil))),
            (
                "round".to_string(),
                Value::Native(Rc::new(native_math_round)),
            ),
            (
                "trunc".to_string(),
                Value::Native(Rc::new(native_math_trunc)),
            ),
            (
                "toFixed".to_string(),
                Value::Native(Rc::new(native_math_to_fixed)),
            ),
            (
                "sin".to_string(),
                Value::Native(Rc::new(native_math_unary_float(
                    "std::math::sin",
                    xulo_std::math::sin,
                ))),
            ),
            (
                "cos".to_string(),
                Value::Native(Rc::new(native_math_unary_float(
                    "std::math::cos",
                    xulo_std::math::cos,
                ))),
            ),
            (
                "tan".to_string(),
                Value::Native(Rc::new(native_math_unary_float(
                    "std::math::tan",
                    xulo_std::math::tan,
                ))),
            ),
            (
                "asin".to_string(),
                Value::Native(Rc::new(native_math_unary_float(
                    "std::math::asin",
                    xulo_std::math::asin,
                ))),
            ),
            (
                "acos".to_string(),
                Value::Native(Rc::new(native_math_unary_float(
                    "std::math::acos",
                    xulo_std::math::acos,
                ))),
            ),
            (
                "atan".to_string(),
                Value::Native(Rc::new(native_math_unary_float(
                    "std::math::atan",
                    xulo_std::math::atan,
                ))),
            ),
            (
                "atan2".to_string(),
                Value::Native(Rc::new(native_math_atan2)),
            ),
            ("log".to_string(), Value::Native(Rc::new(native_math_log))),
            ("log2".to_string(), Value::Native(Rc::new(native_math_log2))),
            (
                "log10".to_string(),
                Value::Native(Rc::new(native_math_log10)),
            ),
            ("exp".to_string(), Value::Native(Rc::new(native_math_exp))),
        ];
        let math_obj = Value::Object(Rc::new(RefCell::new(math_members)));
        // The `std::time` namespace of clocks and async sleep (spec
        // `builtins/intrinsic-functions.md`): reached as `std::time::now` /
        // `await std::time::sleep(ms)`.
        let time_members: Vec<(String, Value)> = vec![
            ("now".to_string(), Value::Native(Rc::new(native_time_now))),
            (
                "now_nanos".to_string(),
                Value::Native(Rc::new(native_time_now_nanos)),
            ),
            (
                "monotonic".to_string(),
                Value::Native(Rc::new(native_time_monotonic)),
            ),
            (
                "monotonic_nanos".to_string(),
                Value::Native(Rc::new(native_time_monotonic_nanos)),
            ),
            (
                "sleep".to_string(),
                Value::Native(Rc::new(native_time_sleep)),
            ),
        ];
        let time_obj = Value::Object(Rc::new(RefCell::new(time_members)));
        // The `std::fs` namespace of filesystem primitives (implementation-first;
        // the language itself has no I/O, spec `memory-and-runtime.md`):
        // reached as `std::fs::read(...)` / `std::fs::write(...)`, each fallible
        // member returning the built-in `Result`.
        let fs_members: Vec<(String, Value)> = vec![
            ("read".to_string(), Value::Native(Rc::new(native_fs_read))),
            ("write".to_string(), Value::Native(Rc::new(native_fs_write))),
            (
                "exists".to_string(),
                Value::Native(Rc::new(native_fs_exists)),
            ),
            (
                "remove".to_string(),
                Value::Native(Rc::new(native_fs_remove)),
            ),
            ("mkdir".to_string(), Value::Native(Rc::new(native_fs_mkdir))),
        ];
        let fs_obj = Value::Object(Rc::new(RefCell::new(fs_members)));
        // The `std::regex` module behind the language's `RegExp` type (spec
        // `regular-expressions.mdx`): `RegExp::compile(…)` resolves through
        // the imported name, which `setup_module` materializes from this
        // object for `use std::regex::{RegExp, …}`.
        let regexp_static: Vec<(String, Value)> = vec![(
            "compile".to_string(),
            Value::Native(Rc::new(native_regex_compile)),
        )];
        let regex_obj = Value::Object(Rc::new(RefCell::new(vec![(
            "RegExp".to_string(),
            Value::Object(Rc::new(RefCell::new(regexp_static))),
        )])));
        // The whole built-in tree hangs off a single `std` namespace object:
        // `std::math::floor(…)` walks `std` → `math` → `floor` through the
        // generic namespace dispatch, and `setup_module` copies members out
        // of it for bare-name imports.
        let std_members: Vec<(String, Value)> = vec![
            ("math".to_string(), math_obj),
            ("time".to_string(), time_obj),
            ("fs".to_string(), fs_obj),
            ("regex".to_string(), regex_obj),
        ];
        global.borrow_mut().define(
            "std",
            Value::Object(Rc::new(RefCell::new(std_members))),
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
            enum_names: RefCell::new(Vec::new()),
            struct_fields: RefCell::new(std::collections::HashMap::new()),
            extern_params: RefCell::new(std::collections::HashMap::new()),
            instance_types: RefCell::new(std::collections::HashMap::new()),
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
        // `use std::…` binds from the global `std` object, as in
        // `setup_module` (the single-module entry point skips the import table).
        for statement in &program.statements {
            if let Statement::Use(u) = statement {
                self.bind_std_use(u, &global).map_err(|e| match e {
                    RunError::Err(e) => e,
                    RunError::Propagate(v) => stray_propagate(&v),
                    RunError::Throw(v) => self.uncaught(&v),
                })?;
            }
        }
        for statement in &program.statements {
            match statement {
                Statement::Fn(f) => self.register_fn(f, &global),
                Statement::Impl(imp) => self.register_impl(imp, &global),
                Statement::Struct(s) => self.register_struct(s, &global),
                Statement::Enum(e) => self.register_derived_enum(e),
                Statement::Export(export) => match &export.item {
                    xulo_core::ast::ExportItem::Fn(f) => self.register_fn(f, &global),
                    xulo_core::ast::ExportItem::Struct(s) => self.register_struct(s, &global),
                    xulo_core::ast::ExportItem::Enum(e) => self.register_derived_enum(e),
                    _ => {}
                },
                // FFI 导入：声明期注册 Native 桥（dlsym + trampoline），先于
                // 任何语句执行（spec ffi.md）
                Statement::ExternBlock(b) => self.register_externs(b, &global)?,
                _ => {}
            }
        }
        for statement in &program.statements {
            match self.exec_stmt(statement, &global) {
                Ok(_) => {}
                Err(RunError::Err(e)) => return Err(e),
                Err(RunError::Propagate(v)) => return Err(stray_propagate(&v)),
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
                        RunError::Propagate(v) => stray_propagate(v),
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
            Err(RunError::Propagate(v)) => Err(stray_propagate(&v)),
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
            match statement {
                Statement::Export(export) => collect_exports(&export.item, &global, &mut bindings),
                // `pub use`/`pub mod` re-export names this module bound but
                // never declared: mirror them into the export map so
                // importers see the same interface the semantic pass does.
                Statement::Use(u) if u.is_pub => {
                    for local in use_local_names(&u.target) {
                        if let Some(v) = global.borrow().get(&local)
                            && !bindings.iter().any(|(n, _)| n == &local)
                        {
                            bindings.push((local, v));
                        }
                    }
                }
                Statement::Mod(m) if m.is_pub => {
                    if let Some(v) = global.borrow().get(&m.name)
                        && !bindings.iter().any(|(n, _)| n == &m.name)
                    {
                        bindings.push((m.name.clone(), v));
                    }
                }
                // `pub extern` 块导出每条 item 的本地绑定（与语义层的接口面一致）
                Statement::ExternBlock(b) if b.is_pub => {
                    for item in &b.items {
                        if let Some(v) = global.borrow().get(&item.local_name)
                            && !bindings.iter().any(|(n, _)| n == &item.local_name)
                        {
                            bindings.push((item.local_name.clone(), v));
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(ModuleExports {
            bindings,
            env: global,
        })
    }

    /// Bind a module's imports, register its declarations, and execute its
    /// top-level statements (skipping `import`s, which the caller pre-binds).
    /// Shared by [`Interpreter::exec_module`] and [`Interpreter::run_test`].
    /// The namespace object a `use` head resolves to in `env`, descended
    /// through the middle segments (`std` + `[math]` → the math table).
    fn use_container(
        &self,
        head: &xulo_core::ast::PathHead,
        mids: &[String],
        env: &Rc<RefCell<Env>>,
    ) -> Option<Value> {
        let head_name = match head {
            xulo_core::ast::PathHead::Name(n) | xulo_core::ast::PathHead::Package(n) => n.as_str(),
            xulo_core::ast::PathHead::SelfModule => "self",
            xulo_core::ast::PathHead::Super => return None,
        };
        let mut current = match env.borrow().get(head_name) {
            Some(Value::Object(obj)) => Value::Object(obj),
            _ => return None,
        };
        for seg in mids {
            let Value::Object(ref obj) = current else {
                return None;
            };
            let next = obj
                .borrow()
                .iter()
                .find(|(k, _)| k == seg)
                .map(|(_, v)| v.clone());
            match next {
                Some(Value::Object(child)) => current = Value::Object(child),
                _ => return None,
            }
        }
        Some(current)
    }

    /// Materialize one `use` declaration's values in `env`.
    ///
    /// The CLI's import table covers module-graph imports; the built-in
    /// `std` tree has no graph behind it, so `use std::math::*`, `use
    /// std::math as m`, and `use std::regex::{RegExp}` bind straight from
    /// the global `std` object here. Names already in `env` (module imports
    /// the table supplied) are left alone, and a non-std head whose
    /// namespace is not in scope binds nothing.
    fn bind_std_use(
        &self,
        u: &xulo_core::ast::UseDecl,
        env: &Rc<RefCell<Env>>,
    ) -> Result<(), RunError> {
        let head_is_std = matches!(&u.target, xulo_core::ast::UseTarget::Path { head, .. } | xulo_core::ast::UseTarget::Group { head, .. } | xulo_core::ast::UseTarget::Glob { head, .. }
            if matches!(head, xulo_core::ast::PathHead::Name(n) if n == "std"));
        match &u.target {
            xulo_core::ast::UseTarget::Path {
                head,
                segments,
                alias,
            } => {
                if segments.is_empty() {
                    return Ok(());
                }
                let (mids, member) = segments.split_at(segments.len() - 1);
                let Some(container) = self.use_container(head, mids, env) else {
                    if head_is_std {
                        return Err(RunError::err(
                            format!("cannot resolve `std::{}`", segments.join("::")),
                            u.span.clone(),
                        ));
                    }
                    return Ok(());
                };
                let Value::Object(ref obj) = container else {
                    return Ok(());
                };
                let value = obj
                    .borrow()
                    .iter()
                    .find(|(k, _)| k == member[0].as_str())
                    .map(|(_, v)| v.clone());
                let Some(value) = value else {
                    // A std member with no runtime value (`RegexError`,
                    // `RegexMatch` — types only) binds nothing: the semantic
                    // pass already rejected any value-position use.
                    return Ok(());
                };
                let local = alias.clone().unwrap_or_else(|| member[0].clone());
                if env.borrow().get(&local).is_none() {
                    env.borrow_mut().define(&local, value, false);
                }
                Ok(())
            }
            xulo_core::ast::UseTarget::Group { head, entries } => {
                for entry in entries {
                    if entry.segments.is_empty() {
                        continue;
                    }
                    let (mids, member) = entry.segments.split_at(entry.segments.len() - 1);
                    let Some(container) = self.use_container(head, mids, env) else {
                        if head_is_std {
                            return Err(RunError::err(
                                format!("cannot resolve `std::{}`", entry.segments.join("::")),
                                u.span.clone(),
                            ));
                        }
                        continue;
                    };
                    let Value::Object(ref obj) = container else {
                        continue;
                    };
                    let value = obj
                        .borrow()
                        .iter()
                        .find(|(k, _)| k == member[0].as_str())
                        .map(|(_, v)| v.clone());
                    let Some(value) = value else {
                        // A std member with no runtime value (a type like
                        // `RegexError`) binds nothing.
                        continue;
                    };
                    let local = entry.alias.clone().unwrap_or_else(|| member[0].clone());
                    if env.borrow().get(&local).is_none() {
                        env.borrow_mut().define(&local, value, false);
                    }
                }
                Ok(())
            }
            xulo_core::ast::UseTarget::Glob { head, segments } => {
                let Some(container) = self.use_container(head, segments, env) else {
                    if head_is_std {
                        return Err(RunError::err(
                            if segments.is_empty() {
                                "`std` has no direct members".to_string()
                            } else {
                                format!("cannot resolve `std::{}`", segments.join("::"))
                            },
                            u.span.clone(),
                        ));
                    }
                    return Ok(());
                };
                let Value::Object(ref obj) = container else {
                    return Ok(());
                };
                let members: Vec<(String, Value)> = obj
                    .borrow()
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                for (name, value) in members {
                    if env.borrow().get(&name).is_none() {
                        env.borrow_mut().define(&name, value, false);
                    }
                }
                Ok(())
            }
            xulo_core::ast::UseTarget::Local(_) => Ok(()),
        }
    }

    fn setup_module(
        &self,
        program: &Program,
        imports: &[(String, Value)],
    ) -> Result<Rc<RefCell<Env>>, RunError> {
        let global = Env::child(&self.global);
        for (name, value) in imports {
            global.borrow_mut().define(name, value.clone(), false);
        }
        // The built-in `std` imports carry no module graph, so the CLI's
        // import table cannot supply them: resolve `use std::…` (and uses of
        // a std alias) against the global `std` object right here.
        for statement in &program.statements {
            if let Statement::Use(u) = statement {
                self.bind_std_use(u, &global)?;
            }
        }
        for statement in &program.statements {
            match statement {
                Statement::Fn(f) => self.register_fn(f, &global),
                Statement::Impl(imp) => self.register_impl(imp, &global),
                Statement::Struct(s) => self.register_struct(s, &global),
                Statement::Enum(e) => self.register_derived_enum(e),
                Statement::Export(export) => self.register_export_fns(&export.item, &global),
                Statement::ExternBlock(b) => {
                    self.register_externs(b, &global).map_err(RunError::Err)?
                }
                _ => {}
            }
        }
        for statement in &program.statements {
            match self.exec_stmt(statement, &global) {
                Ok(_) => {}
                Err(e) => return Err(e),
            }
        }
        Ok(global)
    }

    /// Execute a module exactly like [`Interpreter::exec_module`] with
    /// `run_main: false`, then invoke one `#Test fn` body. The test body sees
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
            RunError::Propagate(v) => stray_propagate(&v),
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
                        RunError::Propagate(v) => stray_propagate(v),
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
            Err(RunError::Propagate(v)) => Err(stray_propagate(&v)),
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
        match item {
            xulo_core::ast::ExportItem::Fn(f) => self.register_fn(f, env),
            // `pub struct`/`pub enum` reach this arm parser-wrapped: the
            // struct's constructor table and derived impls register here so
            // construction and dispatch work exactly like a bare declaration.
            xulo_core::ast::ExportItem::Struct(s) => self.register_struct(s, env),
            xulo_core::ast::ExportItem::Enum(e) => self.register_derived_enum(e),
            _ => {}
        }
    }

    fn uncaught(&self, value: &Value) -> XuloError {
        XuloError::new(
            ErrorKind::Runtime,
            format!("uncaught exception: {}", value.format()),
        )
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
        xulo_core::ast::ExportItem::Type(_) | xulo_core::ast::ExportItem::Trait(_) => {}
        xulo_core::ast::ExportItem::Struct(s) => {
            if let Some(v) = env.borrow().get(&s.name) {
                bindings.push((s.name.clone(), v));
            }
        }
        xulo_core::ast::ExportItem::Enum(e) => {
            bindings.push((e.name.clone(), enum_value(e)));
        }
    }
}

/// The local names a `pub use` target binds, mirroring the semantic pass's
/// interface so re-exported names land in the module's export map. Globs are
/// skipped: each globbed name is an individual `use` binding the runtime
/// already wired, and the semantic pass publishes them under their own locals.
fn use_local_names(target: &xulo_core::ast::UseTarget) -> Vec<String> {
    let head_name = |head: &xulo_core::ast::PathHead| match head {
        xulo_core::ast::PathHead::Package(n) | xulo_core::ast::PathHead::Name(n) => n.clone(),
        xulo_core::ast::PathHead::SelfModule => "self".to_string(),
        xulo_core::ast::PathHead::Super => "super".to_string(),
    };
    match target {
        xulo_core::ast::UseTarget::Path {
            head,
            segments,
            alias,
        } => vec![
            alias
                .clone()
                .or_else(|| segments.last().cloned())
                .unwrap_or_else(|| head_name(head)),
        ],
        xulo_core::ast::UseTarget::Group { entries, .. } => entries
            .iter()
            .map(|e| e.alias.clone().or_else(|| e.segments.last().cloned()))
            .filter_map(|s| s.filter(|s| !s.is_empty()))
            .collect(),
        xulo_core::ast::UseTarget::Glob { .. } => Vec::new(),
        xulo_core::ast::UseTarget::Local(names) => names.clone(),
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
