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

/// `std::math::toFixed(x, digits)`: the decimal numeral of `x` with exactly
/// `digits` digits after the decimal point, rounded halfway away from zero
/// and keeping trailing zeros (spec `builtins/intrinsic-functions.md`).
/// Special values ignore `digits`. A `digits` value outside `0..=15` is a
/// runtime failure (spec `memory-and-runtime.md`).
pub(super) fn native_math_to_fixed(
    _interp: &Interpreter,
    args: &[Value],
) -> Result<Value, RunError> {
    let x = builtin_num_arg(args, 0, "std::math::toFixed")?;
    let digits = match args.get(1) {
        Some(Value::Number(d)) if d.fract() == 0.0 && (0.0..=15.0).contains(d) => *d as usize,
        Some(Value::Number(d)) => {
            return Err(RunError::err(
                format!("`std::math::toFixed` digits must be an integer in 0..=15, found {d}"),
                0..0,
            ));
        }
        other => {
            return Err(RunError::err(
                format!(
                    "`std::math::toFixed` expects a number for `digits`, found {}",
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

/// Wrap a fallible `Fs` outcome in the built-in `Result`: success carries
/// its payload, a failure message becomes `Result::Err(Error(message))`
/// (spec `error-handling.md`), so a program matches it like any other
/// `Result` (the same construction `Task::timeout` uses).
pub(super) fn fs_result(result: Result<Value, String>) -> Value {
    match result {
        Ok(value) => result_ok(value),
        Err(message) => result_err(error_value(message.into())),
    }
}

/// `std::fs::read(path)`: the file's contents as a string, or `Err(Error(...))`
/// for a missing directory, an unreadable file, or non-UTF-8 bytes.
pub(super) fn native_fs_read(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::read")?;
    Ok(fs_result(
        xulo_std::fs::read(&path).map(|s| Value::String(s.into())),
    ))
}

/// `std::fs::write(path, content)`: create or truncate the file, or `Err(...)` —
/// a missing parent directory is a failure, not a `mkdir`.
pub(super) fn native_fs_write(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::write")?;
    let content = builtin_str_arg(args, 1, "std::fs::write")?;
    Ok(fs_result(
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
    Ok(fs_result(xulo_std::fs::remove(&path).map(|()| Value::Null)))
}

/// `std::fs::mkdir(path)`: create the directory with its missing parents.
pub(super) fn native_fs_mkdir(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let path = builtin_str_arg(args, 0, "std::fs::mkdir")?;
    Ok(fs_result(xulo_std::fs::mkdir(&path).map(|()| Value::Null)))
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
    Value::Object(Rc::new(RefCell::new(vec![
        ("value".to_string(), Value::String(m.value.clone().into())),
        (
            "group".to_string(),
            Value::Native(Rc::new(move |_interp, call_args| {
                let index = builtin_num_arg(call_args, 0, "RegexMatch::group")?;
                if !index.is_finite() || index < 1.0 {
                    return Ok(Value::Null);
                }
                Ok(groups
                    .get(index as usize - 1)
                    .cloned()
                    .flatten()
                    .map(|s| Value::String(s.into()))
                    .unwrap_or(Value::Null))
            })),
        ),
    ])))
}

pub(super) fn native_print(interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    let line = args
        .iter()
        .map(|v| v.format())
        .collect::<Vec<_>>()
        .join(" ");
    interp.out.borrow_mut().push(line);
    Ok(Value::Null)
}

pub(super) fn native_str(_interp: &Interpreter, args: &[Value]) -> Result<Value, RunError> {
    Ok(match args.first() {
        Some(v) => Value::String(v.format().into()),
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
