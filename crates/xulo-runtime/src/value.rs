use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use xulo_core::ast::{Block, Expression, GenericParam, Param, Statement, Type};

use crate::env::Env;
use crate::interpreter::{NativeFn, RunError};

/// An opaque foreign address (spec ffi.md): an extern call result or a value
/// passed through Xulo code that received one. Never null by itself — a zero
/// address is represented as [`Value::Null`] (only `Ptr?` may be null) — and
/// it never converts to or from an integer; only `==`/`!=` may inspect it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PtrValue(pub usize);

/// A `StringBuilder`'s shared state: the current text plus the running scalar
/// count, so `.len` stays O(1) as text is appended and cleared. Every appended
/// value is a valid `string`, so the buffer is always valid UTF-8.
#[derive(Clone, Default)]
pub struct StringBuilderValue {
    text: String,
    scalars: usize,
}

impl StringBuilderValue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn append(&mut self, s: &str) {
        self.scalars += s.chars().count();
        self.text.push_str(s);
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.scalars = 0;
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn len(&self) -> usize {
        self.scalars
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn byte_len(&self) -> usize {
        self.text.len()
    }
}

/// A run-time value. Lists and objects are shared `Rc<RefCell<>>` handles so
/// assignment mutates in place and aliasing follows JavaScript reference
/// semantics (`b = a; b[0] = 9` also changes `a`).
#[derive(Clone)]
pub enum Value {
    /// An `int` (64-bit signed) or a signed fixed-bit integer value. All
    /// signed widths share this slot; the static type decides the width, and
    /// the interpreter traps when an operation leaves that width's range
    /// (spec `types/primitive-types.md`, "Overflow").
    Int(i64),
    /// A `u64`/`usize` value above `i64::MAX`. Smaller unsigned values are
    /// ordinary [`Value::Int`]s, so `u64` shares the same arithmetic tower.
    UInt(u64),
    /// A `float`, `f32`, or `f64` value.
    Float(f64),
    /// An immutable string backed by a refcounted buffer: cloning a `Value`
    /// (env reads, argument passing, indexing) is then a cheap refcount bump
    /// instead of a full heap copy, and the buffer can never be mutated in
    /// place (strings are immutable, matching the JS path).
    String(Rc<str>),
    Boolean(bool),
    Null,
    /// An opaque foreign handle from an FFI call (spec ffi.md).
    Ptr(PtrValue),
    List(Rc<RefCell<Vec<Value>>>),
    /// A `Set<T>`: an unordered collection of unique values, constructed by
    /// `Set::from_list` (spec `types/composite-types.md`).
    Set(Rc<RefCell<Vec<Value>>>),
    /// A `StringBuilder`: a shared, mutable UTF-8 text buffer (spec
    /// `types/composite-types.md`). Binding and assignment share the one
    /// buffer, like the other reference types.
    StringBuilder(Rc<RefCell<StringBuilderValue>>),
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
    /// An `ast::AstNode` value (spec memory-and-runtime.md): opaque beyond
    /// its kind and span, carrying the original node or a constructed field
    /// map for the macro surface.
    AstNode(Rc<AstNodeValue>),
}

/// An `ast::Span` value (spec memory-and-runtime.md).
#[derive(Clone, Debug, PartialEq)]
pub struct SpanValue {
    pub file: String,
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

impl SpanValue {
    pub fn empty() -> Self {
        SpanValue {
            file: String::new(),
            start: 0,
            end: 0,
            line: 1,
            column: 1,
        }
    }
}

/// An `ast::AstNode` value: its kind string, span, optional source text, field
/// map, and the original node when the compiler produced it.
#[derive(Clone)]
pub struct AstNodeValue {
    pub kind: String,
    pub span: SpanValue,
    pub text: Option<String>,
    pub fields: Vec<(String, Value)>,
    pub repr: AstNodeRepr,
}

/// The payload of an [`AstNodeValue`].
#[derive(Clone)]
pub enum AstNodeRepr {
    Statement(Box<Statement>),
    Expression(Box<Expression>),
    /// A node built by `ast::node`, with only its field map.
    Constructed,
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
    /// The full generic parameter list (type and const, in written order) so a
    /// call site can bind const parameters before the body runs (spec
    /// `types/generics.md`).
    pub generic_params: Vec<GenericParam>,
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
            Value::Int(_) => "an `int`".into(),
            Value::UInt(_) => "a `u64`".into(),
            Value::Float(_) => "a `float`".into(),
            Value::String(_) => "a `string`".into(),
            Value::Boolean(_) => "a `bool`".into(),
            Value::Null => "`null`".into(),
            Value::Ptr(_) => "a `Ptr`".into(),
            Value::List(_) => "a `List`".into(),
            Value::Set(_) => "a `Set`".into(),
            Value::StringBuilder(_) => "a `StringBuilder`".into(),
            Value::Object(_) => "an `object`".into(),
            Value::Function(_) | Value::Native(_) => "a function".into(),
            Value::Enum { .. } => "an enum value".into(),
            Value::Promise(_) => "a promise".into(),
            Value::AstNode(_) => "an `ast::AstNode`".into(),
        }
    }

