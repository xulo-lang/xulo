//! `defer` semantics (spec `statements/return-and-block.md`): registration does
//! not evaluate the operand, and a block's defers run in reverse registration
//! order when control leaves it.

use xulo_parser::parse_program;
use xulo_runtime::interpreter::Interpreter;

fn run_raw(src: &str) -> Vec<String> {
    let interp = Interpreter::new();
    let program = parse_program(src).unwrap();
    interp.run(&program).unwrap()
}

#[test]
fn defers_run_in_reverse_registration_order() {
    let out = run_raw(
        r#"
        fn main() {
            defer print("first")
            defer print("second")
            print("body")
        }
        "#,
    );
    assert_eq!(out, ["body", "second", "first"]);
}

#[test]
fn registration_does_not_evaluate_the_operand() {
    let out = run_raw(
        r#"
        fn main() {
            let mut x = 0
            defer print(x)
            x = 1
            x = 2
        }
        "#,
    );
    assert_eq!(out, ["2"], "the operand reads the block's final state");
}

#[test]
fn defers_run_after_a_return_value_is_determined() {
    let out = run_raw(
        r#"
        fn work(): number {
            defer print("cleanup")
            return 7
        }
        fn main() {
            print(work())
        }
        "#,
    );
    assert_eq!(out, ["cleanup", "7"]);
}

#[test]
fn implicit_return_value_is_determined_before_defers() {
    let out = run_raw(
        r#"
        fn work(): number {
            defer print("cleanup")
            7
        }
        fn main() {
            print(work())
        }
        "#,
    );
    assert_eq!(out, ["cleanup", "7"]);
}

#[test]
fn defers_run_when_break_or_continue_leaves_a_loop_body() {
    let out = run_raw(
        r#"
        fn main() {
            for i in 0..<3 {
                defer print(i)
                if i == 1 { continue }
                print("body")
                if i == 2 { break }
            }
        }
        "#,
    );
    assert_eq!(out, ["body", "0", "1", "body", "2"]);
}

#[test]
fn nested_blocks_run_innermost_defers_first() {
    let out = run_raw(
        r#"
        fn main() {
            defer print("outer")
            if true {
                defer print("inner")
                print("body")
            }
            print("after")
        }
        "#,
    );
    assert_eq!(out, ["body", "inner", "after", "outer"]);
}

#[test]
fn defers_run_before_question_mark_propagation() {
    let out = run_raw(
        r#"
        fn find(found: bool): number? {
            if found { return 5 }
            null
        }
        fn load(): number? {
            defer print("cleanup")
            let value = find(false)?
            print("unreachable")
            value
        }
        fn main() {
            print(load())
        }
        "#,
    );
    assert_eq!(out, ["cleanup", "null"]);
}

#[test]
fn block_operand_runs_at_exit() {
    let out = run_raw(
        r#"
        fn main() {
            defer {
                print("closed")
                print("done")
            }
            print("body")
        }
        "#,
    );
    assert_eq!(out, ["body", "closed", "done"]);
}
