use xulo_core::ast::{Statement, TestDef, TestMode};
use xulo_parser::parse_program;
use xulo_runtime::interpreter::{Interpreter, TestOutcome};

/// Whether a run outcome satisfies a test's declared expectation (spec
/// testing.md, "Running").
fn passes(test: &TestDef, outcome: &TestOutcome) -> bool {
    match test.mode() {
        TestMode::Normal => matches!(outcome, TestOutcome::Completed),
        TestMode::ShouldFail => !matches!(outcome, TestOutcome::Completed),
        TestMode::ShouldPanic => match outcome {
            TestOutcome::Panicked { message } => match test.expected_message() {
                Some(expected) => message.contains(expected),
                None => true,
            },
            _ => false,
        },
    }
}

/// Full pipeline, then run every `#[test] fn` in the program. Returns the
/// per-test outcomes `(name, error message)`; a `None` error means pass.
fn run_all_tests(src: &str) -> Vec<(String, Option<String>)> {
    let program = parse_program(src).expect("parse failed");
    xulo_semantic::analyze(&program).expect("semantic check failed");
    let tests: Vec<&TestDef> = program
        .statements
        .iter()
        .filter_map(|s| match s {
            Statement::Test(t) => Some(t),
            _ => None,
        })
        .collect();
    assert!(!tests.is_empty(), "no `#[test] fn` found in source");
    tests
        .into_iter()
        .map(|test| {
            if test.is_skipped() {
                return (test.func.name.clone(), None);
            }
            let interp = Interpreter::new();
            let run = interp.run_test(&program, &[], test);
            let outcome = if passes(test, &run.outcome) {
                None
            } else {
                Some(match &run.outcome {
                    TestOutcome::Panicked { message } => message.clone(),
                    TestOutcome::TimedOut => "timed out".to_string(),
                    TestOutcome::Cancelled => "cancelled".to_string(),
                    TestOutcome::Completed => "completed unexpectedly".to_string(),
                })
            };
            (test.func.name.clone(), outcome)
        })
        .collect()
}

#[test]
fn passing_test_completes() {
    let outcomes = run_all_tests(
        r#"
        fn add(a: number, b: number): number { return a + b }
        #[test]
        fn addition_works() {
            assert(add(2, 3) == 5, "2+3=5")
        }
        "#,
    );
    assert_eq!(outcomes, vec![("addition_works".to_string(), None)]);
}

#[test]
fn failed_assert_surfaces_as_error() {
    let outcomes = run_all_tests(
        r#"
        #[test]
        fn fails() {
            assert(1 == 2, "one is not two")
        }
        "#,
    );
    let (name, err) = &outcomes[0];
    assert_eq!(name, "fails");
    assert_eq!(err.as_deref(), Some("one is not two"));
}

#[test]
fn panic_fails_the_test() {
    let outcomes = run_all_tests(
        r#"
        #[test]
        fn panics() {
            panic("boom")
        }
        "#,
    );
    assert_eq!(outcomes[0].1.as_deref(), Some("boom"));
}

#[test]
fn tests_do_not_run_under_interpreter_run() {
    // The test body is inert: `Interpreter::run` must not execute it, and the
    // test identifier binds no value.
    let program = parse_program(
        r#"
        #[test]
        fn never_runs() {
            print("SHOULD NOT PRINT")
        }
        fn main() {
            print("main ran")
        }
        "#,
    )
    .unwrap();
    xulo_semantic::analyze(&program).unwrap();
    let interp = Interpreter::new();
    let out = interp.run(&program).unwrap();
    assert_eq!(out, vec!["main ran".to_string()]);
}

#[test]
fn async_test_passes_when_task_settles() {
    let outcomes = run_all_tests(
        r#"
        async fn double(x: number): number { return x * 2 }
        #[test]
        async fn async_works() {
            let v = await double(21)
            assert(v == 42, "answer")
        }
        "#,
    );
    assert_eq!(outcomes, vec![("async_works".to_string(), None)]);
}

#[test]
fn async_test_fails_when_body_asserts() {
    let outcomes = run_all_tests(
        r#"
        async fn one(): number { return 1 }
        #[test]
        async fn async_fails() {
            let v = await one()
            assert(v == 2, "one is not two")
        }
        "#,
    );
    let (name, err) = &outcomes[0];
    assert_eq!(name, "async_fails");
    assert_eq!(err.as_deref(), Some("one is not two"));
}

#[test]
fn test_body_sees_module_declarations() {
    let outcomes = run_all_tests(
        r#"
        const LIMIT = 10
        fn helper(): number { return LIMIT * 2 }
        #[test]
        fn sees_module() {
            assert(helper() == 20, "helper")
        }
        "#,
    );
    assert_eq!(outcomes, vec![("sees_module".to_string(), None)]);
}

#[test]
fn each_test_gets_a_fresh_interpreter() {
    // Two tests both increment a module-level `let` back to 1: with shared
    // state the second would see 2. The runner creates a fresh interpreter
    // per test, so both pass.
    let outcomes = run_all_tests(
        r#"
        let mut counter = 0
        #[test]
        fn first() {
            counter = counter + 1
            assert(counter == 1, "first sees 1")
        }
        #[test]
        fn second() {
            counter = counter + 1
            assert(counter == 1, "second sees 1")
        }
        "#,
    );
    assert_eq!(
        outcomes,
        vec![("first".to_string(), None), ("second".to_string(), None),]
    );
}