    /// Render the value the way `print`/`str` show it.
    pub fn format(&self) -> String {
        self.format_seen(&mut std::collections::HashSet::new())
    }

    /// Render a value, tracking already-visited list/object identities so a
    /// cyclic structure (`o.x = o`) renders a cycle marker instead of
    /// recursing forever. A handle is removed after its subtree is rendered,
    /// so a shared (non-cyclic) reference like `[a, a]` still prints twice.
    fn format_seen(&self, seen: &mut std::collections::HashSet<usize>) -> String {
        match self {
            Value::Int(n) => n.to_string(),
            Value::UInt(n) => n.to_string(),
            Value::Float(n) => format_number(*n),
            Value::String(s) => s.to_string(),
            Value::Boolean(b) => b.to_string(),
            Value::Null => "null".to_string(),
            Value::Ptr(p) => format!("<ptr {:#x}>", p.0),
            Value::List(list) => {
                let id = Rc::as_ptr(list) as usize;
                if !seen.insert(id) {
                    return "[<cycle>]".into();
                }
                let parts = list
                    .borrow()
                    .iter()
                    .map(|v| v.format_seen(seen))
                    .collect::<Vec<_>>();
                seen.remove(&id);
                format!("[{}]", parts.join(", "))
            }
            Value::Set(set) => {
                let id = Rc::as_ptr(set) as usize;
                if !seen.insert(id) {
                    return "[<cycle>]".into();
                }
                let parts = set
                    .borrow()
                    .iter()
                    .map(|v| v.format_seen(seen))
                    .collect::<Vec<_>>();
                seen.remove(&id);
                format!("Set([{}])", parts.join(", "))
            }
            Value::StringBuilder(builder) => builder.borrow().text().to_string(),
            Value::Object(fields) => {
                let id = Rc::as_ptr(fields) as usize;
                if !seen.insert(id) {
                    return "{ <cycle> }".into();
                }
                let parts = fields
                    .borrow()
                    .iter()
                    .map(|(k, v)| format!("{k}: {}", v.format_seen(seen)))
                    .collect::<Vec<_>>();
                seen.remove(&id);
                format!("{{ {} }}", parts.join(", "))
            }
            Value::Function(_) | Value::Native(_) => "<function>".into(),
            Value::Enum {
                enum_name,
                tag,
                payload,
            } => match payload {
                None => format!("{enum_name}.{tag}"),
                Some(p) => format!("{enum_name}.{tag}({})", p.format_seen(seen)),
            },
            Value::Promise(_) => "Promise".into(),
            Value::AstNode(node) => format!("AstNode({})", node.kind),
        }
    }
}

/// Format a number the way JS `String()` does for the common cases: `3.0` ->
/// `"3"`, `NaN` -> `"NaN"`, `Infinity` -> `"Infinity"`, and `-0` -> `"0"`.
///
/// The implementation lives in `xulo-std` so the interpreter and the compiled
/// path render identically; this re-export keeps the `crate::value` path.
pub use xulo_std::format_number;

/// Xulo `==`: equality is structural for every value-carrying type (spec
/// `expressions/operators.md`, "Equality"). Primitives compare by value,
/// `List`/`Set`/tuple/`Map`/`struct` and enum payloads compare recursively,
/// and only identity-carrying values (`Promise`, function values, `Ptr`,
/// `AstNode`) compare by handle.
///
/// A visited-pair guard makes cyclic structures terminate: a pair already on
/// the comparison stack is treated as equal (coinductive), matching how such
/// values cannot be distinguished by finite observation.
pub fn equal(a: &Value, b: &Value) -> bool {
    equal_seen(a, b, &mut Vec::new())
}

