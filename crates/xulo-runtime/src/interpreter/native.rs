use super::*;

/// Normalize a combinator operand to a promise: tasks stay as they are,
/// plain values become already-fulfilled promises (spec `Task::resolve`).
pub(super) fn as_promise(value: &Value) -> Rc<RefCell<crate::value::Promise>> {
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
pub(super) fn settle_promise_once(
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

/// `Task::all([t, u])`: the combined task settles when every input has settled
/// and carries the results in argument order; the first rejection rejects it.
pub(super) fn native_task_all(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let list = match args.first() {
        Some(Value::List(list)) => list.clone(),
        other => {
            return Err(RunError::err(
                format!(
                    "`Task::all` expects a list of tasks, found {}",
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

/// `Task::race([t, u])`: the combined task settles with the first input to
/// settle; the others keep running and their results are discarded.
pub(super) fn native_task_race(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let list = match args.first() {
        Some(Value::List(list)) => list.clone(),
        other => {
            return Err(RunError::err(
                format!(
                    "`Task::race` expects a list of tasks, found {}",
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

/// `Task::resolve(v)`: an already-fulfilled task holding `v`.
pub(super) fn native_task_resolve(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
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
pub(super) fn error_value(message: Rc<str>) -> Value {
    Value::Object(Rc::new(RefCell::new(vec![(
        "message".to_string(),
        Value::String(message),
    )])))
}

/// `Error(message)`: construct the base error type (the semantic pass checks
/// the argument is a string; the runtime formats anything for robustness).
pub(super) fn native_error(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let message = match args.first() {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.format().into(),
        None => String::new().into(),
    };
    Ok(error_value(message))
}

/// `Result::Ok(value)` as a runtime enum value. `Task::timeout` wraps outcomes
/// in the built-in `Result`, which needs no runtime registration: enum
/// construction and matching are generic over the enum's name.
pub(super) fn result_ok(value: Value) -> Value {
    Value::Enum {
        enum_name: "Result".to_string(),
        tag: "Ok".to_string(),
        payload: Some(Box::new(value)),
    }
}

/// `Result::Err(error)` as a runtime enum value.
pub(super) fn result_err(error: Value) -> Value {
    Value::Enum {
        enum_name: "Result".to_string(),
        tag: "Err".to_string(),
        payload: Some(Box::new(error)),
    }
}

/// A `?` failure value that reached a program entry point with no function
/// boundary left to return to (semantic checking rejects every such `?`;
/// this is the runtime's defensive line).
pub(super) fn stray_propagate(value: &Value) -> XuloError {
    XuloError::new(
        ErrorKind::Runtime,
        format!(
            "`?` propagated with no enclosing function: {}",
            value.format()
        ),
    )
}

/// `Task::timeout(t, ms)`: settles with `Result::Ok(t's result)` when `t`
/// finishes within `ms`, or with `Result::Err(Error("timed out"))` when the
/// deadline elapses first (spec `error-handling.md`). The deadline is a timer
/// entry `drive` fires once nothing else is ready to run; a rejection of `t`
/// propagates as a rejection (the runtime has no separate cancelled state).
///
/// The wrapped task cannot be interrupted — there is no cancellation — so
/// after a timeout it runs on: its late result finds the combined promise
/// already settled and is discarded, and its timer entry is pruned by `drive`
/// instead of being waited for.
pub(super) fn native_task_timeout(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let Some(first) = args.first() else {
        return Err(RunError::err(
            "`Task::timeout` requires a task argument",
            0..0,
        ));
    };
    let inner = as_promise(first);
    let ms = match args.get(1) {
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => *n,
        other => {
            return Err(RunError::err(
                format!(
                    "`Task::timeout` expects a millisecond count of type number, found {}",
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

/// A numeric argument of a built-in namespace call. The semantic pass checks
/// the registered signature; this guard keeps a program that skips it from
/// panicking on a wrong operand.
pub(super) fn builtin_num_arg(args: &[Value], i: usize, name: &str) -> Result<f64, RunError> {
    match args.get(i) {
        Some(Value::Number(n)) => Ok(*n),
        other => Err(RunError::err(
            format!(
                "`{name}` expects a number, found {}",
                other
                    .map(|v| v.kind_name())
                    .unwrap_or_else(|| "nothing".into())
            ),
            0..0,
        )),
    }
}

/// A string argument of a built-in namespace call, the counterpart of
/// [`builtin_num_arg`] for the path/content parameters of `Fs`.
pub(super) fn builtin_str_arg(args: &[Value], i: usize, name: &str) -> Result<Rc<str>, RunError> {
    match args.get(i) {
        Some(Value::String(s)) => Ok(s.clone()),
        other => Err(RunError::err(
            format!(
                "`{name}` expects a string, found {}",
                other
                    .map(|v| v.kind_name())
                    .unwrap_or_else(|| "nothing".into())
            ),
            0..0,
        )),
    }
}

/// `std::math::abs(x)`: the absolute value (spec `builtins/intrinsic-functions.md`).
pub(super) fn native_math_abs(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::abs(builtin_num_arg(
        args,
        0,
        "std::math::abs",
    )?)))
}

/// `std::math::min(a, b)` / `std::math::max(a, b)`: the smaller / larger operand, with
/// the IEEE-754 special values propagating (a `NaN` operand yields `NaN`).
pub(super) fn native_math_min(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let a = builtin_num_arg(args, 0, "std::math::min")?;
    let b = builtin_num_arg(args, 1, "std::math::min")?;
    Ok(Value::Number(xulo_std::math::min(a, b)))
}

pub(super) fn native_math_max(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let a = builtin_num_arg(args, 0, "std::math::max")?;
    let b = builtin_num_arg(args, 1, "std::math::max")?;
    Ok(Value::Number(xulo_std::math::max(a, b)))
}

/// `std::math::sqrt(x)`: the square root; a negative operand yields `NaN`.
pub(super) fn native_math_sqrt(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::sqrt(builtin_num_arg(
        args,
        0,
        "std::math::sqrt",
    )?)))
}

/// `std::math::random()`: a pseudo-random float in `[0.0, 1.0)` (spec
/// `builtins/intrinsic-functions.md`) — the `Math` table's only stateful
/// member.
pub(super) fn native_math_random(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    no_arg_guard(args, "std::math::random")?;
    Ok(Value::Number(xulo_std::math::random()))
}

/// `std::math::sign(x)`, `log1p`, `expm1`: unary float operations.
pub(super) fn native_math_sign(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::sign(builtin_num_arg(
        args,
        0,
        "std::math::sign",
    )?)))
}

pub(super) fn native_math_hypot(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let a = builtin_num_arg(args, 0, "std::math::hypot")?;
    let b = builtin_num_arg(args, 1, "std::math::hypot")?;
    Ok(Value::Number(xulo_std::math::hypot(a, b)))
}

pub(super) fn native_math_clamp(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let x = builtin_num_arg(args, 0, "std::math::clamp")?;
    let lo = builtin_num_arg(args, 1, "std::math::clamp")?;
    let hi = builtin_num_arg(args, 2, "std::math::clamp")?;
    Ok(Value::Number(xulo_std::math::clamp(x, lo, hi)))
}

pub(super) fn native_math_log1p(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::log1p(builtin_num_arg(
        args,
        0,
        "std::math::log1p",
    )?)))
}

pub(super) fn native_math_expm1(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::expm1(builtin_num_arg(
        args,
        0,
        "std::math::expm1",
    )?)))
}

/// `std::math::random_int(lo, hi)`: a uniform integer in `lo …= hi`; `lo > hi`
/// is a runtime failure (spec `builtins/intrinsic-functions.md`).
pub(super) fn native_math_random_int(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let lo = builtin_int_arg(args, 0, "std::math::random_int")?;
    let hi = builtin_int_arg(args, 1, "std::math::random_int")?;
    if lo > hi {
        return Err(RunError::err(
            format!("`std::math::random_int` range is empty: lo {lo} is greater than hi {hi}"),
            0..0,
        ));
    }
    Ok(Value::Number(xulo_std::math::random_int(lo, hi) as f64))
}

/// `std::math::cbrt(x)`: the cube root, defined for every `float`.
pub(super) fn native_math_cbrt(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::cbrt(builtin_num_arg(
        args,
        0,
        "std::math::cbrt",
    )?)))
}

/// `std::math::pow(base, exp)`: `base` raised to `exp`.
pub(super) fn native_math_pow(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let base = builtin_num_arg(args, 0, "std::math::pow")?;
    let exp = builtin_num_arg(args, 1, "std::math::pow")?;
    Ok(Value::Number(xulo_std::math::pow(base, exp)))
}

/// `std::math::floor(x)` / `std::math::ceil(x)` / `std::math::round(x)` / `std::math::trunc(x)`:
/// the integral value, typed `int`. `std::math::round` rounds halfway cases away
/// from zero (spec `builtins/intrinsic-functions.md`); every result stays an
/// integer-valued `float`.
pub(super) fn native_math_floor(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::floor(builtin_num_arg(
        args,
        0,
        "std::math::floor",
    )?)))
}

pub(super) fn native_math_ceil(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::ceil(builtin_num_arg(
        args,
        0,
        "std::math::ceil",
    )?)))
}

pub(super) fn native_math_round(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::round(builtin_num_arg(
        args,
        0,
        "std::math::round",
    )?)))
}

pub(super) fn native_math_trunc(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::trunc(builtin_num_arg(
        args,
        0,
        "std::math::trunc",
    )?)))
}

/// `std::math::to_fixed(x, digits)`: the decimal numeral of `x` with exactly
/// `digits` digits after the decimal point, rounded halfway away from zero
/// and keeping trailing zeros (spec `builtins/intrinsic-functions.md`).
/// Special values ignore `digits`. A `digits` value outside `0..=15` is a
/// runtime failure (spec `memory-and-runtime.md`).
pub(super) fn native_math_to_fixed(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let x = builtin_num_arg(args, 0, "std::math::to_fixed")?;
    let digits = match args.get(1) {
        Some(Value::Number(d)) if d.fract() == 0.0 && (0.0..=15.0).contains(d) => *d as usize,
        Some(Value::Number(d)) => {
            return Err(RunError::err(
                format!("`std::math::to_fixed` digits must be an integer in 0..=15, found {d}"),
                0..0,
            ));
        }
        other => {
            return Err(RunError::err(
                format!(
                    "`std::math::to_fixed` expects a number for `digits`, found {}",
                    other
                        .map(|v| v.kind_name())
                        .unwrap_or_else(|| "nothing".into())
                ),
                0..0,
            ));
        }
    };
    Ok(Value::String(xulo_std::math::to_fixed(x, digits).into()))
}

/// The one-argument trigonometric functions (`std::math::sin` … `std::math::atan`);
/// outside an arcsine/arccosine's domain the `f64` operation yields `NaN`.
pub(super) fn native_math_unary_float(
    name: &'static str,
    f: fn(f64) -> f64,
) -> impl Fn(&Interpreter, &[Value]) -> Result<Value, RunError> {
    move |_interp, args| Ok(Value::Number(f(builtin_num_arg(args, 0, name)?)))
}

/// `std::math::atan2(y, x)`: the quadrant-aware arctangent of `y / x`.
pub(super) fn native_math_atan2(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let y = builtin_num_arg(args, 0, "std::math::atan2")?;
    let x = builtin_num_arg(args, 1, "std::math::atan2")?;
    Ok(Value::Number(xulo_std::math::atan2(y, x)))
}

/// `std::math::log(x)` / `std::math::log2(x)` / `std::math::log10(x)`: `-Infinity` at `0`,
/// `NaN` for a negative argument; `std::math::exp(x)`: `e` raised to `x`,
/// `Infinity` once the result overflows.
pub(super) fn native_math_log(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::log(builtin_num_arg(
        args,
        0,
        "std::math::log",
    )?)))
}

