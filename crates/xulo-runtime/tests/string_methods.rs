//! End-to-end string methods through the full pipeline (lex → parse →
//! semantic → native interpreter): reshaping, querying, splitting, padding,
//! and the argument guards.

use xulo_core::error::XuloError;
use xulo_parser::parse_program;

/// Full pipeline with the semantic pass, like the CLI's `xulo run`.
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
fn reshape_and_query_methods_produce_the_expected_values() {
    let out = run_ok(
        r#"
        fn main() {
            print("  Hi There  ".trim())
            print("ABC".to_lowercase())
            print("abc".to_uppercase())
            print("a,b,c".contains("b"))
            print("a,b,c".contains("z"))
            print("a,b,c".starts_with("a,"))
            print("a,b,c".ends_with(",c"))
            print("hello".length())
            print("héllo".length())
            print("".contains(""))
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "Hi There".to_string(),
            "abc".to_string(),
            "ABC".to_string(),
            "true".to_string(),
            "false".to_string(),
            "true".to_string(),
            "true".to_string(),
            "5".to_string(),
            "5".to_string(),
            "true".to_string(),
        ]
    );
}

#[test]
fn repeat_and_padding_follow_the_documented_edges() {
    let out = run_ok(
        r#"
        fn main() {
            print("ab".repeat(3))
            print("ab".repeat(2.7))
            print("ab".repeat(0))
            print("ab".repeat(-1))
            print("hi".pad_start(5))
            print("hi".pad_end(5))
            print("hi".pad_start(1))
            print("hi".pad_start(-3))
            print("héllo".pad_start(7))
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "ababab".to_string(),
            "abab".to_string(),
            "".to_string(),
            "".to_string(),
            "   hi".to_string(),
            "hi   ".to_string(),
            "hi".to_string(),
            "hi".to_string(),
            "  héllo".to_string(),
        ]
    );
}

#[test]
fn split_yields_every_piece_including_empties() {
    let out = run_ok(
        r#"
        fn main() {
            let parts = "a,,b,c".split(",")
            print(parts[0])
            print(parts[1])
            print(parts[2])
            print(parts[3])
            let chars = "héllo".split("")
            print(chars.join("-"))
            let whole = "no-sep".split(",")
            print(whole[0] == "no-sep")
            let none = "".split(",")
            print(none[0] == "")
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "a".to_string(),
            "".to_string(),
            "b".to_string(),
            "c".to_string(),
            "h-é-l-l-o".to_string(),
            "true".to_string(),
            "true".to_string(),
        ]
    );
}

#[test]
fn arity_and_argument_types_are_enforced() {
    let err = run(r#"fn main() { "a".contains() }"#).expect_err("contains needs an argument");
    assert!(
        err.message.contains("requires exactly 1 argument"),
        "unexpected: {}",
        err.message
    );

    let err = run(r#"fn main() { "a".contains(1) }"#).expect_err("contains needs a string");
    assert!(
        err.message.contains("expects a string argument"),
        "unexpected: {}",
        err.message
    );

    let err = run(r#"fn main() { "a".trim("x") }"#).expect_err("trim takes no arguments");
    assert!(
        err.message.contains("takes no arguments"),
        "unexpected: {}",
        err.message
    );

    let err = run(r#"fn main() { "a".pad_start("5") }"#).expect_err("pad needs a number");
    assert!(
        err.message.contains("expected a number"),
        "unexpected: {}",
        err.message
    );

    let err = run(r#"fn main() { "a".repeat(9999999) }"#).expect_err("repeat is capped");
    assert!(
        err.message.contains("at most"),
        "unexpected: {}",
        err.message
    );

    let err = run(r#"fn main() { "a".pad_end(9999999) }"#).expect_err("pad is capped");
    assert!(
        err.message.contains("at most"),
        "unexpected: {}",
        err.message
    );
}
