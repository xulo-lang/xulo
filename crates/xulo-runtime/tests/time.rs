//! End-to-end `Time`'s calendar through the full pipeline (lex → parse →
//! semantic → native interpreter): spec `std-spec/modules/time.md`,
//! "Calendar" — UTC alone over the `0 …= 253402300799999` ms domain.

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
fn calendar_roundtrips_and_answers_the_spec_examples() {
    let out = run_ok(
        r#"
        fn main() {
            let noon = std::time::fromParts(2026, 10, 26, 12)
            print(std::time::format(noon))
            match std::time::parse("2026-10-26T12:00:00.000Z") {
                Result::Ok(ms) => print(ms == noon)
                Result::Err(e) => print(e.message)
            }
            let f = std::time::parts(noon)
            print(`${f.year}-${f.month}-${f.day}`)
            print(str(std::time::weekday(noon)))
            print(str(std::time::weekday(0)))
            print(std::time::format(std::time::fromParts(2026, 10, 26)))
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "2026-10-26T12:00:00.000Z".to_string(),
            "true".to_string(),
            "2026-10-26".to_string(),
            "1".to_string(),
            "4".to_string(),
            "2026-10-26T00:00:00.000Z".to_string(),
        ],
        "{out:?}"
    );
}

#[test]
fn parse_failures_are_error_values() {
    let out = run_ok(
        r#"
        fn main() {
            match std::time::parse("2026-02-30T00:00:00Z") {
                Result::Ok(ms) => print(ms)
                Result::Err(e) => print("err")
            }
            match std::time::parse("1969-12-31T23:59:59Z") {
                Result::Ok(ms) => print(ms)
                Result::Err(e) => print("err")
            }
        }
        "#,
    );
    assert_eq!(out, vec!["err".to_string(), "err".to_string()], "{out:?}");
}

#[test]
fn domain_violations_are_runtime_failures_that_stop_the_program() {
    // `format` past 9999-12-31 — the toFixed-shape argument-domain failure.
    let err = run(r#"fn main() { print(std::time::format(253402300800000)) }"#)
        .expect_err("past the domain");
    assert!(err.message.contains("9999-12-31"), "{}", err.message);
    // `fromParts` with a date the calendar does not have.
    let err = run(r#"fn main() { print(str(std::time::fromParts(2026, 2, 30))) }"#)
        .expect_err("no such date");
    assert!(err.message.contains("real instant"), "{}", err.message);
}

/// `examples/time.xulo` 全文经解释器运行的输出逐行固定（示例即测试）。
#[test]
fn time_example() {
    let out = run_ok(include_str!("../../../examples/time.xulo"));
    assert_eq!(
        out,
        vec![
            "true".to_string(),
            "true".to_string(),
            "true".to_string(),
            "true".to_string(),
            "awake".to_string(),
            "2026-10-26T12:00:00.000Z".to_string(),
            "true".to_string(),
            "2026 10 26".to_string(),
            "12 0 0 0".to_string(),
            "1".to_string(),
            "4".to_string(),
            "2026-10-26T00:00:00.000Z".to_string(),
            "not a date".to_string(),
        ]
    );
}