fn equal_seen(a: &Value, b: &Value, seen: &mut Vec<(usize, usize)>) -> bool {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => x == y,
        (Value::UInt(x), Value::UInt(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        // `==` compares numbers by value across the three slots (spec
        // `expressions/operators.md`): `1 == 1.0` is true, and an `int` and a
        // `u64` with the same magnitude are equal.
        (Value::Int(x), Value::UInt(y)) | (Value::UInt(y), Value::Int(x)) => {
            *x >= 0 && *x as u64 == *y
        }
        (Value::Int(x), Value::Float(y)) | (Value::Float(y), Value::Int(x)) => *x as f64 == *y,
        (Value::UInt(x), Value::Float(y)) | (Value::Float(y), Value::UInt(x)) => *x as f64 == *y,
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Boolean(x), Value::Boolean(y)) => x == y,
        (Value::Null, Value::Null) => true,
        // `Ptr` 只有地址恒等可比（spec ffi.md）；与 null 不等——可空性在
        // `Ptr?` 上由 Null 变体承载
        (Value::Ptr(x), Value::Ptr(y)) => x == y,
        (Value::List(x), Value::List(y)) => {
            let pair = (Rc::as_ptr(x) as usize, Rc::as_ptr(y) as usize);
            if seen.contains(&pair) {
                return true;
            }
            seen.push(pair);
            let xs = x.borrow();
            let ys = y.borrow();
            let result = xs.len() == ys.len()
                && xs
                    .iter()
                    .zip(ys.iter())
                    .all(|(left, right)| equal_seen(left, right, seen));
            seen.pop();
            result
        }
        // A `Set` is unordered: same cardinality and every element of one has
        // a structurally equal element in the other (spec
        // `types/composite-types.md`).
        (Value::Set(x), Value::Set(y)) => {
            let pair = (Rc::as_ptr(x) as usize, Rc::as_ptr(y) as usize);
            if seen.contains(&pair) {
                return true;
            }
            seen.push(pair);
            let xs = x.borrow();
            let ys = y.borrow();
            let result = xs.len() == ys.len()
                && xs
                    .iter()
                    .all(|left| ys.iter().any(|right| equal_seen(left, right, seen)));
            seen.pop();
            result
        }
        // A builder compares by its current text, like `string`
        // (spec `types/composite-types.md`).
        (Value::StringBuilder(x), Value::StringBuilder(y)) => {
            x.borrow().text() == y.borrow().text()
        }
        // Objects carry both `struct` fields and `Map` entries; compare by
        // key so field/entry order does not affect equality.
        (Value::Object(x), Value::Object(y)) => {
            let pair = (Rc::as_ptr(x) as usize, Rc::as_ptr(y) as usize);
            if seen.contains(&pair) {
                return true;
            }
            seen.push(pair);
            let xs = x.borrow();
            let ys = y.borrow();
            let result = xs.len() == ys.len()
                && xs.iter().all(|(key, left)| {
                    ys.iter()
                        .find(|(other, _)| other == key)
                        .is_some_and(|(_, right)| equal_seen(left, right, seen))
                });
            seen.pop();
            result
        }
        (Value::Promise(x), Value::Promise(y)) => Rc::ptr_eq(x, y),
        (Value::AstNode(x), Value::AstNode(y)) => Rc::ptr_eq(x, y),
        (Value::Function(x), Value::Function(y)) => Rc::ptr_eq(x, y),
        (Value::Native(x), Value::Native(y)) => Rc::ptr_eq(x, y),
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
                    (Some(a), Some(b)) => equal_seen(a, b, seen),
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
        Value::Int(n) => *n != 0,
        Value::UInt(n) => *n != 0,
        Value::Float(n) => *n != 0.0 && !n.is_nan(),
        Value::String(s) => !s.is_empty(),
        Value::AstNode(_) => true,
        Value::Null => false,
        _ => true,
    }
}

/// The interpreter's numeric tower (spec `types/primitive-types.md`): `int`,
/// `u64`, and `float` share three value slots, and an operation stays integral
/// unless a float takes part. Integer operations are checked against their
/// width and trap on overflow; division and remainder by zero and an
/// out-of-range shift amount also trap.
#[derive(Clone, Copy, Debug)]
pub enum Num {
    Int(i64),
    UInt(u64),
    Float(f64),
}