pub(super) fn native_math_log2(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::log2(builtin_num_arg(
        args,
        0,
        "std::math::log2",
    )?)))
}

pub(super) fn native_math_log10(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::log10(builtin_num_arg(
        args,
        0,
        "std::math::log10",
    )?)))
}

pub(super) fn native_math_exp(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(Value::Number(xulo_std::math::exp(builtin_num_arg(
        args,
        0,
        "std::math::exp",
    )?)))
}

/// `std::io::read_n(max)`: up to `max` bytes of stdin (1..=16777216), decoded
/// as UTF-8; `Ok(null)` at end of input. A bad `max` is `Err`, not a runtime
/// failure (spec `modules/io.md`).
pub(super) fn native_io_read_n(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let n = builtin_num_arg(args, 0, "std::io::read_n")?;
    let result = if !n.is_finite() {
        Err(format!("std::io::read_n: max must be a number, found {n}"))
    } else {
        let max = n.trunc() as i64;
        xulo_std::io::read_n(max).map(|opt| match opt {
            Some(text) => Value::String(text.into()),
            None => Value::Null,
        })
    };
    Ok(std_result(result))
}

/// `std::io::stdin_is_tty()` / `stdout_is_tty()`: total terminal probes.
pub(super) fn native_io_stdin_is_tty(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    no_arg_guard(args, "std::io::stdin_is_tty")?;
    Ok(Value::Boolean(xulo_std::io::stdin_is_tty()))
}

pub(super) fn native_io_stdout_is_tty(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    no_arg_guard(args, "std::io::stdout_is_tty")?;
    Ok(Value::Boolean(xulo_std::io::stdout_is_tty()))
}

/// The zero-argument clock members of `Time` reject a stray argument the
/// semantic pass would already have rejected (spec `builtins/intrinsic-functions.md`).
pub(super) fn no_arg_guard(args: &[Value], name: &str) -> Result<(), RunError> {
    if args.is_empty() {
        Ok(())
    } else {
        Err(RunError::err(format!("`{name}` takes no arguments"), 0..0))
    }
}

/// `std::time::now`: milliseconds since the Unix epoch (wall clock).
pub(super) fn native_time_now(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    no_arg_guard(args, "std::time::now")?;
    Ok(Value::Number(xulo_std::time::now_millis() as f64))
}

/// `std::time::now_nanos`: nanoseconds since the Unix epoch (wall clock).
pub(super) fn native_time_now_nanos(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    no_arg_guard(args, "std::time::now_nanos")?;
    Ok(Value::Number(xulo_std::time::now_nanos() as f64))
}

/// `std::time::monotonic`: milliseconds on the monotonic clock, measured from the
/// process-wide baseline `xulo-std` fixes at first use.
pub(super) fn native_time_monotonic(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    no_arg_guard(args, "std::time::monotonic")?;
    Ok(Value::Number(xulo_std::time::monotonic_millis() as f64))
}

/// `std::time::monotonic_nanos`: nanoseconds on the monotonic clock.
pub(super) fn native_time_monotonic_nanos(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    no_arg_guard(args, "std::time::monotonic_nanos")?;
    Ok(Value::Number(xulo_std::time::monotonic_nanos() as f64))
}

/// `std::time::sleep(ms)`: a `Task<unit>` that settles at least `ms` milliseconds
/// from now. The deadline is a timer entry `drive` fires once nothing else is
/// ready to run, exactly like a `Task::timeout` deadline; awaiting the task
/// suspends the body until then.
pub(super) fn native_time_sleep(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let ms = builtin_num_arg(args, 0, "std::time::sleep")?;
    if !ms.is_finite() || ms < 0.0 {
        return Err(RunError::err(
            format!("`std::time::sleep` expects a non-negative millisecond count, found {ms}"),
            0..0,
        ));
    }
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_millis(ms.min(u32::MAX as f64) as u64);
    let promise = Rc::new(RefCell::new(crate::value::Promise {
        state: crate::value::PromiseState::Pending,
        awaiters: VecDeque::new(),
    }));
    interp
        .timers
        .borrow_mut()
        .push((deadline, promise.clone(), Value::Null));
    Ok(Value::Promise(promise))
}

/// A non-negative integer millisecond count (`Time::format`/`parts`/
/// `weekday`): the argument domain of the calendar's timestamp.
fn builtin_ms_arg(args: &[Value], i: usize, name: &str) -> Result<u64, RunError> {
    let ms = builtin_num_arg(args, i, name)?;
    if ms.is_finite() && ms.fract() == 0.0 && ms >= 0.0 {
        Ok(ms as u64)
    } else {
        Err(RunError::err(
            format!("`{name}` expects a non-negative integer millisecond count, found {ms}"),
            0..0,
        ))
    }
}

/// `std::time::format(ms)`: the timestamp as ISO 8601 UTC. A `ms` past
/// `9999-12-31T23:59:59.999Z` is a runtime failure (spec
/// `memory-and-runtime.md`), exactly like `Math::to_fixed`'s digits domain.
pub(super) fn native_time_format(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let ms = builtin_ms_arg(args, 0, "std::time::format")?;
    match xulo_std::time::format(ms) {
        Ok(text) => Ok(Value::String(text.into())),
        Err(message) => Err(RunError::err(message, 0..0)),
    }
}

/// `std::time::parse(text)`: the timestamp an ISO 8601 UTC string denotes —
/// `Result::Err` of the built-in `Error` for anything malformed or outside
/// the calendar domain.
pub(super) fn native_time_parse(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let text = builtin_str_arg(args, 0, "std::time::parse")?;
    Ok(std_result(
        xulo_std::time::parse(&text).map(|ms| Value::Number(ms as f64)),
    ))
}

/// `std::time::parts(ms)`: the UTC date and time fields — total for every
/// timestamp, the domain included.
pub(super) fn native_time_parts(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let ms = builtin_ms_arg(args, 0, "std::time::parts")?;
    let p = xulo_std::time::parts(ms);
    Ok(Value::Object(Rc::new(RefCell::new(vec![
        ("year".to_string(), Value::Number(p.year as f64)),
        ("month".to_string(), Value::Number(p.month as f64)),
        ("day".to_string(), Value::Number(p.day as f64)),
        ("hour".to_string(), Value::Number(p.hour as f64)),
        ("minute".to_string(), Value::Number(p.minute as f64)),
        ("second".to_string(), Value::Number(p.second as f64)),
        (
            "millisecond".to_string(),
            Value::Number(p.millisecond as f64),
        ),
    ]))))
}

/// `std::time::from_parts(year, month, day[, hour[, minute[, second[,
/// millisecond]]]])`: the timestamp those UTC fields denote; the time
/// fields default to 0. A field set outside the calendar is a runtime
/// failure.
pub(super) fn native_time_from_parts(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let field = |i: usize, name: &str| -> Result<i64, RunError> {
        match args.get(i) {
            None | Some(Value::Null) => Ok(0),
            Some(Value::Number(n)) if n.is_finite() && n.fract() == 0.0 => Ok(*n as i64),
            other => Err(RunError::err(
                format!(
                    "`std::time::from_parts` `{name}` expects an integer, found {}",
                    other
                        .map(|v| v.kind_name())
                        .unwrap_or_else(|| "nothing".into())
                ),
                0..0,
            )),
        }
    };
    let year = field(0, "year")?;
    let month = field(1, "month")?;
    let day = field(2, "day")?;
    let hour = field(3, "hour")?;
    let minute = field(4, "minute")?;
    let second = field(5, "second")?;
    let millisecond = field(6, "millisecond")?;
    match xulo_std::time::from_parts(year, month, day, hour, minute, second, millisecond) {
        Ok(ms) => Ok(Value::Number(ms as f64)),
        Err(message) => Err(RunError::err(message, 0..0)),
    }
}

/// `std::time::weekday(ms)`: `0` Sunday … `6` Saturday — total for every
/// timestamp.
pub(super) fn native_time_weekday(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let ms = builtin_ms_arg(args, 0, "std::time::weekday")?;
    Ok(Value::Number(xulo_std::time::weekday(ms) as f64))
}

/// Wrap a fallible `std` member's outcome in the built-in `Result`: success
/// carries its payload, a failure message becomes `Result::Err(Error(message))`
/// (spec `error-handling.md`), so a program matches it like any other `Result`
/// (the same construction `Task::timeout` uses). `Fs` and `io` both go
/// through here.
pub(super) fn std_result(result: Result<Value, String>) -> Value {
    match result {
        Ok(value) => result_ok(value),
        Err(message) => result_err(error_value(message.into())),
    }
}

/// `std::fs::read(path)`: the file's contents as a string, or `Err(Error(...))`
/// for a missing directory, an unreadable file, or non-UTF-8 bytes.
pub(super) fn native_fs_read(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::read")?;
    Ok(std_result(
        xulo_std::fs::read(&path).map(|s| Value::String(s.into())),
    ))
}

