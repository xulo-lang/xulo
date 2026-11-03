//! Built-in `string` members through the full pipeline (lex → parse →
//! semantic → native interpreter): `.len`, `.byte_len`, `.code_at`, `.slice`,
//! `.bytes()`, and their runtime failures (spec `types/primitive-types.md`).

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
fn string_members_produce_the_expected_values() {
    let out = run_ok(
        r#"
        fn main() {
            print("  Hi There  ".len)
            print("ABC".byte_len)
            print("a,b,c".code_at(0))
            print("hello".slice(1, 4))
            print("hé".bytes())
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "12".to_string(),
            "3".to_string(),
            "97".to_string(),
            "ell".to_string(),
            "[104, 195, 169]".to_string(),
        ]
    );
}

#[test]
fn string_slice_and_code_at_reject_out_of_range() {
    let err = run(r#"fn main() { "abc".code_at(3) }"#).expect_err("index out of range");
    assert_eq!(err.code(), "E0006");

    let err = run(r#"fn main() { "abc".slice(2, 1) }"#).expect_err("start > end");
    assert_eq!(err.code(), "E0006");

    let err = run(r#"fn main() { "abc".slice(0, 4) }"#).expect_err("end out of range");
    assert_eq!(err.code(), "E0006");
}

#[test]
fn string_len_is_a_property_not_a_call() {
    let err = run(r#"fn main() { "abc".len() }"#).expect_err("len is not callable");
    assert!(err.message.contains("property"), "{}", err.message);
}