/// Why an integral operation could not produce a value. The caller names the
/// operator in the diagnostic.
#[derive(Clone, Copy, Debug)]
pub enum NumError {
    /// The result leaves the operand type's range (spec
    /// `types/primitive-types.md`, "Overflow").
    Overflow,
    /// `/` or `%` with a zero divisor (spec `expressions/operators.md`).
    DivByZero,
    /// A shift amount outside `0 … width-1` (spec `expressions/operators.md`).
    ShiftRange,
    /// `**` with a negative integer exponent.
    NegativeExponent,
    /// A bitwise operator applied to a `float`.
    NotAnInteger,
}

impl NumError {
    pub fn message(self) -> &'static str {
        match self {
            NumError::Overflow => "integer overflow",
            NumError::DivByZero => "division by zero",
            NumError::ShiftRange => "shift amount out of range",
            NumError::NegativeExponent => "negative exponent",
            NumError::NotAnInteger => "expected an integer",
        }
    }
}

impl Value {
    /// The numeric view of this value, if it is a number.
    pub fn num(&self) -> Option<Num> {
        match self {
            Value::Int(n) => Some(Num::Int(*n)),
            Value::UInt(n) => Some(Num::UInt(*n)),
            Value::Float(n) => Some(Num::Float(*n)),
            _ => None,
        }
    }

    /// A numeric value in the slot that holds it: a non-negative `u64` that
    /// fits `i64` is an `int`, so ordinary arithmetic keeps the narrower slot.
    pub fn from_num(num: Num) -> Value {
        match num {
            Num::Int(n) => Value::Int(n),
            Num::UInt(n) => {
                if n <= i64::MAX as u64 {
                    Value::Int(n as i64)
                } else {
                    Value::UInt(n)
                }
            }
            Num::Float(n) => Value::Float(n),
        }
    }
}

/// The tower's methods deliberately share names with `std::ops` traits
/// (`add`, `cmp`, …): each is a checked, `Result`-returning operation, not an
/// operator implementation, so the trait analogy would be misleading.
#[allow(clippy::should_implement_trait)]
impl Num {
    pub fn to_f64(self) -> f64 {
        match self {
            Num::Int(n) => n as f64,
            Num::UInt(n) => n as f64,
            Num::Float(n) => n,
        }
    }

    pub fn is_float(self) -> bool {
        matches!(self, Num::Float(_))
    }

    /// The exact integer value, when this is an integral number.
    pub fn integer(self) -> Option<i128> {
        match self {
            Num::Int(n) => Some(n as i128),
            Num::UInt(n) => Some(n as i128),
            Num::Float(n) if n.is_finite() && n.fract() == 0.0 => Some(n as i128),
            Num::Float(_) => None,
        }
    }

    /// Promote to `f64` when either operand is a float, else stay integral.
    fn pair(a: Num, b: Num) -> Option<(Num, Num)> {
        if a.is_float() || b.is_float() {
            Some((Num::Float(a.to_f64()), Num::Float(b.to_f64())))
        } else {
            Some((a, b))
        }
    }

    /// Narrow an exact 128-bit result back into a value slot: an `int` when it
    /// fits, else a `u64`, else overflow.
    fn narrow(value: i128) -> Result<Value, NumError> {
        if let Ok(n) = i64::try_from(value) {
            Ok(Value::Int(n))
        } else if let Ok(n) = u64::try_from(value) {
            Ok(Value::UInt(n))
        } else {
            Err(NumError::Overflow)
        }
    }

    pub fn add(a: Num, b: Num) -> Result<Value, NumError> {
        match Num::pair(a, b).expect("pair always succeeds") {
            (Num::Float(x), Num::Float(y)) => Ok(Value::Float(x + y)),
            (Num::Int(x), Num::Int(y)) => {
                x.checked_add(y).map(Value::Int).ok_or(NumError::Overflow)
            }
            (Num::UInt(x), Num::UInt(y)) => {
                x.checked_add(y).map(Value::UInt).ok_or(NumError::Overflow)
            }
            (x, y) => Num::narrow(
                x.integer()
                    .expect("integral")
                    .checked_add(y.integer().expect("integral"))
                    .ok_or(NumError::Overflow)?,
            ),
        }
    }