/// `std::fs::write(path, content)`: create or truncate the file, or `Err(...)` —
/// a missing parent directory is a failure, not a `mkdir`.
pub(super) fn native_fs_write(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::write")?;
    let content = builtin_str_arg(args, 1, "std::fs::write")?;
    Ok(std_result(
        xulo_std::fs::write(&path, &content).map(|()| Value::Null),
    ))
}

/// `std::fs::exists(path)`: total, so a probe that cannot be answered (permission,
/// invalid syntax) simply reads as `false`.
pub(super) fn native_fs_exists(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::exists")?;
    Ok(Value::Boolean(xulo_std::fs::exists(&path)))
}

/// `std::fs::remove(path)`: delete one file; a non-empty directory fails.
pub(super) fn native_fs_remove(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::remove")?;
    Ok(std_result(
        xulo_std::fs::remove(&path).map(|()| Value::Null),
    ))
}

/// `std::fs::mkdir(path)`: create the directory with its missing parents.
pub(super) fn native_fs_mkdir(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::mkdir")?;
    Ok(std_result(xulo_std::fs::mkdir(&path).map(|()| Value::Null)))
}

/// An optional trailing string argument: absent or `null` reads as `None`.
pub(super) fn builtin_opt_str_arg(
    args: &[Value],
    i: usize,
    name: &str,
) -> Result<Option<Rc<str>>, RunError> {
    match args.get(i) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        other => Err(RunError::err(
            format!(
                "`{name}` expects a string or null, found {}",
                other
                    .map(|v| v.kind_name())
                    .unwrap_or_else(|| "nothing".into())
            ),
            0..0,
        )),
    }
}

/// A `List<u8>` argument: each element must be an integer in `0 …= 255`.
pub(super) fn builtin_bytes_arg(args: &[Value], i: usize, name: &str) -> Result<Vec<u8>, RunError> {
    let Some(Value::List(items)) = args.get(i) else {
        let found = args
            .get(i)
            .map(|v| v.kind_name())
            .unwrap_or_else(|| "nothing".into());
        return Err(RunError::err(
            format!("`{name}` expects a `List<u8>`, found {found}"),
            0..0,
        ));
    };
    let items = items.borrow();
    let mut out = Vec::with_capacity(items.len());
    for (idx, item) in items.iter().enumerate() {
        let Value::Number(n) = item else {
            return Err(RunError::err(
                format!("`{name}` expects a `List<u8>`, found a non-byte at index {idx}"),
                0..0,
            ));
        };
        if !n.is_finite() || n.fract() != 0.0 || *n < 0.0 || *n > 255.0 {
            return Err(RunError::err(
                format!("`{name}` expects bytes in 0 …= 255, found {n} at index {idx}"),
                0..0,
            ));
        }
        out.push(*n as u8);
    }
    Ok(out)
}

fn str_list_value(items: Vec<String>) -> Value {
    Value::List(Rc::new(RefCell::new(
        items.into_iter().map(|s| Value::String(s.into())).collect(),
    )))
}

fn byte_list_value(bytes: Vec<u8>) -> Value {
    Value::List(Rc::new(RefCell::new(
        bytes
            .into_iter()
            .map(|b| Value::Number(f64::from(b)))
            .collect(),
    )))
}

/// `std::fs::append(path, contents)`: grow the file, creating it when absent.
pub(super) fn native_fs_append(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::append")?;
    let content = builtin_str_arg(args, 1, "std::fs::append")?;
    Ok(std_result(
        xulo_std::fs::append(&path, &content).map(|()| Value::Null),
    ))
}

/// `std::fs::create_new(path, contents)`: exclusive creation — the path must
/// name nothing yet, and the check is one host operation with the create.
pub(super) fn native_fs_create_new(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::create_new")?;
    let content = builtin_str_arg(args, 1, "std::fs::create_new")?;
    Ok(std_result(
        xulo_std::fs::create_new(&path, &content).map(|()| Value::Null),
    ))
}

/// `std::fs::touch(path)`: create an empty file when absent; refresh an
/// existing entry's stamp, keeping its content.
pub(super) fn native_fs_touch(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::touch")?;
    Ok(std_result(xulo_std::fs::touch(&path).map(|()| Value::Null)))
}

/// `std::fs::set_times(path, modified)`: stamp the modification time in
/// milliseconds since the Unix epoch.
pub(super) fn native_fs_set_times(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::set_times")?;
    let ms = builtin_num_arg(args, 1, "std::fs::set_times")?;
    let ms = if ms.is_finite() && ms >= 0.0 {
        ms as u64
    } else {
        0
    };
    Ok(std_result(
        xulo_std::fs::set_times(&path, ms).map(|()| Value::Null),
    ))
}

/// `std::fs::set_mode(path, mode)`: set the POSIX permission bits; a mode
/// outside `0 …= 4095` fails before the host is touched.
pub(super) fn native_fs_set_mode(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::set_mode")?;
    let mode = builtin_num_arg(args, 1, "std::fs::set_mode")?;
    let mode = if mode.is_finite() && mode >= 0.0 {
        mode as u64
    } else {
        u64::MAX
    };
    Ok(std_result(
        xulo_std::fs::set_mode(&path, mode).map(|()| Value::Null),
    ))
}

/// `std::fs::realpath(path)`: the absolute, link-free path that names it.
pub(super) fn native_fs_realpath(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::realpath")?;
    Ok(std_result(
        xulo_std::fs::realpath(&path).map(|s| Value::String(s.into())),
    ))
}

/// `std::fs::stat(path)`: `{ kind, size, modified, mode }` for the path,
/// following symlinks; anything that is not a file or directory is `Err`.
pub(super) fn native_fs_stat(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::stat")?;
    Ok(std_result(xulo_std::fs::stat(&path).map(|st| {
        Value::Object(Rc::new(RefCell::new(vec![
            ("kind".to_string(), Value::String(st.kind.into())),
            ("size".to_string(), Value::Number(st.size as f64)),
            ("modified".to_string(), Value::Number(st.modified as f64)),
            ("mode".to_string(), Value::Number(st.mode as f64)),
        ])))
    })))
}

/// `std::fs::copy(from, to)`: byte-level copy that replaces `to`.
pub(super) fn native_fs_copy(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let from = builtin_str_arg(args, 0, "std::fs::copy")?;
    let to = builtin_str_arg(args, 1, "std::fs::copy")?;
    Ok(std_result(
        xulo_std::fs::copy(&from, &to).map(|()| Value::Null),
    ))
}

/// `std::fs::rename(from, to)`: move, replacing `to` when the kinds agree.
pub(super) fn native_fs_rename(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let from = builtin_str_arg(args, 0, "std::fs::rename")?;
    let to = builtin_str_arg(args, 1, "std::fs::rename")?;
    Ok(std_result(
        xulo_std::fs::rename(&from, &to).map(|()| Value::Null),
    ))
}

/// `std::fs::remove_dir(path)`: delete the directory and everything in it; a
/// symlink at `path` fails rather than deleting what it points at.
pub(super) fn native_fs_remove_dir(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::remove_dir")?;
    Ok(std_result(
        xulo_std::fs::remove_dir(&path).map(|()| Value::Null),
    ))
}

/// `std::fs::read_dir(path)`: the bare names of the directory's entries.
pub(super) fn native_fs_read_dir(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::read_dir")?;
    Ok(std_result(
        xulo_std::fs::read_dir(&path).map(str_list_value),
    ))
}

/// `std::fs::walk(path)`: the start path and every path beneath it.
pub(super) fn native_fs_walk(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::walk")?;
    Ok(std_result(xulo_std::fs::walk(&path).map(str_list_value)))
}

/// `std::fs::read_bytes(path)`: the whole file as raw bytes; any content is
/// `Ok`, only path failures are `Err`.
pub(super) fn native_fs_read_bytes(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::read_bytes")?;
    Ok(std_result(
        xulo_std::fs::read_bytes(&path).map(byte_list_value),
    ))
}

/// `std::fs::write_bytes(path, bytes)`: create or truncate, then write the
/// bytes; no decoding in either direction.
pub(super) fn native_fs_write_bytes(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::write_bytes")?;
    let bytes = builtin_bytes_arg(args, 1, "std::fs::write_bytes")?;
    Ok(std_result(
        xulo_std::fs::write_bytes(&path, &bytes).map(|()| Value::Null),
    ))
}

/// `std::fs::append_bytes(path, bytes)`: grow the file by the bytes.
pub(super) fn native_fs_append_bytes(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::append_bytes")?;
    let bytes = builtin_bytes_arg(args, 1, "std::fs::append_bytes")?;
    Ok(std_result(
        xulo_std::fs::append_bytes(&path, &bytes).map(|()| Value::Null),
    ))
}

/// `std::fs::temp_file([prefix])`: a unique empty file in the host temp dir.
pub(super) fn native_fs_temp_file(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let prefix = builtin_opt_str_arg(args, 0, "std::fs::temp_file")?;
    Ok(std_result(
        xulo_std::fs::temp_file(prefix.as_deref()).map(|p| Value::String(p.into())),
    ))
}

/// `std::fs::temp_dir([prefix])`: a unique empty directory in the host temp dir.
pub(super) fn native_fs_temp_dir(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let prefix = builtin_opt_str_arg(args, 0, "std::fs::temp_dir")?;
    Ok(std_result(
        xulo_std::fs::temp_dir(prefix.as_deref()).map(|p| Value::String(p.into())),
    ))
}

/// `std::fs::temp_file_in(dir[, prefix])`: a unique empty file inside `dir`.
pub(super) fn native_fs_temp_file_in(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let dir = builtin_str_arg(args, 0, "std::fs::temp_file_in")?;
    let prefix = builtin_opt_str_arg(args, 1, "std::fs::temp_file_in")?;
    Ok(std_result(
        xulo_std::fs::temp_file_in(&dir, prefix.as_deref()).map(|p| Value::String(p.into())),
    ))
}

/// `std::fs::temp_dir_in(dir[, prefix])`: a unique empty directory inside `dir`.
pub(super) fn native_fs_temp_dir_in(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let dir = builtin_str_arg(args, 0, "std::fs::temp_dir_in")?;
    let prefix = builtin_opt_str_arg(args, 1, "std::fs::temp_dir_in")?;
    Ok(std_result(
        xulo_std::fs::temp_dir_in(&dir, prefix.as_deref()).map(|p| Value::String(p.into())),
    ))
}

