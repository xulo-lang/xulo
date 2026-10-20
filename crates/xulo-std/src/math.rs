//! The `Math` namespace (spec `builtins/intrinsic-functions.md`): constants
//! and pure `f64` operations, shared by the interpreter's `native_math_*`
//! adapters and the compiled path's [`crate::ffi`] entry points.

use crate::format::format_number;

/// `Math::PI`.
pub const PI: f64 = std::f64::consts::PI;
/// `Math::E`.
pub const E: f64 = std::f64::consts::E;
/// `Math::TAU`.
pub const TAU: f64 = std::f64::consts::TAU;
/// `Math::INFINITY`.
pub const INFINITY: f64 = f64::INFINITY;
/// `Math::NAN`.
pub const NAN: f64 = f64::NAN;

/// `Math::abs(x)`: the absolute value.
pub fn abs(x: f64) -> f64 {
    x.abs()
}

/// `Math::min(a, b)`: the smaller operand. A `NaN` operand propagates, so the
/// result is `NaN` (Rust's `f64::min` treats `NaN` as "missing" instead).
pub fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// `Math::max(a, b)`: the larger operand, with `NaN` propagation like
/// [`min`].
pub fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// `Math::floor(x)`: the largest integer value not greater than `x`.
pub fn floor(x: f64) -> f64 {
    x.floor()
}

/// `Math::ceil(x)`: the smallest integer value not less than `x`.
pub fn ceil(x: f64) -> f64 {
    x.ceil()
}

/// `Math::round(x)`: the nearest integer value, halfway cases away from zero.
pub fn round(x: f64) -> f64 {
    x.round()
}

/// `Math::trunc(x)`: the integer part of `x`, discarding the fraction.
pub fn trunc(x: f64) -> f64 {
    x.trunc()
}

/// `Math::sqrt(x)`: the square root; a negative operand yields `NaN`.
pub fn sqrt(x: f64) -> f64 {
    x.sqrt()
}

/// `Math::cbrt(x)`: the cube root, defined for every `float`.
pub fn cbrt(x: f64) -> f64 {
    x.cbrt()
}

/// `Math::pow(base, exp)`: `base` raised to `exp`.
pub fn pow(base: f64, exp: f64) -> f64 {
    base.powf(exp)
}

/// `Math::sin(x)`: the sine of an angle in radians.
pub fn sin(x: f64) -> f64 {
    x.sin()
}

/// `Math::cos(x)`: the cosine of an angle in radians.
pub fn cos(x: f64) -> f64 {
    x.cos()
}

/// `Math::tan(x)`: the tangent of an angle in radians.
pub fn tan(x: f64) -> f64 {
    x.tan()
}

/// `Math::asin(x)`: the arcsine of `x`; outside `[-1, 1]` the result is `NaN`.
pub fn asin(x: f64) -> f64 {
    x.asin()
}

/// `Math::acos(x)`: the arccosine of `x`; outside `[-1, 1]` the result is
/// `NaN`.
pub fn acos(x: f64) -> f64 {
    x.acos()
}

/// `Math::atan(x)`: the arctangent of `x`.
pub fn atan(x: f64) -> f64 {
    x.atan()
}

/// `Math::atan2(y, x)`: the quadrant-aware arctangent of `y / x`.
pub fn atan2(y: f64, x: f64) -> f64 {
    y.atan2(x)
}

/// `Math::log(x)`: the natural logarithm — `-Infinity` at `0`, `NaN` for a
/// negative argument.
pub fn log(x: f64) -> f64 {
    x.ln()
}

/// `Math::log2(x)`: the base-2 logarithm.
pub fn log2(x: f64) -> f64 {
    x.log2()
}

/// `Math::log10(x)`: the base-10 logarithm.
pub fn log10(x: f64) -> f64 {
    x.log10()
}

/// `Math::exp(x)`: `e` raised to `x`, `Infinity` once the result overflows.
pub fn exp(x: f64) -> f64 {
    x.exp()
}

/// `Math::toFixed(x, digits)`: the decimal numeral of `x` with exactly
/// `digits` digits after the decimal point, rounded halfway away from zero
/// and keeping trailing zeros. Special values ignore `digits`.
///
/// `digits` is assumed to be an integer in `0..=15`: the interpreter rejects
/// anything else with a `RunError`, the compiled path's
/// [`crate::ffi::xulo_math_to_fixed`] with a runtime failure (spec
/// `memory-and-runtime.md`).
pub fn to_fixed(x: f64, digits: usize) -> String {
    if x.is_nan() || x.is_infinite() {
        return format_number(x);
    }
    let factor = 10f64.powi(digits as i32);
    let scaled = x * factor;
    // Scaling an enormous operand can overflow to infinity; its decimal
    // numeral then carries no meaningful fractional part and formatting the
    // operand directly is the fixed-width answer.
    if scaled.is_infinite() {
        format!("{x:.*}", digits)
    } else {
        format!("{:.*}", digits, scaled.round() / factor)
    }
}