    pub fn sub(a: Num, b: Num) -> Result<Value, NumError> {
        match Num::pair(a, b).expect("pair always succeeds") {
            (Num::Float(x), Num::Float(y)) => Ok(Value::Float(x - y)),
            (Num::Int(x), Num::Int(y)) => {
                x.checked_sub(y).map(Value::Int).ok_or(NumError::Overflow)
            }
            // `T op T` keeps the operand type (spec
            // `types/primitive-types.md`, "Arithmetic result types"), so a
            // `u64` difference below zero leaves the type's range and traps.
            // Mixed dynamic operands still narrow.
            (Num::UInt(x), Num::UInt(y)) => {
                x.checked_sub(y).map(Value::UInt).ok_or(NumError::Overflow)
            }
            (x, y) => Num::narrow(
                x.integer()
                    .expect("integral")
                    .checked_sub(y.integer().expect("integral"))
                    .ok_or(NumError::Overflow)?,
            ),
        }
    }

    pub fn mul(a: Num, b: Num) -> Result<Value, NumError> {
        match Num::pair(a, b).expect("pair always succeeds") {
            (Num::Float(x), Num::Float(y)) => Ok(Value::Float(x * y)),
            (Num::Int(x), Num::Int(y)) => {
                x.checked_mul(y).map(Value::Int).ok_or(NumError::Overflow)
            }
            (Num::UInt(x), Num::UInt(y)) => {
                x.checked_mul(y).map(Value::UInt).ok_or(NumError::Overflow)
            }
            (x, y) => Num::narrow(
                x.integer()
                    .expect("integral")
                    .checked_mul(y.integer().expect("integral"))
                    .ok_or(NumError::Overflow)?,
            ),
        }
    }

    /// `/`: integer division truncates toward zero (spec
    /// `expressions/operators.md`); float division follows IEEE-754.
    pub fn div(a: Num, b: Num) -> Result<Value, NumError> {
        match Num::pair(a, b).expect("pair always succeeds") {
            (Num::Float(x), Num::Float(y)) => Ok(Value::Float(x / y)),
            (Num::Int(x), Num::Int(y)) => x.checked_div(y).map(Value::Int).ok_or(if y == 0 {
                NumError::DivByZero
            } else {
                NumError::Overflow
            }),
            (Num::UInt(x), Num::UInt(y)) => {
                x.checked_div(y).map(Value::UInt).ok_or(NumError::DivByZero)
            }
            (x, y) => {
                let divisor = y.integer().expect("integral");
                if divisor == 0 {
                    return Err(NumError::DivByZero);
                }
                Num::narrow(x.integer().expect("integral") / divisor)
            }
        }
    }

    /// `%`: the remainder takes the dividend's sign (spec
    /// `expressions/operators.md`).
    pub fn rem(a: Num, b: Num) -> Result<Value, NumError> {
        match Num::pair(a, b).expect("pair always succeeds") {
            (Num::Float(x), Num::Float(y)) => Ok(Value::Float(x % y)),
            (Num::Int(x), Num::Int(y)) => x.checked_rem(y).map(Value::Int).ok_or(if y == 0 {
                NumError::DivByZero
            } else {
                NumError::Overflow
            }),
            (Num::UInt(x), Num::UInt(y)) => {
                x.checked_rem(y).map(Value::UInt).ok_or(NumError::DivByZero)
            }
            (x, y) => {
                let divisor = y.integer().expect("integral");
                if divisor == 0 {
                    return Err(NumError::DivByZero);
                }
                Num::narrow(x.integer().expect("integral") % divisor)
            }
        }
    }

    pub fn pow(a: Num, b: Num) -> Result<Value, NumError> {
        if a.is_float() || b.is_float() {
            return Ok(Value::Float(a.to_f64().powf(b.to_f64())));
        }
        let Some(exponent) = b.integer() else {
            return Err(NumError::NegativeExponent);
        };
        let Ok(exponent) = u32::try_from(exponent) else {
            return Err(NumError::NegativeExponent);
        };
        match (a, b) {
            (Num::Int(x), _) => x
                .checked_pow(exponent)
                .map(Value::Int)
                .ok_or(NumError::Overflow),
            (Num::UInt(x), _) => x
                .checked_pow(exponent)
                .map(Value::UInt)
                .ok_or(NumError::Overflow),
            _ => unreachable!("floats handled above"),
        }
    }

    fn bit_op(
        a: Num,
        b: Num,
        int: fn(i64, i64) -> i64,
        uint: fn(u64, u64) -> u64,
    ) -> Result<Value, NumError> {
        let (a, b) = match (a, b) {
            (Num::Int(x), Num::Int(y)) => return Ok(Value::Int(int(x, y))),
            (Num::UInt(x), Num::UInt(y)) => return Ok(Value::UInt(uint(x, y))),
            (Num::Float(_), _) | (_, Num::Float(_)) => return Err(NumError::NotAnInteger),
            (x, y) => (x, y),
        };
        let value = int(
            a.integer().expect("integral") as i64,
            b.integer().expect("integral") as i64,
        ) as i128;
        Num::narrow(value)
    }

