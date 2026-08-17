use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use xulo_core::ast::{Block, Param, Type};

use crate::env::Env;
use crate::interpreter::{NativeFn, RunError};

/// A run-time value. Lists and objects are shared `Rc<RefCell<>>` handles so
/// assignment mutates in place and aliasing follows JavaScript reference
/// semantics (`b = a; b[0] = 9` also changes `a`).
#[derive(Clone)]
pub enum Value {
    Number(f64),
    String(String),
    Boolean(bool),
    Null,
    List(Rc<RefCell<Vec<Value>>>),
    Object(Rc<RefCell<Vec<(String, Value)>>>),
    /// A user function or closure, capturing its defining environment.
    Function(Rc<FunctionValue>),
    /// An enum value: payload-less variants render as `Enum.Variant`, payload
    /// variants carry one value (a single argument or a `List` for several).
    Enum {
        enum_name: String,
        tag: String,
        payload: Option<Box<Value>>,
    },
    /// A builtin (`print`, `str`).
    Native(NativeFn),
    /// An in-flight `async` call: the result of running an async function. Await
    /// tasks register themselves in `Promise.awaiters` and the scheduler
    /// resumes them in FIFO order once the promise settles.
    Promise(Rc<RefCell<Promise>>),
}

/// The pending computation of an async call, shared with every `await`er.
pub struct Promise {
    pub state: PromiseState,
    /// Tasks waiting to resume once this promise settles (FIFO).
    pub awaiters: VecDeque<usize>,
}

pub enum PromiseState {
    Pending,
    Fulfilled(Value),
    Rejected(RunError),
}

/// The shared body of a user-defined function: parameters (in declared order,
/// so named arguments can be reordered), body block, and the environment the
/// function was defined in (closures capture their enclosing scope).
pub struct FunctionValue {
    pub params: Vec<Param>,
    pub body: Block,
    /// `Some` when the function declared a return type: a trailing expression
    /// statement without a `;` is then the function's implicit return.
    pub return_type: Option<Type>,
    pub is_async: bool,
    pub closure: Rc<RefCell<Env>>,
}

impl Value {
    /// A short type name for error messages (e.g. "a `number`").
    pub fn kind_name(&self) -> String {
        match self {
            Value::Number(_) => "a `number`".into(),
            Value::String(_) => "a `string`".into(),
            Value::Boolean(_) => "a `boolean`".into(),
            Value::Null => "`null`".into(),
            Value::List(_) => "a `list`".into(),
            Value::Object(_) => "an `object`".into(),
            Value::Function(_) | Value::Native(_) => "a function".into(),
            Value::Enum { .. } => "an enum value".into(),
            Value::Promise(_) => "a promise".into(),
        }
    }

    /// Render the value the way `print`/`str` show it.
    pub fn format(&self) -> String {
        match self {
            Value::Number(n) => format_number(*n),
            Value::String(s) => s.clone(),
            Value::Boolean(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::List(list) => {
                let list = list.borrow();
                let parts = list.iter().map(|v| v.format()).collect::<Vec<_>>();
                format!("[{}]", parts.join(", "))
            }
            Value::Object(fields) => {
                let fields = fields.borrow();
                let parts = fields
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", v.format()))
                    .collect::<Vec<_>>();
                format!("{{ {} }}", parts.join(", "))
            }
            Value::Function(_) | Value::Native(_) => "<function>".into(),
            Value::Enum {
                enum_name,
                tag,
                payload,
            } => match payload {
                None => format!("{enum_name}.{tag}"),
                Some(p) => format!("{enum_name}.{tag}({})", p.format()),
            },
            Value::Promise(_) => "Promise".into(),
        }
    }
}

/// Format a number the way JS `String()` does for the common cases: `3.0` ->
/// `"3"`, `NaN` -> `"NaN"`, `Infinity` -> `"Infinity"`, and `-0` -> `"0"`.
pub fn format_number(n: f64) -> String {
    if n.is_nan() {
        return "NaN".into();
    }
    if n.is_infinite() {
        return if n > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        };
    }
    if n == 0.0 && n.is_sign_negative() {
        return "0".into();
    }
    format!("{n}")
}

/// Xulo `==`: numeric/string/boolean/null compare by value, lists and objects
/// by identity (like JS `===` on references), enums structurally (name + tag +
/// payload, mirroring how `match` tests them).
pub fn equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x == y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Boolean(x), Value::Boolean(y)) => x == y,
        (Value::Null, Value::Null) => true,
        (Value::List(x), Value::List(y)) => Rc::ptr_eq(x, y),
        (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
        (Value::Promise(x), Value::Promise(y)) => Rc::ptr_eq(x, y),
        (
            Value::Enum {
                enum_name: n1,
                tag: t1,
                payload: p1,
            },
            Value::Enum {
                enum_name: n2,
                tag: t2,
                payload: p2,
            },
        ) => {
            n1 == n2
                && t1 == t2
                && match (p1, p2) {
                    (Some(a), Some(b)) => equal(a, b),
                    (None, None) => true,
                    _ => false,
                }
        }
        _ => false,
    }
}

/// JS-like truthiness used by `and`/`or`/`!`/`if`/`while`: `false`, `0`,
/// `NaN`, `""`, and `null` are falsy; everything else is truthy.
pub fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Boolean(b) => *b,
        Value::Number(n) => *n != 0.0 && !n.is_nan(),
        Value::String(s) => !s.is_empty(),
        Value::Null => false,
        _ => true,
    }
}
