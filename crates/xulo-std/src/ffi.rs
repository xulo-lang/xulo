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

/// `Math::toFixed` compiled entry point: `digits` outside `0..=15` is a
/// runtime failure (spec `memory-and-runtime.md`), matching the interpreter's
/// `RunError`.
#[unsafe(no_mangle)]
pub extern "C" fn xulo_math_to_fixed(x: f64, digits: i64) -> *const c_char {
    if !(0..=15).contains(&digits) {
        eprintln!("`Math::toFixed` digits must be an integer in 0..=15, found {digits}");
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