/// `std::io::print(text)`: `text` on the process's stdout, no line
/// terminator. Unlike the language's `print`, which the interpreter collects
/// into `out` for the host to print when the program ends, this is the real
/// stream — written and flushed as the program runs.
pub(super) fn native_io_print(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let text = builtin_str_arg(args, 0, "std::io::print")?;
    Ok(std_result(xulo_std::io::print(&text).map(|()| Value::Null)))
}

/// `std::io::println(text)`: `text` and a newline on stdout.
pub(super) fn native_io_println(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let text = builtin_str_arg(args, 0, "std::io::println")?;
    Ok(std_result(
        xulo_std::io::println(&text).map(|()| Value::Null),
    ))
}

/// `std::io::eprint(text)`: `text` on the process's stderr.
pub(super) fn native_io_eprint(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let text = builtin_str_arg(args, 0, "std::io::eprint")?;
    Ok(std_result(
        xulo_std::io::eprint(&text).map(|()| Value::Null),
    ))
}

/// `std::io::eprintln(text)`: `text` and a newline on stderr.
pub(super) fn native_io_eprintln(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let text = builtin_str_arg(args, 0, "std::io::eprintln")?;
    Ok(std_result(
        xulo_std::io::eprintln(&text).map(|()| Value::Null),
    ))
}

/// `std::io::flush()`: push buffered stdout to the operating system (the
/// writes above flush themselves).
pub(super) fn native_io_flush(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    no_arg_guard(args, "std::io::flush")?;
    Ok(std_result(xulo_std::io::flush().map(|()| Value::Null)))
}

/// `std::io::read_line()`: the next line of stdin without its terminator, or
/// `Result::Ok(null)` at end of input — the sentinel a line loop breaks on.
/// An empty line is `Result::Ok("")`, so it never passes for the end.
pub(super) fn native_io_read_line(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    no_arg_guard(args, "std::io::read_line")?;
    Ok(std_result(xulo_std::io::read_line().map(
        |line| match line {
            Some(text) => Value::String(text.into()),
            None => Value::Null,
        },
    )))
}

/// `std::io::read()`: all of stdin as a string (`""` at end of input); bytes
/// that are not valid UTF-8 are `Result::Err`.
pub(super) fn native_io_read(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    no_arg_guard(args, "std::io::read")?;
    Ok(std_result(
        xulo_std::io::read().map(|text| Value::String(text.into())),
    ))
}

/// `std::os::getenv(name)`: the variable's value, or `Result::Ok(null)` when
/// unset — the optional-string shape a program matches like `read_line`.
pub(super) fn native_os_getenv(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let name = builtin_str_arg(args, 0, "std::os::getenv")?;
    Ok(std_result(xulo_std::os::getenv(&name).map(
        |opt| match opt {
            Some(value) => Value::String(value.into()),
            None => Value::Null,
        },
    )))
}

/// `std::os::setenv(name, value)`: set the variable for this process, or
/// `Err(...)` for a name the OS would reject (empty, contains `=`).
pub(super) fn native_os_setenv(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let name = builtin_str_arg(args, 0, "std::os::setenv")?;
    let value = builtin_str_arg(args, 1, "std::os::setenv")?;
    Ok(std_result(
        xulo_std::os::setenv(&name, &value).map(|()| Value::Null),
    ))
}

/// `std::os::cwd()`: the current working directory as a string.
pub(super) fn native_os_cwd(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    no_arg_guard(args, "std::os::cwd")?;
    Ok(std_result(
        xulo_std::os::cwd().map(|path| Value::String(path.into())),
    ))
}

/// `std::os::chdir(path)`: change the working directory, or `Err(...)` for a
/// missing / unreadable path.
pub(super) fn native_os_chdir(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::os::chdir")?;
    Ok(std_result(xulo_std::os::chdir(&path).map(|()| Value::Null)))
}

/// `std::os::unsetenv(name)`: remove a variable, or `Err(...)`.
pub(super) fn native_os_unsetenv(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let name = builtin_str_arg(args, 0, "std::os::unsetenv")?;
    Ok(std_result(
        xulo_std::os::unsetenv(&name).map(|()| Value::Null),
    ))
}

/// `std::os::tmp_dir()`: the host's temporary directory — total.
pub(super) fn native_os_tmp_dir(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    no_arg_guard(args, "std::os::tmp_dir")?;
    Ok(Value::String(xulo_std::os::tmp_dir().into()))
}

/// `std::os::home_dir()` / `host_name()` / `user_name()` / `exec_path()`:
/// `Result<string, Error>`.
pub(super) fn native_os_string_result(
    name: &'static str,
    f: fn() -> Result<String, String>,
) -> impl Fn(&Interpreter, &[Value]) -> Result<Value, RunError> {
    move |_interp, args| {
        no_arg_guard(args, name)?;
        Ok(std_result(f().map(|value| Value::String(value.into()))))
    }
}

/// `std::os::cpu_count()`: the logical processors available — total.
pub(super) fn native_os_cpu_count(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    no_arg_guard(args, "std::os::cpu_count")?;
    Ok(Value::Number(xulo_std::os::cpu_count() as f64))
}

/// `std::os::pid()`: the process id — total, so no `Result` wraps it.
pub(super) fn native_os_pid(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    no_arg_guard(args, "std::os::pid")?;
    Ok(Value::Number(xulo_std::os::pid() as f64))
}

/// `std::os::platform()`: the OS family (`"linux"`, `"macos"`, …) — total.
pub(super) fn native_os_platform(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    no_arg_guard(args, "std::os::platform")?;
    Ok(Value::String(xulo_std::os::platform().into()))
}

/// `std::os::arch()`: the CPU architecture (`"x86_64"`, `"aarch64"`, …) —
/// total.
pub(super) fn native_os_arch(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    no_arg_guard(args, "std::os::arch")?;
    Ok(Value::String(xulo_std::os::arch().into()))
}

/// `std::path::join(a, b)`: append `b` to `a` (an absolute `b` replaces
/// `a`); total, so no `Result`.
pub(super) fn native_path_join(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let a = builtin_str_arg(args, 0, "std::path::join")?;
    let b = builtin_str_arg(args, 1, "std::path::join")?;
    Ok(Value::String(xulo_std::path::join(&a, &b).into()))
}

/// `std::path::dirname(path)`: the parent directory (`""` when there is
/// none); total.
pub(super) fn native_path_dirname(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::path::dirname")?;
    Ok(Value::String(xulo_std::path::dirname(&path).into()))
}

/// `std::path::basename(path)`: the final component; total.
pub(super) fn native_path_basename(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::path::basename")?;
    Ok(Value::String(xulo_std::path::basename(&path).into()))
}

/// `std::path::ext(path)`: the final component's extension including the dot
/// (`""` when there is none); total.
pub(super) fn native_path_ext(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::path::ext")?;
    Ok(Value::String(xulo_std::path::ext(&path).into()))
}

/// `std::path::is_absolute(path)`: whether it starts with `/`; total.
pub(super) fn native_path_is_absolute(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::path::is_absolute")?;
    Ok(Value::Boolean(xulo_std::path::is_absolute(&path)))
}

/// `std::path::split(path)`: the `{ dir, base, ext }` record; total.
pub(super) fn native_path_split(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::path::split")?;
    let (dir, base, ext) = xulo_std::path::split(&path);
    Ok(Value::Object(Rc::new(RefCell::new(vec![
        ("dir".to_string(), Value::String(dir.into())),
        ("base".to_string(), Value::String(base.into())),
        ("ext".to_string(), Value::String(ext.into())),
    ]))))
}

/// `std::path::stem(path)`: the basename without its extension; total.
pub(super) fn native_path_stem(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::path::stem")?;
    Ok(Value::String(xulo_std::path::stem(&path).into()))
}

/// `std::path::with_ext(path, ext)`: replace the final component's extension.
pub(super) fn native_path_with_ext(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::path::with_ext")?;
    let ext = builtin_str_arg(args, 1, "std::path::with_ext")?;
    Ok(Value::String(xulo_std::path::with_ext(&path, &ext).into()))
}

/// `std::path::relative(from, to)`: the relative path, or `null` when no such
/// string names the relation (absence, not failure).
pub(super) fn native_path_relative(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let from = builtin_str_arg(args, 0, "std::path::relative")?;
    let to = builtin_str_arg(args, 1, "std::path::relative")?;
    Ok(match xulo_std::path::relative(&from, &to) {
        Some(p) => Value::String(p.into()),
        None => Value::Null,
    })
}

/// `std::path::normalize(path)`: collapse `//`, `.`, and `..`; total.
pub(super) fn native_path_normalize(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::path::normalize")?;
    Ok(Value::String(xulo_std::path::normalize(&path).into()))
}

// ──────────────────────────────────────────────────────────────────
//  std::net — TCP (opaque numeric handles, sync-blocking in v1)
// ──────────────────────────────────────────────────────────────────

/// `std::net::tcp_listen(addr)`: `Result::Ok` of the listener handle, or
/// `Err(Error(...))` for an unresolvable name or a refused bind.
pub(super) fn native_net_tcp_listen(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let addr = builtin_str_arg(args, 0, "std::net::tcp_listen")?;
    Ok(std_result(
        xulo_std::net::tcp_listen(&addr).map(|h| Value::Number(h as f64)),
    ))
}

/// An optional trailing wait budget in milliseconds: absent or `null` means
/// wait without limit.
fn builtin_opt_timeout_ms(args: &[Value], i: usize, name: &str) -> Result<Option<u64>, RunError> {
    match args.get(i) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) if n.is_finite() && *n >= 0.0 => Ok(Some(*n as u64)),
        other => Err(RunError::err(
            format!(
                "`{name}` expects a `u64?` timeout in milliseconds, found {}",
                other
                    .map(|v| v.kind_name())
                    .unwrap_or_else(|| "nothing".into())
            ),
            0..0,
        )),
    }
}

/// `std::net::tcp_accept(listener)`: block for the next connection —
/// `Result::Ok` of the new stream handle.
pub(super) fn native_net_tcp_accept(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let handle = builtin_num_arg(args, 0, "std::net::tcp_accept")? as i64;
    let timeout = builtin_opt_timeout_ms(args, 1, "std::net::tcp_accept")?;
    Ok(std_result(
        xulo_std::net::tcp_accept(handle, timeout).map(|h| Value::Number(h as f64)),
    ))
}

