use super::*;

/// `spawn.process Type { … }`: box a service around the already-built state and
/// the compiled `on_message` function address.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_service_new(state: i64, on_message: i64) -> *mut XuloService {
    let service = Box::new(XuloService {
        state,
        on_message,
        stopped: false,
    });
    let ptr = Box::into_raw(service);
    SERVICES.with(|services| services.borrow_mut().push(ptr));
    ptr
}

/// `send(msg)` on the compiled path: the service is single-threaded and every
/// handler call completes eagerly, so deliver immediately and return the value
/// the handler passed to `reply` (null when it replied with nothing).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_service_send(service: *mut XuloService, value: i64, tag: i64) -> i64 {
    if service.is_null() {
        return 0;
    }
    let service = unsafe { &*service };
    if service.stopped || service.on_message == 0 {
        return 0;
    }
    // The message is a value: copy it in transit (spec `concurrency.md`) and
    // expose its runtime tag to an `unknown` handler parameter.
    let message = xulo_value_copy(value, tag);
    xulo_set_call_tag(1, tag);
    SERVICE_REPLY.with(|reply| *reply.borrow_mut() = None);
    let handler: unsafe extern "C" fn(i64, i64) =
        unsafe { std::mem::transmute(service.on_message) };
    unsafe {
        handler(service.state, message);
    }
    let (value, tag) = SERVICE_REPLY
        .with(|reply| reply.borrow_mut().take())
        .unwrap_or((0, 6));
    if tag == 2 {
        xulo_register_float(value);
    }
    value
}

/// `stop()` on the compiled path: further sends are ignored (the checked
/// example only stops at the end).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_service_stop(service: *mut XuloService) {
    if !service.is_null() {
        unsafe {
            (*service).stopped = true;
        }
    }
}

/// `reply(value)` inside `on_message`: record the value (and tag) the service
/// send should return.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_service_reply(value: i64, tag: i64) {
    if tag == 2 {
        xulo_register_float(value);
    }
    SERVICE_REPLY.with(|reply| *reply.borrow_mut() = Some((value, tag)));
}

/// `Task::race([t, u])` on the compiled path. Async calls complete eagerly
/// there, so every input has already settled by the time the combinator runs
/// and the first argument is the first to settle (spec `Task::race`). The
/// empty case yields `null` instead of hanging; the interpreter keeps waiting
/// forever on an empty race, so a program that reaches it is already outside
/// the checked forms.
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_task_race(list: *mut XuloArray) -> i64 {
    unsafe {
        if list.is_null() {
            return 0;
        }
        (*list).elements.first().copied().unwrap_or(0)
    }
}

/// `Task::timeout(t, ms, elapsed)` on the compiled path: `t` has already run,
/// so "settled within `ms`" is decided from the measured elapsed nanoseconds.
/// A miss becomes `Result::Err(Error("timed out"))`, exactly the value
/// `await Task::timeout(...)` produces under the interpreter
/// (spec `error-handling.md`).
#[unsafe(no_mangle)]
pub extern "C" fn xulo_task_timeout(
    value: i64,
    tag: i64,
    start_ns: i64,
    end_ns: i64,
    limit_ms: f64,
) -> *mut XuloEnum {
    let elapsed = end_ns.saturating_sub(start_ns).max(0);
    // The interpreter turns the count into `Duration::from_millis`, which
    // truncates fractions, and clamps it to the timer's 32-bit range.
    let limit_ns = if limit_ms.is_finite() && limit_ms > 0.0 {
        (limit_ms.min(u32::MAX as f64) as u64).saturating_mul(1_000_000) as i64
    } else {
        0
    };
    if elapsed <= limit_ns {
        let ok = xulo_alloc_enum(c"Result".as_ptr(), c"Ok".as_ptr(), 0, 1);
        xulo_enum_set(ok, 0, value, tag);
        ok
    } else {
        let error = xulo_alloc_object(1);
        xulo_object_set(
            error,
            c"message".as_ptr(),
            intern_string("timed out") as i64,
            0,
        );
        let err = xulo_alloc_enum(c"Result".as_ptr(), c"Err".as_ptr(), 1, 1);
        xulo_enum_set(err, 0, error as i64, 4);
        err
    }
}
