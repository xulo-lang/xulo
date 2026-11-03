//! String inspection prelude functions (spec `builtins/prelude.md`) and the
//! integer intrinsics from `builtins/intrinsic-functions.md`, end to end
//! through lex → parse → semantic → native interpreter.

use xulo_core::error::XuloError;
use xulo_parser::parse_program;

fn run(src: &str) -> Result<Vec<String>, XuloError> {
    let program = parse_program(src).unwrap();
    xulo_semantic::analyze(&program)?;
    let interp = xulo_runtime::interpreter::Interpreter::new();
    interp.run(&program)
}

fn run_ok(src: &str) -> Vec<String> {
    run(src).unwrap_or_else(|e| panic!("{}", e.message))
}

#[test]
fn string_members_use_scalar_indices() {
    let out = run_ok(
        r#"
        fn main() {
            print("".len)
            print("hello".len)
            print("héllo".len)
            print("héllo".byte_len)
            print("héllo".code_at(1))
            print("abc".code_at(0))
            print("héllo".slice(1, 4))
            print("abc".slice(1, 1))
            print("hé".bytes())
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "0".to_string(),
            "5".to_string(),
            "5".to_string(),
            "6".to_string(),
            "233".to_string(),
            "97".to_string(),
            "éll".to_string(),
            "".to_string(),
            "[104, 195, 169]".to_string(),
        ]
    );
}

#[test]
fn out_of_range_string_indices_fail_at_runtime() {
    let err = run(r#"
        fn main() {
            print("abc".code_at(3))
        }
        "#)
    .expect_err("index 3 is out of range");
    assert_eq!(err.code(), "E0006");

    let err = run(r#"
        fn main() {
            print("abc".slice(2, 1))
        }
        "#)
    .expect_err("start > end is out of range");
    assert_eq!(err.code(), "E0006");
}

#[test]
fn wrapping_and_bit_intrinsics_follow_64_bit_semantics() {
    let out = run_ok(
        r#"
        fn main() {
            print(std::math::wrapping_add(1, 2))
            print(std::math::wrapping_sub(1, 2))
            print(std::math::wrapping_mul(3, 4))
            print(std::math::pop_count(255))
            print(std::math::pop_count(-1))
            print(std::math::count_leading_zeros(1))
            print(std::math::count_trailing_zeros(8))
            print(std::math::rotate_left(1, 1))
            print(std::math::rotate_left(1, 64))
            print(std::math::rotate_right(2, 1))
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "3".to_string(),
            "-1".to_string(),
            "12".to_string(),
            "8".to_string(),
            "64".to_string(),
            "63".to_string(),
            "3".to_string(),
            "2".to_string(),
            "1".to_string(),
            "1".to_string(),
        ]
    );
}

#[test]
fn shift_amounts_are_checked_at_runtime() {
    assert_eq!(run_ok(r#"fn main() { print(1 << 3) }"#), vec!["8"]);
    assert_eq!(run_ok(r#"fn main() { print(-8 >> 1) }"#), vec!["-4"]);

    let err =
        run(r#"fn main() { let n = 64; print(1 << n) }"#).expect_err("shift by 64 is out of range");
    assert_eq!(err.code(), "E0006");

    let err = run(r#"fn main() { let n = -1; print(1 >> n) }"#)
        .expect_err("negative shift is out of range");
    assert_eq!(err.code(), "E0006");
}

#[test]
fn constant_shift_amounts_outside_the_domain_are_rejected() {
    let err = run(r#"fn main() { print(1 << 64) }"#).expect_err("constant 64");
    assert_eq!(err.code(), "E0210");

    let err = run(r#"fn main() { print(1 >> -1) }"#).expect_err("constant -1");
    assert_eq!(err.code(), "E0210");

    let err = run(r#"fn main() { print(1 << 1.5) }"#).expect_err("fractional amount");
    assert_eq!(err.code(), "E0210");
}