/// `std::net::tcp_connect(addr)`: block until the handshake completes —
/// `Result::Ok` of the stream handle.
pub(super) fn native_net_tcp_connect(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let addr = builtin_str_arg(args, 0, "std::net::tcp_connect")?;
    let timeout = builtin_opt_timeout_ms(args, 1, "std::net::tcp_connect")?;
    Ok(std_result(
        xulo_std::net::tcp_connect(&addr, timeout).map(|h| Value::Number(h as f64)),
    ))
}

/// `std::net::sock_read(handle, max)`: block for up to `max` bytes —
/// `Result::Ok` of text, or `null` at end-of-stream. The size domain is
/// enforced inside `xulo_std::net` (as `Result::Err`, matching the FFI
/// path): negative/NaN saturate to 0 under the `as usize` cast and land on
/// the same "read size" error.
pub(super) fn native_net_sock_read(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let handle = builtin_num_arg(args, 0, "std::net::sock_read")? as i64;
    let max = builtin_num_arg(args, 1, "std::net::sock_read")?;
    let timeout = builtin_opt_timeout_ms(args, 2, "std::net::sock_read")?;
    Ok(std_result(
        xulo_std::net::sock_read(handle, max as usize, timeout).map(|text| match text {
            Some(s) => Value::String(s.into()),
            None => Value::Null,
        }),
    ))
}

/// `std::net::sock_write(handle, text)`: block until the text is written —
/// `Result::Ok` of the byte count.
pub(super) fn native_net_sock_write(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let handle = builtin_num_arg(args, 0, "std::net::sock_write")? as i64;
    let text = builtin_str_arg(args, 1, "std::net::sock_write")?;
    let timeout = builtin_opt_timeout_ms(args, 2, "std::net::sock_write")?;
    Ok(std_result(
        xulo_std::net::sock_write(handle, &text, timeout).map(|n| Value::Number(n as f64)),
    ))
}

/// `std::net::sock_shutdown(handle)`: half-close both directions.
pub(super) fn native_net_sock_shutdown(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let handle = builtin_num_arg(args, 0, "std::net::sock_shutdown")? as i64;
    Ok(std_result(
        xulo_std::net::sock_shutdown(handle).map(|()| Value::Null),
    ))
}

/// `std::net::sock_shutdown_write(handle)`: close only the writing half —
/// the peer sees EOF while this side keeps reading until it closes.
pub(super) fn native_net_sock_shutdown_write(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let handle = builtin_num_arg(args, 0, "std::net::sock_shutdown_write")? as i64;
    Ok(std_result(
        xulo_std::net::sock_shutdown_write(handle).map(|()| Value::Null),
    ))
}

/// `std::net::sock_close(handle)`: drop the socket from the registry; a
/// second close on the same handle is an `Err`.
pub(super) fn native_net_sock_close(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let handle = builtin_num_arg(args, 0, "std::net::sock_close")? as i64;
    Ok(std_result(
        xulo_std::net::sock_close(handle).map(|()| Value::Null),
    ))
}

/// `std::net::sock_local_addr(handle)`: the bound `"ip:port"`.
pub(super) fn native_net_sock_local_addr(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let handle = builtin_num_arg(args, 0, "std::net::sock_local_addr")? as i64;
    Ok(std_result(
        xulo_std::net::sock_local_addr(handle).map(|a| Value::String(a.into())),
    ))
}

/// `std::net::sock_peer_addr(handle)`: the remote `"ip:port"` of a
/// connected stream.
pub(super) fn native_net_sock_peer_addr(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let handle = builtin_num_arg(args, 0, "std::net::sock_peer_addr")? as i64;
    Ok(std_result(
        xulo_std::net::sock_peer_addr(handle).map(|a| Value::String(a.into())),
    ))
}

// ──────────────────────────────────────────────────────────────────
//  std::process — process control and child capture
// ──────────────────────────────────────────────────────────────────

/// `std::process::exit(code)`: terminate the process with `code`. The
/// interpreter drains its buffered prelude `print` lines to stdout first —
/// exactly the lines the CLI would have printed at a normal end — then the
/// platform exit flushes stdout/stderr and never returns.
pub(super) fn native_process_exit(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let code = builtin_num_arg(args, 0, "std::process::exit")?;
    let out = std::mem::take(&mut *interp.out.borrow_mut());
    if !out.is_empty() {
        println!("{}", out.join("\n"));
    }
    xulo_std::process::exit(code as i32);
}

/// `std::process::args()`: the process's argv as `List<string>` (element 0
/// is the program path — the CLI binary or test harness that started us).
pub(super) fn native_process_args(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    no_arg_guard(args, "std::process::args")?;
    let items = xulo_std::process::args()
        .into_iter()
        .map(|a| Value::String(a.into()))
        .collect();
    Ok(Value::List(Rc::new(RefCell::new(items))))
}

/// `std::process::run(cmd[, input[, timeout]])`: capture a child's
/// stdout/stderr and exit code as `Result::Ok({stdout, stderr, code})` — a
/// non-zero child exit is data; a spawn failure, an oversized capture, or an
/// expired budget is `Result::Err`. The optional second argument is the
/// piped stdin (absent = inherit), the third a millisecond budget.
pub(super) fn native_process_run(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let cmd = builtin_str_arg(args, 0, "std::process::run")?;
    let input = builtin_opt_input(args, 1, "std::process::run")?;
    let timeout = builtin_opt_timeout_ms(args, 2, "std::process::run")?;
    Ok(std_result(
        xulo_std::process::run(&cmd, input.as_deref(), timeout).map(run_output_value),
    ))
}

/// `std::process::exec(program, args[, input[, timeout]])`: run `program`
/// directly — no shell, `args` is the child's argv exactly as given — and
/// capture both streams the same way `run` does.
pub(super) fn native_process_exec(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let program = builtin_str_arg(args, 0, "std::process::exec")?;
    let argv = builtin_str_list_arg(args, 1, "std::process::exec")?;
    let input = builtin_opt_input(args, 2, "std::process::exec")?;
    let timeout = builtin_opt_timeout_ms(args, 3, "std::process::exec")?;
    Ok(std_result(
        xulo_std::process::exec(&program, &argv, input.as_deref(), timeout).map(run_output_value),
    ))
}

/// `std::process::launch(cmd[, input[, timeout]])`: start the child through
/// the shell without waiting — `Result::Ok` of the `Child` handle to collect
/// or stop later.
pub(super) fn native_process_launch(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let cmd = builtin_str_arg(args, 0, "std::process::launch")?;
    let input = builtin_opt_input(args, 1, "std::process::launch")?;
    let timeout = builtin_opt_timeout_ms(args, 2, "std::process::launch")?;
    Ok(std_result(
        xulo_std::process::launch(&cmd, input.as_deref(), timeout).map(child_object),
    ))
}

/// An optional stdin string: absent or `null` means inherit (an omitted
/// input is not an empty input — spec `modules/process.md`).
fn builtin_opt_input(args: &[Value], i: usize, name: &str) -> Result<Option<String>, RunError> {
    match args.get(i) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.to_string())),
        other => Err(RunError::err(
            format!(
                "`{name}` input expects a `string?`, found {}",
                other
                    .map(|v| v.kind_name())
                    .unwrap_or_else(|| "nothing".into())
            ),
            0..0,
        )),
    }
}

/// A `List<string>` argument (`std::process::exec`'s argv): every element
/// must be a string, so no shell word-splitting can hide in it.
fn builtin_str_list_arg(args: &[Value], i: usize, name: &str) -> Result<Vec<String>, RunError> {
    let Some(Value::List(items)) = args.get(i) else {
        return Err(RunError::err(
            format!(
                "`{name}` expects a `List<string>` argument, found {}",
                args.get(i)
                    .map(|v| v.kind_name())
                    .unwrap_or_else(|| "nothing".into())
            ),
            0..0,
        ));
    };
    items
        .borrow()
        .iter()
        .map(|v| match v {
            Value::String(s) => Ok(s.to_string()),
            other => Err(RunError::err(
                format!(
                    "`{name}` expects string elements in its `List<string>` argument, found {}",
                    other.kind_name()
                ),
                0..0,
            )),
        })
        .collect()
}

/// One capture record as the `{stdout, stderr, code}` payload object
/// (`run`, `exec`, `Command::run`, `Child::wait`).
fn run_output_value(r: xulo_std::process::RunOutput) -> Value {
    Value::Object(Rc::new(RefCell::new(vec![
        ("stdout".to_string(), Value::String(r.stdout.into())),
        ("stderr".to_string(), Value::String(r.stderr.into())),
        ("code".to_string(), Value::Number(r.code as f64)),
    ])))
}

/// One `Command` specification as the object its setters and runners hang
/// on (spec `modules/process.md`): every setter is pure — it returns a new
/// `Command` with that one setting applied — and `run`/`launch` start the
/// child from the accumulated specification.
pub(super) fn command_object(spec: Rc<xulo_std::process::CommandSpec>) -> Value {
    let input_spec = spec.clone();
    let cwd_spec = spec.clone();
    let env_spec = spec.clone();
    let timeout_spec = spec.clone();
    let run_spec = spec.clone();
    let launch_spec = spec;
    Value::Object(Rc::new(RefCell::new(vec![
        (
            "input".to_string(),
            Value::Native(Rc::new(move |_i, call_args| {
                let s = builtin_str_arg(call_args, 0, "Command::input")?;
                Ok(command_object(Rc::new((*input_spec).clone().input(&s))))
            })),
        ),
        (
            "cwd".to_string(),
            Value::Native(Rc::new(move |_i, call_args| {
                let path = builtin_str_arg(call_args, 0, "Command::cwd")?;
                Ok(command_object(Rc::new((*cwd_spec).clone().cwd(&path))))
            })),
        ),
        (
            "env".to_string(),
            Value::Native(Rc::new(move |_i, call_args| {
                let name = builtin_str_arg(call_args, 0, "Command::env")?;
                let value = builtin_str_arg(call_args, 1, "Command::env")?;
                Ok(command_object(Rc::new(
                    (*env_spec).clone().env(&name, &value),
                )))
            })),
        ),
        (
            "timeout".to_string(),
            Value::Native(Rc::new(move |_i, call_args| {
                let ms = builtin_num_arg(call_args, 0, "Command::timeout")?;
                if !ms.is_finite() || ms < 0.0 {
                    return Err(RunError::err(
                        "`Command::timeout` expects a `u64` budget in milliseconds",
                        0..0,
                    ));
                }
                Ok(command_object(Rc::new(
                    (*timeout_spec).clone().timeout(ms as u64),
                )))
            })),
        ),
        (
            "run".to_string(),
            Value::Native(Rc::new(move |_i, call_args| {
                no_arg_guard(call_args, "Command::run")?;
                Ok(std_result(run_spec.run().map(run_output_value)))
            })),
        ),
        (
            "launch".to_string(),
            Value::Native(Rc::new(move |_i, call_args| {
                no_arg_guard(call_args, "Command::launch")?;
                Ok(std_result(launch_spec.launch().map(child_object)))
            })),
        ),
    ])))
}