#[test]
fn assert_intrinsic_fails_from_plain_run() {
    // `assert` works outside tests too (spec: the fourth intrinsic).
    let program = parse_program(
        r#"
        fn main() {
            assert(false, "assertion failed at runtime")
        }
        "#,
    )
    .unwrap();
    xulo_semantic::analyze(&program).unwrap();
    let interp = Interpreter::new();
    let err = interp.run(&program).unwrap_err();
    assert_eq!(
        err.message,
        "uncaught exception: assertion failed at runtime"
    );
}

#[test]
fn assert_true_is_a_no_op() {
    let program = parse_program(
        r#"
        fn main() {
            assert(true, "holds")
            print("after assert")
        }
        "#,
    )
    .unwrap();
    xulo_semantic::analyze(&program).unwrap();
    let interp = Interpreter::new();
    let out = interp.run(&program).unwrap();
    assert_eq!(out, vec!["after assert".to_string()]);
}

#[test]
fn should_fail_passes_on_panic_and_assertion() {
    let outcomes = run_all_tests(
        r#"
        #[test(should_fail)]
        fn panics() { panic("boom") }
        #[test(should_fail)]
        fn asserts() { assert(false, "nope") }
        "#,
    );
    assert_eq!(
        outcomes,
        vec![("panics".to_string(), None), ("asserts".to_string(), None),]
    );
}

#[test]
fn should_fail_fails_when_body_completes() {
    let outcomes = run_all_tests(
        r#"
        #[test(should_fail)]
        fn completes() { assert(true, "ok") }
        "#,
    );
    assert_eq!(outcomes[0].1.as_deref(), Some("completed unexpectedly"));
}

#[test]
fn should_panic_passes_only_on_panic() {
    let outcomes = run_all_tests(
        r#"
        #[test(should_panic)]
        fn panics() { panic("boom") }
        #[test(should_panic)]
        fn completes() { }
        "#,
    );
    assert_eq!(outcomes[0].1, None);
    assert_eq!(outcomes[1].1.as_deref(), Some("completed unexpectedly"));
}

#[test]
fn should_panic_message_is_a_case_sensitive_substring() {
    let outcomes = run_all_tests(
        r#"
        #[test(should_panic = "division by zero")]
        fn matches() { panic("error: division by zero here") }
        #[test(should_panic = "Division By Zero")]
        fn wrong_case() { panic("error: division by zero here") }
        "#,
    );
    assert_eq!(outcomes[0].1, None);
    assert!(outcomes[1].1.is_some(), "case must match exactly");
}

#[test]
fn should_panic_matches_empty_message() {
    let outcomes = run_all_tests(
        r#"
        #[test(should_panic = "")]
        fn any_panic() { panic("anything") }
        "#,
    );
    assert_eq!(outcomes[0].1, None);
}

#[test]
fn skipped_test_is_not_executed() {
    let outcomes = run_all_tests(
        r#"
        #[test(skip)]
        fn skipped() { panic("must not run") }
        "#,
    );
    assert_eq!(outcomes, vec![("skipped".to_string(), None)]);
}

#[test]
fn timeout_reports_timed_out() {
    let outcomes = run_all_tests(
        r#"
        #[test(timeout = 1)]
        fn too_slow() {
            let mut i = 0
            while i < 100000000 {
                i = i + 1
            }
        }
        "#,
    );
    assert_eq!(outcomes[0].1.as_deref(), Some("timed out"));
}

#[test]
fn fast_test_keeps_its_normal_outcome_under_a_deadline() {
    let outcomes = run_all_tests(
        r#"
        #[test(timeout = 10000)]
        fn quick() { assert(true, "ok") }
        "#,
    );
    assert_eq!(outcomes[0].1, None);
}

#[test]
fn timeout_passes_should_fail_but_not_should_panic() {
    let outcomes = run_all_tests(
        r#"
        #[test(should_fail, timeout = 1)]
        fn fails_by_timeout() {
            let mut i = 0
            while i < 100000000 { i = i + 1 }
        }
        #[test(should_panic, timeout = 1)]
        fn panics_by_timeout() {
            let mut i = 0
            while i < 100000000 { i = i + 1 }
        }
        "#,
    );
    assert_eq!(outcomes[0].1, None);
    assert_eq!(outcomes[1].1.as_deref(), Some("timed out"));
}

#[test]
fn async_panic_under_a_deadline_keeps_the_panic_outcome() {
    // A settled async body must not wait out its deadline: the run is a
    // panic, not a timeout.
    let outcomes = run_all_tests(
        r#"
        async fn explode() { panic("boom") }
        #[test(should_panic = "boom", timeout = 10000)]
        async fn fast_panic() { await explode() }
        "#,
    );
    assert_eq!(outcomes[0].1, None);
}

#[test]
fn async_completion_under_a_deadline_completes() {
    let outcomes = run_all_tests(
        r#"
        async fn value(): number { return 1 }
        #[test(timeout = 10000)]
        async fn quick() { assert(await value() == 1, "one") }
        "#,
    );
    assert_eq!(outcomes[0].1, None);
}
