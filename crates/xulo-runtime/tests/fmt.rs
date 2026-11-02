//! End-to-end `std::fmt` through the full pipeline (lex → parse → semantic →
//! native interpreter): the checked examples of `std-spec/modules/fmt.md`,
//! the import forms, and the runtime-failure shapes (a malformed template, a
//! mismatched operand, a directive count that disagrees with `args`).

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
fn fmt_checked_call_examples_print_their_results() {
    let out = run_ok(
        r#"
        fn main() {
            print(std::fmt::format("%6.2f", [3.14159]))
            print(std::fmt::format("100%%"))
            print(std::fmt::format("%-8s|%d", ["hi", 42]))
            print(std::fmt::format("%#08x", [255]))
            print(std::fmt::format("%.3g, %q", [0.1, "hi"]))
            print(std::fmt::format("%d", [-42]))
            print(std::fmt::format("%x / %#X", [255, 255]))
            print(std::fmt::format("%b", [5]))
            print(std::fmt::format("%f", [2.5]))
            print(std::fmt::format("%e", [1234.5]))
            print(std::fmt::format("%.3g", [0.1]))
            print(std::fmt::format("%g", [0.0000001]))
            print(std::fmt::format("%q", ["a\n"]))
            print(std::fmt::format("%c", [8364]))
            print(std::fmt::format("%t / %v", [true, null]))
            print(std::fmt::format("%-6s|", ["hi"]))
            print(std::fmt::format("%+8.2f", [3.1]))
            print(std::fmt::format("%.3s", ["hello"]))
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "  3.14",
            "100%",
            "hi      |42",
            "0x0000ff",
            "0.1, \"hi\"",
            "-42",
            "ff / 0XFF",
            "101",
            "2.500000",
            "1.234500e+03",
            "0.1",
            "1e-07",
            "\"a\\n\"",
            "€",
            "true / null",
            "hi    |",
            "   +3.10",
            "hel",
        ]
    );
}

#[test]
fn fmt_omitted_args_and_import_forms_work() {
    let out = run_ok(
        r#"
        use std::fmt::*;
        use std::fmt as f;
        fn main() {
            print(format("100%%"))
            print(f::format("%x", [255]))
            print(format("%d", [42]))
        }
        "#,
    );
    assert_eq!(out, vec!["100%", "ff", "42"]);
}

#[test]
fn fmt_renders_containers_numbers_and_booleans_through_v_and_s() {
    let out = run_ok(
        r#"
        fn main() {
            print(std::fmt::format("%s", [[1, 2]]))
            print(std::fmt::format("%v", [{ a: 1 }]))
            print(std::fmt::format("%v", [3]))
            print(std::fmt::format("%v", [2.5]))
            print(std::fmt::format("%s", [true]))
            print(std::fmt::format("%d", [2.0]))
        }
        "#,
    );
    assert_eq!(
        out,
        vec!["[1, 2]", "{ a: 1 }", "3", "2.5", "true", "2"]
    );
}

#[test]
fn fmt_runtime_failures_stop_the_program() {
    let template = run(r#"fn main() { print(std::fmt::format("%z", [1])) }"#).unwrap_err();
    assert!(template.message.contains("unknown verb `z`"), "{}", template.message);

    let kind = run(r#"fn main() { print(std::fmt::format("%d", ["a"])) }"#).unwrap_err();
    assert!(
        kind.message.contains("`%d` takes an int value, found a string"),
        "{}",
        kind.message
    );

    let count = run(r#"fn main() { print(std::fmt::format("%d")) }"#).unwrap_err();
    assert!(
        count.message.contains("expects 1 argument, found 0"),
        "{}",
        count.message
    );

    let flag = run(r#"fn main() { print(std::fmt::format("%+s", ["x"])) }"#).unwrap_err();
    assert!(
        flag.message.contains("flag `+` is not valid for verb `s`"),
        "{}",
        flag.message
    );

    let precision = run(r#"fn main() { print(std::fmt::format("%.3t", [true])) }"#).unwrap_err();
    assert!(
        precision
            .message
            .contains("precision is not valid for verb `t`"),
        "{}",
        precision.message
    );
}

#[test]
fn fmt_special_values_and_negative_zero_come_through() {
    let out = run_ok(
        r#"
        fn main() {
            print(std::fmt::format("%f", [std::math::NAN]))
            print(std::fmt::format("%f", [std::math::INFINITY]))
            print(std::fmt::format("%f", [-std::math::INFINITY]))
            print(std::fmt::format("%g", [-0.0]))
        }
        "#,
    );
    assert_eq!(out, vec!["NaN", "Infinity", "-Infinity", "-0"]);
}


#[test]
fn fmt_text_verbs_consult_display_then_debug_with_the_directive_format() {
    let out = run_ok(
        r#"
        #Derive(Debug)
        struct Point { x: int, y: int }
        struct Money { cents: int }
        impl Display for Money {
            fn display(self, format: Format): string {
                let p = format.precision ?? 2
                return `${self.cents}:${p}:${format.alternate}`
            }
        }
        fn main() {
            print(std::fmt::format("%v", [Point(x: 1, y: 2)]))
            print(std::fmt::format("%s", [Money(cents: 12345)]))
            print(std::fmt::format("%.1s", [Money(cents: 12345)]))
            print(std::fmt::format("%#s", [Money(cents: 12345)]))
            print(std::fmt::format("%.3s", [[1, 2, 3]]))
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "Point(x: 1, y: 2)",
            "12345:2:false",
            "12345:1:false",
            "12345:2:true",
            "[1,",
        ]
    );
}

#[test]
fn fmt_text_verb_rejects_a_bare_struct_at_run_time() {
    let err = run(
        r#"
        struct Bare { x: int }
        fn main() { print(std::fmt::format("%v", [Bare(x: 1)])) }
        "#,
    )
    .unwrap_err();
    assert!(
        err.message.contains("need `Display` or `Debug` for a struct"),
        "{}",
        err.message
    );
}
