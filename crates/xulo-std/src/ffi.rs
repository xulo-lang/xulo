//! The `extern "C"` entry points compiled (JIT/AOT) code calls for the `Math`
//! and `Time` namespaces. Symbol names are part of the codegen contract
//! (`xulo-compiler`'s `RUNTIME_FNS` registers these exact names); the bodies
//! delegate to [`crate::math`] / [`crate::time`] so the compiled and
//! interpreted paths share one implementation.

use std::cell::RefCell;
use std::ffi::CString;
use std::os::raw::c_char;

use crate::{math, time};

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
