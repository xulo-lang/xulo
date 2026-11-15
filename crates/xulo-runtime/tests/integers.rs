//! The `int`/`u64` value model of stage-1 readiness (WP1): exact 64-bit
//! integers, the spec numeric tower, and its runtime traps (spec
//! `types/primitive-types.md`, `expressions/operators.md`).

use xulo_core::error::XuloError;
use xulo_parser::parse_program;
use xulo_runtime::interpreter::Interpreter;

/// Full pipeline: lex -> parse -> semantic check -> run the interpreter.
fn run(src: &str) -> Result<Vec<String>, XuloError> {
    let program = parse_program(src)?;
    xulo_semantic::analyze(&program)?;
    Interpreter::new().run(&program)
}

fn out(src: &str) -> Vec<String> {
    run(src).unwrap_or_else(|e| panic!("{}", e.message))
}

fn trap(src: &str) -> String {
    match run(src) {
        Ok(lines) => panic!("expected a trap, got {lines:?}"),
        Err(e) => e.message,
    }
}

#[test]
fn integers_are_exact_above_the_f64_mantissa() {
    // 2^53 + 1 and 2^53 are distinct `int`s, even though both round to the
    // same `f64`.
    assert_eq!(
        out("fn main() { print(9007199254740993 == 9007199254740992) }"),
        vec!["false"]
    );
    assert_eq!(
        out("fn main() { print(9007199254740993 > 9007199254740992) }"),
        vec!["true"]
    );
}

#[test]
fn u64_max_binds_prints_and_compares() {
    assert_eq!(
        out("fn main() {
                let max: u64 = 18446744073709551615
                print(max)
                print(max == 18446744073709551615)
                print(max > 9223372036854775807)
            }"),
        vec!["18446744073709551615", "true", "true"]
    );
}

#[test]
fn shifts_keep_all_64_bits() {
    // `<<` shifts the two's-complement representation (spec
    // `expressions/operators.md`): 1 << 62 is positive, 1 << 63 is `int`'s
    // minimum, and bits shifted out are discarded.
    let source = "fn main() {
        print(1 << 62)
        print(1 << 63)
        print((1 << 63) >> 63)
    }";
    assert_eq!(
        out(source),
        vec!["4611686018427387904", "-9223372036854775808", "-1"]
    );
    let errors = trap("fn main() { let n = 64 print(1 << n) }");
    assert!(errors.contains("shift amount out of range"), "{errors}");
}

#[test]
fn integer_division_truncates_and_remainder_keeps_the_dividend_sign() {
    // Spec `expressions/operators.md`: `/` truncates toward zero, `%` takes
    // the dividend's sign.
    assert_eq!(
        out("fn main() {
                print(-7 / 2)
                print(7 / -2)
                print(-7 % 2)
                print(7 % -2)
            }"),
        vec!["-3", "-3", "-1", "1"]
    );
}

#[test]
fn division_by_zero_traps() {
    let errors = trap("fn main() { let z = 0 print(1 / z) }");
    assert!(errors.contains("division by zero"), "{errors}");
    let errors = trap("fn main() { let z = 0 print(1 % z) }");
    assert!(errors.contains("division by zero"), "{errors}");
}

#[test]
fn integer_overflow_traps() {
    let errors = trap(
        "fn main() {
            let max: int = 9223372036854775807
            print(max + 1)
        }",
    );
    assert!(errors.contains("integer overflow"), "{errors}");
}

#[test]
fn u64_subtraction_below_zero_traps() {
    // `T op T` keeps `T` (spec `types/primitive-types.md`, "Arithmetic result
    // types"), so a `u64` difference below zero leaves the range.
    let errors = trap(
        "fn main() {
            let a: u64 = 3
            let b: u64 = 5
            print(a - b)
        }",
    );
    assert!(errors.contains("integer overflow"), "{errors}");
    assert_eq!(out("fn main() { print(0 - 1) }"), vec!["-1"],);
}

#[test]
fn mixed_int_float_arithmetic_promotes_to_float() {
    assert_eq!(
        out("fn main() {
                print(1 + 0.5)
                print(7 / 2.0)
                print(2 * 1.5)
            }"),
        vec!["1.5", "3.5", "3"]
    );
}

#[test]
fn to_fixed_rounds_half_away_from_zero_and_keeps_zeros() {
    assert_eq!(
        out("fn main() {
                print(std::math::to_fixed(3.14159, 2))
                print(std::math::to_fixed(2.5, 0))
                print(std::math::to_fixed(-2.5, 0))
                print(std::math::to_fixed(3.1, 2))
            }"),
        vec!["3.14", "3", "-3", "3.10"]
    );
}

#[test]
fn wrapping_intrinsics_never_trap() {
    assert_eq!(
        out("fn main() {
                print(std::math::wrapping_add(9223372036854775807, 1))
                print(std::math::wrapping_sub(0, 1))
                print(std::math::wrapping_mul(4294967296, 4294967296))
                print(std::math::pop_count(255))
            }"),
        vec!["-9223372036854775808", "-1", "0", "8",]
    );
}

#[test]
fn length_members_are_ints() {
    assert_eq!(
        out("fn main() {
                print([1, 2, 3].len)
                print(\"abc\".len)
                print(\"😀\".byte_len)
            }"),
        vec!["3", "3", "4"]
    );
}

#[test]
fn ranges_iterate_exactly() {
    assert_eq!(
        out("fn main() {
                let mut sum = 0
                for i in 1..=5 { sum = sum + i }
                print(sum)
                print((1..<4).len)
            }"),
        vec!["15", "3"]
    );
}

#[test]
fn json_keeps_integers_exact() {
    assert_eq!(
        out("fn main() {
                match std::json::parse(\"18446744073709551615\") {
                    Result::Ok(v) => print(v)
                    Result::Err(e) => print(e)
                }
                print(std::json::stringify(18446744073709551615))
            }"),
        vec!["18446744073709551615", "Result.Ok(18446744073709551615)"]
    );
}