/// The `Command` statics as the object `use std::process::Command` binds
/// (and `std::process::Command::…` walks): the two constructors each return
/// a fresh specification — setters reached here are semantic errors before
/// the runtime is ever asked.
pub(super) fn command_static_object() -> Value {
    Value::Object(Rc::new(RefCell::new(vec![
        (
            "shell".to_string(),
            Value::Native(Rc::new(|_i, call_args| {
                let cmd = builtin_str_arg(call_args, 0, "Command::shell")?;
                Ok(command_object(Rc::new(
                    xulo_std::process::CommandSpec::shell(&cmd),
                )))
            })),
        ),
        (
            "direct".to_string(),
            Value::Native(Rc::new(|_i, call_args| {
                let program = builtin_str_arg(call_args, 0, "Command::direct")?;
                let argv = builtin_str_list_arg(call_args, 1, "Command::direct")?;
                Ok(command_object(Rc::new(
                    xulo_std::process::CommandSpec::direct(&program, argv),
                )))
            })),
        ),
    ])))
}

/// One running child as the handle object (spec, "Children"): `wait` is
/// `run` delayed — block, capture both streams, apply the same cap and
/// budget — `kill` forceful termination, `pid` the process id. The handle is
/// shared behind the object so its one-shot discipline holds across calls.
pub(super) fn child_object(handle: xulo_std::process::ChildHandle) -> Value {
    let handle = Rc::new(RefCell::new(handle));
    let wait_h = handle.clone();
    let kill_h = handle.clone();
    let pid_h = handle;
    Value::Object(Rc::new(RefCell::new(vec![
        (
            "wait".to_string(),
            Value::Native(Rc::new(move |_i, call_args| {
                no_arg_guard(call_args, "Child::wait")?;
                Ok(std_result(wait_h.borrow_mut().wait().map(run_output_value)))
            })),
        ),
        (
            "kill".to_string(),
            Value::Native(Rc::new(move |_i, call_args| {
                no_arg_guard(call_args, "Child::kill")?;
                Ok(std_result(kill_h.borrow_mut().kill().map(|()| Value::Null)))
            })),
        ),
        (
            "pid".to_string(),
            Value::Native(Rc::new(move |_i, call_args| {
                no_arg_guard(call_args, "Child::pid")?;
                Ok(Value::Number(pid_h.borrow().pid() as f64))
            })),
        ),
    ])))
}

/// `std::json::parse(text)`: JSON 文本 → 它描述的任意值（对象/列表/
/// 字符串/数字/布尔/null），语法错误是 `Result::Err`。
pub(super) fn native_json_parse(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let text = builtin_str_arg(args, 0, "std::json::parse")?;
    Ok(std_result(
        xulo_std::json::parse(&text).map(|json| crate::runtime::json_to_value(&json)),
    ))
}

/// `std::json::stringify(value)`: 任意值 → 紧凑 JSON 文本；不可序列化的
/// 值（函数、枚举、信号、NaN/Infinity）与环结构是 `Result::Err`。
pub(super) fn native_json_stringify(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let Some(value) = args.first() else {
        return Err(RunError::err(
            "`std::json::stringify` expects 1 argument",
            0..0,
        ));
    };
    let indent = match args.get(1) {
        None | Some(Value::Null) => None,
        Some(Value::Number(n)) if n.is_finite() => Some(n.trunc() as i64),
        Some(other) => {
            return Err(RunError::err(
                format!(
                    "argument to `std::json::stringify` must be `number?`, found `{}`",
                    other.kind_name()
                ),
                0..0,
            ));
        }
    };
    Ok(std_result(
        crate::runtime::value_to_json(value, 0)
            .and_then(|json| match indent {
                None | Some(0) => xulo_std::json::stringify(&json),
                Some(width) => xulo_std::json::stringify_indent(&json, width),
            })
            .map(|text| Value::String(text.into())),
    ))
}

/// `std::json::valid(text)`: whether the text is a well-formed JSON document
/// (the same grammar, depth ceiling and number range as `parse`). Total.
pub(super) fn native_json_valid(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let text = builtin_str_arg(args, 0, "std::json::valid")?;
    Ok(Value::Boolean(xulo_std::json::valid(&text)))
}

/// `std::fmt::format(template, args)`: the template's literal text with each
/// `%` directive replaced by its formatted argument (spec
/// `std-spec/modules/fmt.md`). A malformed template, a directive-count
/// mismatch, or an operand that does not suit its verb is an uncaught
/// runtime failure — the argument domain § 5.3 leaves to stop the program
/// (the shape `std::math::to_fixed`'s `digits` already takes).
pub(super) fn native_fmt_format(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let template = builtin_str_arg(args, 0, "std::fmt::format")?;
    use xulo_std::fmt::FmtArg;
    let arg_values: Vec<Value> = match args.get(1) {
        None => Vec::new(),
        Some(Value::List(items)) => items.borrow().clone(),
        Some(other) => {
            return Err(RunError::err(
                format!(
                    "`std::fmt::format` expects a list for `args`, found {}",
                    other.kind_name()
                ),
                0..0,
            ));
        }
    };
    // A number is one value model: a whole float is its integer (`2` and
    // `2.0` suit `%d` alike). Every other value carries the Output consult:
    // `%v`/`%s` ask its `Display`/`Debug` under the directive's `Format`,
    // then fall back to the table (spec `std-spec/modules/fmt.md`).
    let fmt_args: Vec<FmtArg<'_>> = arg_values
        .iter()
        .map(|value| match value {
            Value::String(s) => FmtArg::Str(s.to_string()),
            Value::Number(n) => {
                // A whole float is its integer (`2` and `2.0` suit `%d`
                // alike), but a negative zero keeps the float slot so `%f`
                // and `%g` can render its sign.
                if n.fract() == 0.0
                    && *n != 0.0
                    && n.is_finite()
                    && (i64::MIN as f64..=i64::MAX as f64).contains(n)
                {
                    FmtArg::Int(*n as i64)
                } else {
                    FmtArg::Float(*n)
                }
            }
            Value::Boolean(b) => FmtArg::Bool(*b),
            Value::Null => FmtArg::Null,
            other => {
                let kind = interp.fmt_user_kind(other).unwrap_or("a value");
                let value = other.clone();
                FmtArg::User {
                    kind,
                    render: Box::new(move |precision, alternate| {
                        interp.fmt_render(&value, precision, alternate)
                    }),
                }
            }
        })
        .collect();
    xulo_std::fmt::format(&template, &fmt_args)
        .map(|text| Value::String(text.into()))
        .map_err(|message| RunError::err(message, 0..0))
}

