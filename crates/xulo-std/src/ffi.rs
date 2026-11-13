//! The `extern "C"` entry points compiled (JIT/AOT) code calls for the `Math`,
//! `Time`, and `Path` namespaces. Symbol names are part of the codegen contract
//! (`xulo-compiler`'s `RUNTIME_FNS` registers these exact names); the bodies
//! delegate to [`crate::math`] / [`crate::time`] / [`crate::path`] so the
//! compiled and interpreted paths share one implementation.

use std::cell::RefCell;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;

use crate::{math, path, time};

thread_local! {
    /// C strings handed to compiled code, kept alive until released (the
    /// pointers stay valid for as long as the program may read them).
    static KEPT_STRINGS: RefCell<Vec<CString>> = const { RefCell::new(Vec::new()) };
}

/// Keep `s` alive for the process and return the pointer compiled code sees.
fn keep(s: String) -> *const c_char {
    let c_str = CString::new(s).unwrap();
    let ptr = c_str.as_ptr();
    KEPT_STRINGS.with(|kept| kept.borrow_mut().push(c_str));
    ptr
}

/// A C string argument of a `Path` entry point: a null pointer reads as the
/// empty string, so a bad caller fails in `crate::path` rather than here.
fn cstr_arg(text: *const c_char) -> String {
    if text.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(text).to_str().unwrap_or("").to_string() }
    }
}

/// Release every string this crate handed out. Called from
/// `xulo-runtime`'s `xulo_cleanup`.
pub fn release_strings() {
    KEPT_STRINGS.with(|kept| kept.borrow_mut().clear());
}

/// `std::hint::black_box` compiled entry point for every `i64` slot (integer
/// numbers, booleans, strings, lists, objects, enums). `std::hint::black_box`
/// is an optimization barrier: the AOT caller cannot treat the result as a
/// constant, which keeps benchmark loops from being folded away.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_hint_black_box(value: i64) -> i64 {
    std::hint::black_box(value)
}

/// `std::hint::black_box` for true `f64` slots (float operands).
#[unsafe(no_mangle)]
pub extern "C" fn xulo_hint_black_box_f64(value: f64) -> f64 {
    std::hint::black_box(value)
}

