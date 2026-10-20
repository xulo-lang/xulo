//! End-to-end `RegExp` through the full pipeline (lex → parse → semantic →
//! native interpreter): compilation into `Result`, the `RegexMatch` read
//! model behind a null test, string `replace`/`to_uppercase`, `?`
//! propagation out of sync and async functions, and narrowing an optional
//! binding with `x != null`.

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
fn compile_test_honours_the_case_insensitive_flag() {
    let out = run_ok(
        r#"
        use regex::{RegExp, RegexError};
        fn main() {
            match RegExp::compile("[a-z]+", "i") {
                Result::Ok(re) => {
                    print(str(re.test("ABC123")))
                    print(str(re.test("123")))
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["true".to_string(), "false".to_string()]);
}

#[test]
fn compile_reports_a_bad_pattern_as_error_data() {
    let out = run_ok(
        r#"
        use regex::{RegExp, RegexError};
        fn main() {
            match RegExp::compile("[a-z") {
                Result::Ok(_) => print("compiled")
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out.len(), 1);
    assert!(
        out[0].contains("unclosed character class"),
        "unexpected message: {}",
        out[0]
    );
}

#[test]
fn compile_rejects_an_unknown_flag() {
    let out = run_ok(
        r#"
        use regex::{RegExp, RegexError};
        fn main() {
            match RegExp::compile("a", "z") {
                Result::Ok(_) => print("compiled")
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["unknown flag `z`".to_string()]);
}

#[test]
fn find_read_value_and_groups_behind_a_null_test() {
    let out = run_ok(
        r#"
        use regex::{RegExp, RegexError};
        fn main() {
            match RegExp::compile("(\\d+)-(\\d+)") {
                Result::Ok(re) => {
                    let m = re.find("room 42-7")
                    if m != null {
                        print(m.value)
                        print(str(m.group(1)))
                        print(str(m.group(2)))
                    } else {
                        print("miss")
                    }
                    if re.find("nothing") != null {
                        print("found")
                    } else {
                        print("miss")
                    }
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "42-7".to_string(),
            "42".to_string(),
            "7".to_string(),
            "miss".to_string()
        ]
    );
}

#[test]
fn find_all_iterates_every_match() {
    let out = run_ok(
        r#"
        use regex::{RegExp, RegexError};
        fn main() {
            match RegExp::compile("[a-c]") {
                Result::Ok(re) => {
                    for m in re.find_all("a b c") {
                        print(m.value)
                    }
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
}

#[test]
fn replace_expands_capture_groups_in_the_template() {
    let out = run_ok(
        r#"
        use regex::{RegExp, RegexError};
        fn main() {
            match RegExp::compile("(a)(b)") {
                Result::Ok(re) => print("ab".replace(re, "$2-$1"))
                Result::Err(e) => print(e.message)
            }
            match RegExp::compile("(a)(b)", "g") {
                Result::Ok(re) => print("ab ab".replace(re, "$2$1"))
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    // Without `g` only the first match is substituted; with it, every match.
    assert_eq!(out, vec!["b-a".to_string(), "ba ba".to_string()]);
}

#[test]
fn replace_accepts_a_function_replacer() {
    let out = run_ok(
        r#"
        use regex::{RegExp, RegexError};
        fn main() {
            match RegExp::compile("(cat|dog)") {
                Result::Ok(re) => {
                    let out = "cat and dog".replace(re, fn(m) { "[" + m.value + "]" })
                    print(out)
                }
                Result::Err(e) => print(e.message)
            }
            match RegExp::compile("(cat|dog)", "g") {
                Result::Ok(re) => {
                    let out = "cat and dog".replace(re, fn(m) { "[" + m.value + "]" })
                    print(out)
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(
        out,
        vec!["[cat] and dog".to_string(), "[cat] and [dog]".to_string()]
    );
}

#[test]
fn string_to_uppercase_runs_on_a_receiver() {
    let out = run_ok(
        r#"
        fn main() {
            print("abc".to_uppercase())
        }
        "#,
    );
    assert_eq!(out, vec!["ABC".to_string()]);
}

#[test]
fn question_mark_returns_the_compile_error_to_the_caller() {
    let out = run_ok(
        r#"
        use regex::{RegExp, RegexError};
        fn compile_any(pattern: string): Result<RegExp, RegexError> {
            let re = RegExp::compile(pattern)?
            Result::Ok(re)
        }
        fn main() {
            match compile_any("[a-z]+") {
                Result::Ok(re) => print(str(re.test("az")))
                Result::Err(e) => print(e.message)
            }
            match compile_any("[a-z") {
                Result::Ok(_) => print("compiled")
                Result::Err(e) => print("failed")
            }
        }
        "#,
    );
    assert_eq!(out, vec!["true".to_string(), "failed".to_string()]);
}

#[test]
fn question_mark_inside_an_async_function_crosses_the_spawn_boundary() {
    // The propagate signal thrown by `?` must be caught by the async call's
    // own frame (the spawned task), not escape as a stray top-level error.
    let out = run_ok(
        r#"
        use regex::{RegExp, RegexError};
        async fn compile_any(pattern: string): Result<RegExp, RegexError> {
            let re = RegExp::compile(pattern)?
            Result::Ok(re)
        }
        async fn main() {
            let good = await compile_any("[a-z]+")
            match good {
                Result::Ok(re) => print(str(re.test("az")))
                Result::Err(e) => print(e.message)
            }
            let bad = await compile_any("[a-z")
            match bad {
                Result::Ok(_) => print("compiled")
                Result::Err(e) => print("failed")
            }
        }
        "#,
    );
    assert_eq!(out, vec!["true".to_string(), "failed".to_string()]);
}

#[test]
fn null_test_narrows_an_optional_struct_end_to_end() {
    let out = run_ok(
        r#"
        struct User { email: string }
        fn show(u: User?): string {
            if u != null {
                u.email
            } else {
                "none"
            }
        }
        fn main() {
            let u: User? = { email: "ada@example.com" }
            print(show(u))
            print(show(null))
        }
        "#,
    );
    assert_eq!(out, vec!["ada@example.com".to_string(), "none".to_string()]);
}
