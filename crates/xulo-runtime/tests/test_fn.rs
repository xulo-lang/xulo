use xulo_core::ast::{FnDef, Statement};
use xulo_parser::parse_program;
use xulo_runtime::interpreter::Interpreter;

/// Full pipeline, then run every `#Test fn` in the program. Returns the
/// per-test outcomes `(name, error message)`; a `None` error means pass.
fn run_all_tests(src: &str) -> Vec<(String, Option<String>)> {
    let program = parse_program(src).expect("parse failed");
    xulo_semantic::analyze(&program).expect("semantic check failed");
    let tests: Vec<&FnDef> = program
        .statements
        .iter()
        .filter_map(|s| match s {
            Statement::Test(f) => Some(f),
            _ => None,
        })
        .collect();
    assert!(!tests.is_empty(), "no `#Test fn` found in source");
    tests
        .into_iter()
        .map(|test| {
            let interp = Interpreter::new();
            let result = interp.run_test(&program, &[], test);
            let outcome = match result {
                Ok(_) => None,
                Err(e) => Some(e.message),
            };
            (test.name.clone(), outcome)
        })
        .collect()
}

#[test]
fn passing_test_completes() {
    let outcomes = run_all_tests(
        r#"
        fn add(a: number, b: number): number { return a + b }
        #Test
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
        #Test
        fn fails() {
            assert(1 == 2, "one is not two")
        }
        "#,
    );
    let (name, err) = &outcomes[0];
    assert_eq!(name, "fails");
    assert_eq!(err.as_deref(), Some("uncaught exception: one is not two"));
}

#[test]
fn panic_fails_the_test() {
    let outcomes = run_all_tests(
        r#"
        #Test
        fn panics() {
            panic("boom")
        }
        "#,
    );
    assert_eq!(outcomes[0].1.as_deref(), Some("uncaught exception: boom"));
}

#[test]
fn tests_do_not_run_under_interpreter_run() {
    // The test body is inert: `Interpreter::run` must not execute it, and the
    // test identifier binds no value.
    let program = parse_program(
        r#"
        #Test
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
        #Test
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
        #Test
        async fn async_fails() {
            let v = await one()
            assert(v == 2, "one is not two")
        }
        "#,
    );
    let (name, err) = &outcomes[0];
    assert_eq!(name, "async_fails");
    assert_eq!(err.as_deref(), Some("uncaught exception: one is not two"));
}

#[test]
fn test_body_sees_module_declarations() {
    let outcomes = run_all_tests(
        r#"
        const LIMIT = 10
        fn helper(): number { return LIMIT * 2 }
        #Test
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
        #Test
        fn first() {
            counter = counter + 1
            assert(counter == 1, "first sees 1")
        }
        #Test
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