/// `RegExp::compile(pattern, flags)` (spec `regular-expressions.mdx`):
/// `Result::Ok` of a pattern object carrying its operations, or
/// `Result::Err` of the `RegexError` message the engine reported — an
/// illegal pattern or an unknown flag is data, not a crash.
pub(super) fn native_regex_compile(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let pattern = builtin_str_arg(args, 0, "RegExp::compile")?;
    let flags = match args.get(1) {
        None => String::new().into(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => {
            return Err(RunError::err(
                format!(
                    "argument to `RegExp::compile` must be `string`, found `{}`",
                    other.kind_name()
                ),
                0..0,
            ));
        }
    };
    match xulo_std::regex::compile(&pattern, &flags) {
        Ok(re) => Ok(result_ok(regexp_object(re))),
        Err(message) => Ok(result_err(error_value(message.into()))),
    }
}

/// One compiled `RegExp` as the object instance methods hang on: `test`,
/// `find`, and `find_all` are the engine's queries, each capturing the
/// pattern. `__replace` is the substitution entry point `text.replace(re, …)`
/// hands its arguments to (it needs the pattern behind the string); it is
/// internal — semantic checking answers `no member` for it on `RegExp`.
pub(super) fn regexp_object(re: xulo_std::regex::RegExp) -> Value {
    let re = Rc::new(re);
    let test_re = re.clone();
    let find_re = re.clone();
    let find_all_re = re.clone();
    let split_re = re.clone();
    let replace_re = re;
    let fields: Vec<(String, Value)> = vec![
        (
            "test".to_string(),
            Value::Native(Rc::new(move |_interp, call_args| {
                let input = builtin_str_arg(call_args, 0, "RegExp::test")?;
                Ok(Value::Boolean(test_re.test(&input)))
            })),
        ),
        (
            "find".to_string(),
            Value::Native(Rc::new(move |_interp, call_args| {
                let input = builtin_str_arg(call_args, 0, "RegExp::find")?;
                Ok(match find_re.find(&input) {
                    Some(m) => regex_match_value(&m),
                    None => Value::Null,
                })
            })),
        ),
        (
            "find_all".to_string(),
            Value::Native(Rc::new(move |_interp, call_args| {
                let input = builtin_str_arg(call_args, 0, "RegExp::find_all")?;
                let matches = find_all_re
                    .find_all(&input)
                    .iter()
                    .map(regex_match_value)
                    .collect();
                Ok(Value::List(Rc::new(RefCell::new(matches))))
            })),
        ),
        (
            "__split".to_string(),
            Value::Native(Rc::new(move |_interp, call_args| {
                let input = builtin_str_arg(call_args, 0, "split")?;
                let parts: Vec<Value> = split_re
                    .split(&input)
                    .into_iter()
                    .map(|part| Value::String(part.into()))
                    .collect();
                Ok(Value::List(Rc::new(RefCell::new(parts))))
            })),
        ),
        (
            "__replace".to_string(),
            Value::Native(Rc::new(move |interp, call_args| {
                let input = builtin_str_arg(call_args, 0, "replace")?;
                match call_args.get(1) {
                    Some(Value::String(template)) => {
                        Ok(Value::String(replace_re.replace(&input, template).into()))
                    }
                    Some(func @ (Value::Function(_) | Value::Native(_))) => {
                        let func = func.clone();
                        let out = replace_re.replace_with(&input, |m| {
                            let replacement =
                                interp.call_value_with_values(&func, &[regex_match_value(m)])?;
                            match replacement {
                                Value::String(s) => Ok::<String, RunError>(s.to_string()),
                                other => Err(RunError::err(
                                    format!(
                                        "replacement function must return a `string`, found {}",
                                        other.kind_name()
                                    ),
                                    0..0,
                                )),
                            }
                        })?;
                        Ok(Value::String(out.into()))
                    }
                    Some(other) => Err(RunError::err(
                        format!(
                            "`replace` expects a string template or a function, found {}",
                            other.kind_name()
                        ),
                        0..0,
                    )),
                    None => Err(RunError::err(
                        "`replace` expects a string template or a function, found nothing",
                        0..0,
                    )),
                }
            })),
        ),
    ];
    Value::Object(Rc::new(RefCell::new(fields)))
}

/// One match as the object the language reads: `value` is the matched text,
/// `group(i)` the i-th capture — `null` when that group did not participate
/// or the index is out of range (group 0 is not exposed; `value` already
/// carries the whole match).
pub(super) fn regex_match_value(m: &xulo_std::regex::Match) -> Value {
    let groups: Vec<Option<String>> = m.groups.clone();
    let named: Vec<(String, Option<String>)> = m.named.clone();
    Value::Object(Rc::new(RefCell::new(vec![
        ("value".to_string(), Value::String(m.value.clone().into())),
        ("index".to_string(), Value::Number(m.index as f64)),
        ("end".to_string(), Value::Number(m.end_index as f64)),
        (
            "group".to_string(),
            Value::Native(Rc::new(move |_interp, call_args| match call_args.first() {
                Some(Value::Number(index)) => {
                    if !index.is_finite() || *index < 1.0 || index.fract() != 0.0 {
                        return Ok(Value::Null);
                    }
                    Ok(groups
                        .get(*index as usize - 1)
                        .cloned()
                        .flatten()
                        .map(|s| Value::String(s.into()))
                        .unwrap_or(Value::Null))
                }
                Some(Value::String(name)) => Ok(named
                    .iter()
                    .find(|(n, _)| n == name.as_ref())
                    .and_then(|(_, g)| g.clone())
                    .map(|s| Value::String(s.into()))
                    .unwrap_or(Value::Null)),
                _ => Ok(Value::Null),
            })),
        ),
    ])))
}

pub(super) fn native_print(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let line = args
        .iter()
        .map(|v| interp.render_print(v))
        .collect::<Result<Vec<_>, _>>()?
        .join(" ");
    interp.out.borrow_mut().push(line);
    Ok(Value::Null)
}

pub(super) fn native_str(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(match args.first() {
        Some(v) => Value::String(interp.render_display(v)?.into()),
        None => Value::String(String::new().into()),
    })
}

/// `assert(condition, message)`: a falsy condition stops the program with
/// `panic(message)` (an uncaught throw of the message string), mirroring
/// `Expression::Panic`. Semantic checking already enforced the
/// `bool`/`string` signature; the runtime check is defensive.
pub(super) fn native_assert(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
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

pub(super) fn native_array_new(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
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

pub(super) fn native_array_fill(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
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

pub(super) fn native_array_from(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    match args.first() {
        Some(Value::List(list)) => Ok(Value::List(Rc::new(RefCell::new(list.borrow().clone())))),
        Some(other) => Err(RunError::err(
            format!("Array.clone() expected a list, found {}", other.kind_name()),
            0..0,
        )),
        None => Err(RunError::err("Array.clone() requires 1 argument", 0..0)),
    }
}

/// An integer argument of a built-in call (`int` / `u64` signatures). The
/// semantic pass checks the registered signature; this guard rejects a
/// non-integral value instead of silently truncating it.
pub(super) fn builtin_int_arg(args: &[Value], i: usize, name: &str) -> Result<i64, RunError> {
    let n = builtin_num_arg(args, i, name)?;
    if !n.is_finite() || n.fract() != 0.0 {
        return Err(RunError::err(
            format!("`{name}` expects an integer, found {n}"),
            0..0,
        ));
    }
    Ok(n as i64)
}

/// `std::math::wrapping_add(a, b)` (spec `builtins/intrinsic-functions.md`).
pub(super) fn native_math_wrapping_add(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let a = builtin_int_arg(args, 0, "std::math::wrapping_add")?;
    let b = builtin_int_arg(args, 1, "std::math::wrapping_add")?;
    Ok(Value::Number(xulo_std::math::wrapping_add(a, b) as f64))
}

/// `std::math::wrapping_sub(a, b)`.
pub(super) fn native_math_wrapping_sub(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let a = builtin_int_arg(args, 0, "std::math::wrapping_sub")?;
    let b = builtin_int_arg(args, 1, "std::math::wrapping_sub")?;
    Ok(Value::Number(xulo_std::math::wrapping_sub(a, b) as f64))
}

/// `std::math::wrapping_mul(a, b)`.
pub(super) fn native_math_wrapping_mul(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let a = builtin_int_arg(args, 0, "std::math::wrapping_mul")?;
    let b = builtin_int_arg(args, 1, "std::math::wrapping_mul")?;
    Ok(Value::Number(xulo_std::math::wrapping_mul(a, b) as f64))
}

/// `std::math::pop_count(x)`.
pub(super) fn native_math_pop_count(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let x = builtin_int_arg(args, 0, "std::math::pop_count")?;
    Ok(Value::Number(xulo_std::math::pop_count(x) as f64))
}

/// `std::math::count_leading_zeros(x)`.
pub(super) fn native_math_count_leading_zeros(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let x = builtin_int_arg(args, 0, "std::math::count_leading_zeros")?;
    Ok(Value::Number(xulo_std::math::count_leading_zeros(x) as f64))
}

/// `std::math::count_trailing_zeros(x)`.
pub(super) fn native_math_count_trailing_zeros(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let x = builtin_int_arg(args, 0, "std::math::count_trailing_zeros")?;
    Ok(Value::Number(xulo_std::math::count_trailing_zeros(x) as f64))
}

/// `std::math::rotate_left(x, amount)`: `amount` is `u64` and taken modulo 64.
pub(super) fn native_math_rotate_left(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let x = builtin_int_arg(args, 0, "std::math::rotate_left")?;
    let amount = builtin_int_arg(args, 1, "std::math::rotate_left")? as u64;
    Ok(Value::Number(xulo_std::math::rotate_left(x, amount) as f64))
}

/// `std::math::rotate_right(x, amount)`: `amount` is `u64` and modulo 64.
pub(super) fn native_math_rotate_right(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let x = builtin_int_arg(args, 0, "std::math::rotate_right")?;
    let amount = builtin_int_arg(args, 1, "std::math::rotate_right")? as u64;
    Ok(Value::Number(xulo_std::math::rotate_right(x, amount) as f64))
}

// ──────────────────────────────────────────────────────────────────
//  std::http — the client, the server, and the data types
// ──────────────────────────────────────────────────────────────────

/// The `{ name, value }` record list of `query`/`form`/`headers` as a value:
/// a `List` of two-field objects (spec `modules/http.md`, "Types").
fn http_header_list_value(headers: &[xulo_std::http::Header]) -> Value {
    let items = headers
        .iter()
        .map(|h| {
            Value::Object(Rc::new(RefCell::new(vec![
                ("name".to_string(), Value::String(h.name.clone().into())),
                ("value".to_string(), Value::String(h.value.clone().into())),
            ])))
        })
        .collect();
    Value::List(Rc::new(RefCell::new(items)))
}

/// A `{ name, value }` record list argument (`std::http::request`'s
/// headers): every element must carry string `name`/`value` fields.
fn http_header_list_arg(
    value: &Value,
    name: &str,
) -> Result<Vec<xulo_std::http::Header>, RunError> {
    let Value::List(items) = value else {
        return Err(RunError::err(
            format!(
                "`{name}` headers expect a `List<{{ name: string, value: string }}>`, found {}",
                value.kind_name()
            ),
            0..0,
        ));
    };
    items
        .borrow()
        .iter()
        .map(|item| {
            let Some(Value::Object(fields)) = Some(item) else {
                return Err(RunError::err(
                    format!("`{name}` headers expect `{{ name, value }}` records"),
                    0..0,
                ));
            };
            let field = |key: &str| -> Result<String, RunError> {
                match fields.borrow().iter().find(|(k, _)| k == key) {
                    Some((_, Value::String(s))) => Ok(s.to_string()),
                    _ => Err(RunError::err(
                        format!("`{name}` headers expect string `{key}` fields"),
                        0..0,
                    )),
                }
            };
            Ok(xulo_std::http::Header {
                name: field("name")?,
                value: field("value")?,
            })
        })
        .collect()
}

/// One field of an object value, cloned out.
fn http_field(value: &Value, key: &str) -> Option<Value> {
    match value {
        Value::Object(fields) => fields
            .borrow()
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone()),
        _ => None,
    }
}

/// A handler's `Response` value back into the pure record: the three public
/// fields (`status` defaults to 0, which the server sends as `500`).
fn http_response_from_value(value: &Value) -> Result<xulo_std::http::Response, RunError> {
    let status = match http_field(value, "status") {
        Some(Value::Number(n)) => n as i64,
        _ => 0,
    };
    let headers = match http_field(value, "headers") {
        Some(v) => http_header_list_arg(&v, "Response.headers")?,
        None => Vec::new(),
    };
    let body = match http_field(value, "body") {
        Some(Value::String(s)) => s.to_string(),
        _ => String::new(),
    };
    Ok(xulo_std::http::Response {
        status,
        headers,
        body,
    })
}

/// A `Request` value: the eight public fields, with the type recorded so the
/// lookup methods resolve on it (`req.header(…)`).
fn http_request_value(interp: &Interpreter, req: xulo_std::http::Request) -> Value {
    let obj = Rc::new(RefCell::new(vec![
        ("method".to_string(), Value::String(req.method.into())),
        ("path".to_string(), Value::String(req.path.into())),
        ("query".to_string(), http_header_list_value(&req.query)),
        ("form".to_string(), http_header_list_value(&req.form)),
        ("headers".to_string(), http_header_list_value(&req.headers)),
        ("body".to_string(), Value::String(req.body.into())),
        ("version".to_string(), Value::String(req.version.into())),
        (
            "remote_addr".to_string(),
            Value::String(req.remote_addr.into()),
        ),
    ]));
    interp.register_instance(&obj, "Request");
    Value::Object(obj)
}

/// A `Response` value (`fetch`, `request`, `Response.text`, the handler's
/// answer), with the type recorded so `.header(…)` resolves on it.
fn http_response_value(interp: &Interpreter, res: xulo_std::http::Response) -> Value {
    let obj = Rc::new(RefCell::new(vec![
        ("status".to_string(), Value::Number(res.status as f64)),
        ("headers".to_string(), http_header_list_value(&res.headers)),
        ("body".to_string(), Value::String(res.body.into())),
    ]));
    interp.register_instance(&obj, "Response");
    Value::Object(obj)
}

/// A `Server` value: the two handle methods hang on it directly (the object
/// is opaque — only `listen` issues one).
fn http_server_value(server: xulo_std::http::Server) -> Value {
    let server = Rc::new(server);
    let addr_srv = server.clone();
    let serve_srv = server;
    Value::Object(Rc::new(RefCell::new(vec![
        (
            "addr".to_string(),
            Value::Native(Rc::new(move |_i, call_args| {
                no_arg_guard(call_args, "Server::addr")?;
                Ok(std_result(addr_srv.addr().map(|a| Value::String(a.into()))))
            })),
        ),
        (
            "serve".to_string(),
            Value::Native(Rc::new(move |interp, call_args| {
                let Some(handler) = call_args.first().cloned() else {
                    return Err(RunError::err(
                        "`Server::serve` expects a handler function",
                        0..0,
                    ));
                };
                // The handler runs on the serving task; its own failures and
                // malformed answers become the 500 the client sees.
                let result = serve_srv.serve(|req| {
                    let req_value = http_request_value(interp, req.clone());
                    match interp.call_value_with_values(&handler, &[req_value]) {
                        Ok(value) => match http_response_from_value(&value) {
                            Ok(res) => res,
                            Err(e) => xulo_std::http::Response::text(&run_error_text(e), 500),
                        },
                        Err(e) => xulo_std::http::Response::text(&run_error_text(e), 500),
                    }
                });
                Ok(std_result(result.map(|()| Value::Null)))
            })),
        ),
    ])))
}

/// `std::http::fetch(url[, timeout])`: `GET` and read the whole answer.
pub(super) fn native_http_fetch(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let url = builtin_str_arg(args, 0, "std::http::fetch")?;
    let timeout = builtin_opt_timeout_ms(args, 1, "std::http::fetch")?;
    Ok(std_result(
        xulo_std::http::fetch(&url, timeout).map(|r| http_response_value(interp, r)),
    ))
}

/// `std::http::request(method, url[, headers[, body[, timeout]]])`: any
/// method, with headers and a body.
pub(super) fn native_http_request(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let method = builtin_str_arg(args, 0, "std::http::request")?;
    let url = builtin_str_arg(args, 1, "std::http::request")?;
    let headers = match args.get(2) {
        None | Some(Value::Null) => None,
        Some(v) => Some(http_header_list_arg(v, "std::http::request")?),
    };
    let body = match args.get(3) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.to_string()),
        Some(other) => {
            return Err(RunError::err(
                format!(
                    "`std::http::request` body expects a `string?`, found {}",
                    other.kind_name()
                ),
                0..0,
            ));
        }
    };
    let timeout = builtin_opt_timeout_ms(args, 4, "std::http::request")?;
    Ok(std_result(
        xulo_std::http::request(&method, &url, headers, body.as_deref(), timeout)
            .map(|r| http_response_value(interp, r)),
    ))
}

/// `std::http::listen(addr)`: bind and listen — `Result::Ok` of the opaque
/// `Server` handle.
pub(super) fn native_http_listen(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let addr = builtin_str_arg(args, 0, "std::http::listen")?;
    Ok(std_result(
        xulo_std::http::listen(&addr).map(http_server_value),
    ))
}

/// `Response.text(body[, status])`: the `text/plain` shorthand (status
/// defaults to `200`).
pub(super) fn native_http_response_text(
    interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let body = builtin_str_arg(args, 0, "Response.text")?;
    let status = match args.get(1) {
        None | Some(Value::Null) => 200,
        Some(Value::Number(n)) => *n as i64,
        Some(other) => {
            return Err(RunError::err(
                format!(
                    "argument 2 to `Response.text` must be `number`, found `{}`",
                    other.kind_name()
                ),
                0..0,
            ));
        }
    };
    Ok(http_response_value(
        interp,
        xulo_std::http::Response::text(&body, status),
    ))
}

/// `Response.json(value)`: the `application/json` shorthand over
/// `json.md`'s stringification — unserializable input is `Result::Err`.
pub(super) fn native_http_response_json(
    interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let Some(value) = args.first() else {
        return Err(RunError::err("`Response.json` expects 1 argument", 0..0));
    };
    match crate::runtime::value_to_json(value, 0).and_then(|json| xulo_std::json::stringify(&json))
    {
        Ok(text) => Ok(result_ok(http_response_value(
            interp,
            xulo_std::http::Response::json(&text, 200),
        ))),
        Err(message) => Ok(result_err(error_value(message.into()))),
    }
}

/// The `Request`/`Response` lookup methods (`header`, `query_value`,
/// `form_value`) hang off the type objects with the receiver as their first
/// argument — the instance-method convention of constructed values.
pub(super) fn native_http_request_header(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    http_lookup(args, "Request.header", |req, name| req.header(name))
}

pub(super) fn native_http_request_query_value(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    http_lookup(args, "Request.query_value", |req, name| {
        req.query_value(name)
    })
}

pub(super) fn native_http_request_form_value(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    http_lookup(args, "Request.form_value", |req, name| req.form_value(name))
}

pub(super) fn native_http_response_header(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let (recv, name) = lookup_receiver(args, "Response.header")?;
    let Some(Value::Object(_)) = Some(&recv) else {
        return Err(RunError::err(
            "`Response.header` runs on a Response value",
            0..0,
        ));
    };
    let res = http_response_from_value(&recv)?;
    Ok(match res.header(&name) {
        Some(v) => Value::String(v.into()),
        None => Value::Null,
    })
}

/// The failure text of a handler call — what the 500 answer carries as its
/// body.
fn run_error_text(e: RunError) -> String {
    match e {
        RunError::Err(err) => err.message,
        RunError::Propagate(_) => "handler propagated a stray value".to_string(),
        RunError::Throw(_) => "handler threw".to_string(),
    }
}

/// Shared shape of the `Request` lookup methods: receiver first, name second.
fn http_lookup(
    args: &[Value],
    name: &str,
    get: impl Fn(&xulo_std::http::Request, &str) -> Option<String>,
) -> Result<Value, RunError> {
    let (recv, key) = lookup_receiver(args, name)?;
    let req = http_request_from_value(&recv, name)?;
    Ok(match get(&req, &key) {
        Some(v) => Value::String(v.into()),
        None => Value::Null,
    })
}

/// The instance-method argument convention: `self` first, the call's own
/// arguments after it.
fn lookup_receiver(args: &[Value], name: &str) -> Result<(Value, String), RunError> {
    let Some(recv) = args.first().cloned() else {
        return Err(RunError::err(
            format!("`{name}` runs on a value of its type"),
            0..0,
        ));
    };
    let key = builtin_str_arg(args, 1, name)?;
    Ok((recv, key.to_string()))
}

/// A `Request` value's eight fields back into the pure record (the lookup
/// methods read it).
fn http_request_from_value(value: &Value, name: &str) -> Result<xulo_std::http::Request, RunError> {
    let text = |key: &str| -> Result<String, RunError> {
        match http_field(value, key) {
            Some(Value::String(s)) => Ok(s.to_string()),
            Some(other) => Err(RunError::err(
                format!(
                    "`{name}` expects a Request value (field `{key}` is {})",
                    other.kind_name()
                ),
                0..0,
            )),
            None => Ok(String::new()),
        }
    };
    let pairs = |key: &str| -> Result<Vec<xulo_std::http::Header>, RunError> {
        match http_field(value, key) {
            Some(v) => http_header_list_arg(&v, name),
            None => Ok(Vec::new()),
        }
    };
    Ok(xulo_std::http::Request {
        method: text("method")?,
        path: text("path")?,
        query: pairs("query")?,
        form: pairs("form")?,
        headers: pairs("headers")?,
        body: text("body")?,
        version: text("version")?,
        remote_addr: text("remote_addr")?,
    })
}

/// The `Request` type object: the named-field constructor marker plus the
/// lookup methods. The marker is what `Request(…)` construction resolves
/// through — bare, `std::http::Request(…)`, or an alias.
pub(super) fn http_request_type_object() -> Value {
    Value::Object(Rc::new(RefCell::new(vec![
        ("__ctor".to_string(), Value::String("Request".into())),
        (
            "header".to_string(),
            Value::Native(Rc::new(native_http_request_header)),
        ),
        (
            "query_value".to_string(),
            Value::Native(Rc::new(native_http_request_query_value)),
        ),
        (
            "form_value".to_string(),
            Value::Native(Rc::new(native_http_request_form_value)),
        ),
    ])))
}

/// The `Response` type object: the constructor marker, the two statics, and
/// the lookup method.
pub(super) fn http_response_type_object() -> Value {
    Value::Object(Rc::new(RefCell::new(vec![
        ("__ctor".to_string(), Value::String("Response".into())),
        (
            "text".to_string(),
            Value::Native(Rc::new(native_http_response_text)),
        ),
        (
            "json".to_string(),
            Value::Native(Rc::new(native_http_response_json)),
        ),
        (
            "header".to_string(),
            Value::Native(Rc::new(native_http_response_header)),
        ),
    ])))
}

/// The `Server` type object: opaque — no constructor, no statics. The
/// handle methods live on the server value `listen` issues.
pub(super) fn http_server_type_object() -> Value {
    Value::Object(Rc::new(RefCell::new(vec![(
        "serve".to_string(),
        Value::Native(Rc::new(|_i, _args| {
            Err(RunError::err(
                "`Server.serve` is a method: run it on a `Server` value",
                0..0,
            ))
        })),
    )])))
}