    pub fn bit_and(a: Num, b: Num) -> Result<Value, NumError> {
        Num::bit_op(a, b, |x, y| x & y, |x, y| x & y)
    }

    pub fn bit_or(a: Num, b: Num) -> Result<Value, NumError> {
        Num::bit_op(a, b, |x, y| x | y, |x, y| x | y)
    }

    pub fn bit_xor(a: Num, b: Num) -> Result<Value, NumError> {
        Num::bit_op(a, b, |x, y| x ^ y, |x, y| x ^ y)
    }

    pub fn bit_not(a: Num) -> Result<Value, NumError> {
        match a {
            Num::Int(x) => Ok(Value::Int(!x)),
            Num::UInt(x) => Ok(Value::UInt(!x)),
            Num::Float(_) => Err(NumError::NotAnInteger),
        }
    }

    /// `<<` fills from the right with `0`; `>>` fills with the sign bit for a
    /// signed type and with `0` for an unsigned one. Bits shifted out are
    /// discarded (spec `expressions/operators.md`).
    pub fn shift(a: Num, amount: Num, left: bool) -> Result<Value, NumError> {
        let amount = match amount.integer() {
            Some(amount) if (0..64).contains(&amount) => amount as u32,
            _ => return Err(NumError::ShiftRange),
        };
        match a {
            Num::Int(x) if left => Ok(Value::Int((x as u64).wrapping_shl(amount) as i64)),
            Num::Int(x) => Ok(Value::Int(x >> amount)),
            Num::UInt(x) if left => Ok(Value::UInt(x.wrapping_shl(amount))),
            Num::UInt(x) => Ok(Value::UInt(x >> amount)),
            Num::Float(_) => Err(NumError::NotAnInteger),
        }
    }

    pub fn neg(a: Num) -> Result<Value, NumError> {
        match a {
            Num::Float(x) => Ok(Value::Float(-x)),
            Num::Int(x) => x.checked_neg().map(Value::Int).ok_or(NumError::Overflow),
            Num::UInt(x) => Num::narrow(-(x as i128)),
        }
    }

    /// Ordering between two numbers (`None` when a `NaN` takes part). Mixed
    /// integer/float comparisons are exact: `9007199254740993 > 9007199254740992.0`
    /// holds even though both round to the same `f64`.
    pub fn cmp(a: Num, b: Num) -> Option<std::cmp::Ordering> {
        use std::cmp::Ordering;
        match (a, b) {
            (Num::Int(x), Num::Int(y)) => Some(x.cmp(&y)),
            (Num::UInt(x), Num::UInt(y)) => Some(x.cmp(&y)),
            (Num::Int(x), Num::UInt(y)) => Some((x as i128).cmp(&(y as i128))),
            (Num::UInt(x), Num::Int(y)) => Some((x as i128).cmp(&(y as i128))),
            (Num::Float(x), Num::Float(y)) => x.partial_cmp(&y),
            (Num::Int(x), Num::Float(y)) => cmp_i128_f64(x as i128, y),
            (Num::UInt(x), Num::Float(y)) => cmp_i128_f64(x as i128, y),
            (Num::Float(x), Num::Int(y)) => cmp_i128_f64(y as i128, x).map(Ordering::reverse),
            (Num::Float(x), Num::UInt(y)) => cmp_i128_f64(y as i128, x).map(Ordering::reverse),
        }
    }
}

/// Exact comparison of an integer with a float: values beyond `i128` compare
/// by sign, and a fractional float orders against the truncation.
fn cmp_i128_f64(integer: i128, float: f64) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    if float.is_nan() {
        return None;
    }
    if float >= 1.7e38 {
        return Some(Ordering::Less);
    }
    if float <= -1.7e38 {
        return Some(Ordering::Greater);
    }
    let truncated = float.trunc() as i128;
    match integer.cmp(&truncated) {
        Ordering::Equal if float.fract() > 0.0 => Some(Ordering::Less),
        Ordering::Equal if float.fract() < 0.0 => Some(Ordering::Greater),
        other => Some(other),
    }
}
