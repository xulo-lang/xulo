//! 回归：`xulo-std` 的纯语义（format_number / Math / Time）与解释器、
//! 编译路径共用同一实现，这里锁定各边界的输出。

use xulo_std::format_number;
use xulo_std::math;
use xulo_std::time;

#[test]
fn format_number_renders_like_print() {
    assert_eq!(format_number(3.0), "3");
    assert_eq!(format_number(0.5), "0.5");
    assert_eq!(format_number(-42.25), "-42.25");
    assert_eq!(format_number(1e21), "1000000000000000000000");
    assert_eq!(format_number(f64::NAN), "NaN");
    assert_eq!(format_number(f64::INFINITY), "Infinity");
    assert_eq!(format_number(f64::NEG_INFINITY), "-Infinity");
    assert_eq!(format_number(-0.0), "0");
}

#[test]
fn min_max_propagate_nan() {
    assert!(math::min(f64::NAN, 1.0).is_nan());
    assert!(math::min(1.0, f64::NAN).is_nan());
    assert!(math::max(f64::NAN, 1.0).is_nan());
    assert!(math::max(1.0, f64::NAN).is_nan());
    assert_eq!(math::min(3.0, 9.0), 3.0);
    assert_eq!(math::max(3.0, 9.0), 9.0);
}

// 3.14159 是 spec/回归用例的既定字面量（`std::math::toFixed(3.14159, 2)`），非 PI 近似。
#[allow(clippy::approx_constant)]
#[test]
fn to_fixed_keeps_trailing_zeros_and_rounds_away_from_zero() {
    assert_eq!(math::to_fixed(3.14159, 2), "3.14");
    assert_eq!(math::to_fixed(3.0, 2), "3.00");
    assert_eq!(math::to_fixed(2.5, 0), "3");
    assert_eq!(math::to_fixed(-2.5, 0), "-3");
    assert_eq!(math::to_fixed(-1.05, 1), "-1.1");
    assert_eq!(math::to_fixed(0.5, 3), "0.500");
    // 0.125 二进制精确，half-away-from-zero 进位到 0.13
    assert_eq!(math::to_fixed(0.125, 2), "0.13");
}

#[test]
fn to_fixed_special_values_ignore_digits() {
    assert_eq!(math::to_fixed(f64::INFINITY, 2), "Infinity");
    assert_eq!(math::to_fixed(f64::NEG_INFINITY, 2), "-Infinity");
    assert_eq!(math::to_fixed(f64::NAN, 2), "NaN");
}

#[test]
fn to_fixed_overflowing_scale_falls_back_to_direct_format() {
    // 1e300 × 10^15 溢出为 +Infinity，走直接格式化分支（仍非科学计数法）。
    let fixed = math::to_fixed(1e300, 15);
    assert!(
        !fixed.contains(['e', 'E']),
        "科学计数法不应出现: {}",
        &fixed[..32]
    );
    assert!(fixed.starts_with('1'), "{fixed}");
}

#[test]
fn unary_ops_match_the_spec_examples() {
    assert_eq!(math::abs(-42.0), 42.0);
    assert_eq!(math::floor(3.7), 3.0);
    assert_eq!(math::ceil(3.2), 4.0);
    assert_eq!(math::round(3.5), 4.0);
    assert_eq!(math::round(-2.5), -3.0);
    assert_eq!(math::trunc(-2.7), -2.0);
    assert_eq!(math::sqrt(4.0), 2.0);
    assert!(math::sqrt(-1.0).is_nan());
    assert_eq!(math::cbrt(-8.0), -2.0);
    assert_eq!(math::pow(2.0, 10.0), 1024.0);
    assert_eq!(math::log(std::f64::consts::E), 1.0);
    assert_eq!(math::log2(8.0), 3.0);
    assert_eq!(math::log10(1000.0), 3.0);
    assert_eq!(math::exp(0.0), 1.0);
    assert_eq!(math::atan2(1.0, 1.0), std::f64::consts::FRAC_PI_4);
    assert_eq!(math::sin(0.0), 0.0);
    assert_eq!(math::cos(0.0), 1.0);
    assert!(math::asin(2.0).is_nan());
}

#[test]
fn math_constants_match_std() {
    assert_eq!(math::PI, std::f64::consts::PI);
    assert_eq!(math::E, std::f64::consts::E);
    assert_eq!(math::TAU, std::f64::consts::TAU);
    assert_eq!(math::INFINITY, f64::INFINITY);
    assert!(math::NAN.is_nan());
}

#[test]
fn clocks_are_non_negative_and_monotonic_moves_forward() {
    assert!(time::now_millis() >= 0);
    assert!(time::now_nanos() > 0);
    let first = time::monotonic_millis();
    let first_nanos = time::monotonic_nanos();
    let second = time::monotonic_millis();
    assert!(second >= first);
    assert!(time::monotonic_nanos() >= first_nanos);
}