/// `Math::abs` compiled entry point.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_abs(x: f64) -> f64 {
    math::abs(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_min(a: f64, b: f64) -> f64 {
    math::min(a, b)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_max(a: f64, b: f64) -> f64 {
    math::max(a, b)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_floor(x: f64) -> f64 {
    math::floor(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_ceil(x: f64) -> f64 {
    math::ceil(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_round(x: f64) -> f64 {
    math::round(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_trunc(x: f64) -> f64 {
    math::trunc(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_sqrt(x: f64) -> f64 {
    math::sqrt(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_cbrt(x: f64) -> f64 {
    math::cbrt(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_pow(base: f64, exp: f64) -> f64 {
    math::pow(base, exp)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_sin(x: f64) -> f64 {
    math::sin(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_cos(x: f64) -> f64 {
    math::cos(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_tan(x: f64) -> f64 {
    math::tan(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_asin(x: f64) -> f64 {
    math::asin(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_acos(x: f64) -> f64 {
    math::acos(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_atan(x: f64) -> f64 {
    math::atan(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_atan2(y: f64, x: f64) -> f64 {
    math::atan2(y, x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_log(x: f64) -> f64 {
    math::log(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_log2(x: f64) -> f64 {
    math::log2(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_log10(x: f64) -> f64 {
    math::log10(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_exp(x: f64) -> f64 {
    math::exp(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_sign(x: f64) -> f64 {
    math::sign(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_hypot(a: f64, b: f64) -> f64 {
    math::hypot(a, b)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_clamp(x: f64, lo: f64, hi: f64) -> f64 {
    math::clamp(x, lo, hi)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_log1p(x: f64) -> f64 {
    math::log1p(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_expm1(x: f64) -> f64 {
    math::expm1(x)
}

/// `Math::random_int(lo, hi)` compiled entry point: `lo > hi` is a runtime
/// failure (spec `memory-and-runtime.md`).
#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_random_int(lo: i64, hi: i64) -> i64 {
    if lo > hi {
        eprintln!("`Math::random_int` range is empty: lo {lo} is greater than hi {hi}");
        std::process::exit(1);
    }
    math::random_int(lo, hi)
}

/// `Math::to_fixed` compiled entry point: `digits` outside `0..=15` is a
/// runtime failure (spec `memory-and-runtime.md`), matching the interpreter's
/// `RunError`.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_to_fixed(x: f64, digits: i64) -> *const c_char {
    if !(0..=15).contains(&digits) {
        eprintln!("`Math::to_fixed` digits must be an integer in 0..=15, found {digits}");
        std::process::exit(1);
    }
    keep(math::to_fixed(x, digits as usize))
}

/// `Time::now` compiled entry point: milliseconds since the Unix epoch.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_time_now() -> i64 {
    time::now_millis()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_time_now_nanos() -> i64 {
    time::now_nanos()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_time_monotonic() -> i64 {
    time::monotonic_millis()
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_time_monotonic_nanos() -> i64 {
    time::monotonic_nanos()
}

/// `Math::random` compiled entry point: a pseudo-random float in `[0.0, 1.0)`.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_random() -> f64 {
    math::random()
}

/// `Path::join(a, b)` compiled entry point: the joined path as a kept C
/// string (the caller reads it, never frees it — `release_strings` owns the
/// lifetime).
#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_path_join(a: *const c_char, b: *const c_char) -> *const c_char {
    keep(path::join(&cstr_arg(a), &cstr_arg(b)))
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_path_dirname(p: *const c_char) -> *const c_char {
    keep(path::dirname(&cstr_arg(p)))
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_path_basename(p: *const c_char) -> *const c_char {
    keep(path::basename(&cstr_arg(p)))
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_path_ext(p: *const c_char) -> *const c_char {
    keep(path::ext(&cstr_arg(p)))
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_path_normalize(p: *const c_char) -> *const c_char {
    keep(path::normalize(&cstr_arg(p)))
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_path_is_absolute(p: *const c_char) -> i64 {
    i64::from(path::is_absolute(&cstr_arg(p)))
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_path_stem(p: *const c_char) -> *const c_char {
    keep(path::stem(&cstr_arg(p)))
}

#[unsafe(no_mangle)]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn xulo_path_with_ext(p: *const c_char, ext: *const c_char) -> *const c_char {
    keep(path::with_ext(&cstr_arg(p), &cstr_arg(ext)))
}

/// `Math::wrapping_add` / `Math::wrapping_sub` / `Math::wrapping_mul` compiled
/// entry points: two's-complement results modulo 2⁶⁴, never trapping.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_wrapping_add(a: i64, b: i64) -> i64 {
    math::wrapping_add(a, b)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_wrapping_sub(a: i64, b: i64) -> i64 {
    math::wrapping_sub(a, b)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_wrapping_mul(a: i64, b: i64) -> i64 {
    math::wrapping_mul(a, b)
}

/// `Math::pop_count` / `Math::count_leading_zeros` / `Math::count_trailing_zeros`
/// compiled entry points.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_pop_count(x: i64) -> i64 {
    math::pop_count(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_count_leading_zeros(x: i64) -> i64 {
    math::count_leading_zeros(x)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_count_trailing_zeros(x: i64) -> i64 {
    math::count_trailing_zeros(x)
}

/// `Math::rotate_left` / `Math::rotate_right` compiled entry points: the
/// amount is taken modulo 64, so every value is in domain.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_rotate_left(x: i64, amount: u64) -> i64 {
    math::rotate_left(x, amount)
}

#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_rotate_right(x: i64, amount: u64) -> i64 {
    math::rotate_right(x, amount)
}

/// `Time::format` compiled entry point: the ISO 8601 UTC text of a
/// timestamp; a `ms` past the calendar domain is a runtime failure (spec
/// `memory-and-runtime.md`), matching the interpreter's `RunError`.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_time_format(ms: f64) -> *const c_char {
    if !(ms.is_finite() && ms.fract() == 0.0 && ms >= 0.0) {
        eprintln!(
            "`std::time::format` expects a non-negative integer millisecond count, found {ms}"
        );
        std::process::exit(1);
    }
    match time::format(ms as u64) {
        Ok(text) => keep(text),
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}

/// `Time::from_parts` compiled entry point: the timestamp those UTC fields
/// denote; the time fields default to 0 (the IR pads the absent arguments).
/// A field set outside the calendar is a runtime failure.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_time_from_parts(
    year: f64,
    month: f64,
    day: f64,
    hour: f64,
    minute: f64,
    second: f64,
    millisecond: f64,
) -> i64 {
    let field = |x: f64, name: &str| -> i64 {
        if x.is_finite() && x.fract() == 0.0 {
            x as i64
        } else {
            eprintln!("`std::time::from_parts` `{name}` expects an integer, found {x}");
            std::process::exit(1);
        }
    };
    match time::from_parts(
        field(year, "year"),
        field(month, "month"),
        field(day, "day"),
        field(hour, "hour"),
        field(minute, "minute"),
        field(second, "second"),
        field(millisecond, "millisecond"),
    ) {
        Ok(ms) => ms as i64,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(1);
        }
    }
}

/// `Time::weekday` compiled entry point: `0` Sunday … `6` Saturday.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_time_weekday(ms: f64) -> i64 {
    if !(ms.is_finite() && ms.fract() == 0.0 && ms >= 0.0) {
        eprintln!(
            "`std::time::weekday` expects a non-negative integer millisecond count, found {ms}"
        );
        std::process::exit(1);
    }
    time::weekday(ms as u64)
}
