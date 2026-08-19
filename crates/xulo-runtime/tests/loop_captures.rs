//! Loop-closure capture regression tests.
//!
//! `for` normally re-creates a fresh iteration environment every round
//! (`exec_for`), so an `fn` declared in the body captures *that round's* loop
//! variable — JS `let` semantics (`funcs[0]()/.../funcs[2]()` read 0/1/2), not
//! a shared `var` binding (which would read the final value from every
//! closure). When the body contains no `fn` at all, `exec_for` recycles one
//! iteration scope across rounds (P3) — that must never be observable, so the
//! tests below pin both sides: accurate per-round captures when closures exist
//! (anywhere in the body), and correct evaluation when they are recycled.

use xulo_lexer::tokenize;
use xulo_parser::parse_program;
use xulo_runtime::interpreter::Interpreter;

fn run_raw(src: &str) -> Vec<String> {
    let interp = Interpreter::new();
    let tokens = tokenize(src).unwrap();
    let program = parse_program(&tokens).unwrap();
    interp.run(&program).unwrap()
}

/// Range loop (`for i in 0..<n`): each captured `get` sees its own `i`.
#[test]
fn ranged_loop_function_captures_its_own_iteration() {
    let out = run_raw(
        r#"
        let funcs = []
        for i in 0..<3 {
            fn get(): number { i }
            funcs = [...funcs, get]
        }
        fn main() {
            for j in 0..<3 {
                print(funcs[j]())
            }
        }
        "#,
    );
    assert_eq!(
        out,
        ["0", "1", "2"],
        "every closure reads the iteration it was defined in, not the final `i`"
    );
}

/// List iteration (`for x in xs`): same per-iteration capture, matching JS
/// `for...of`/`let`.
#[test]
fn list_loop_function_captures_its_own_iteration() {
    let out = run_raw(
        r#"
        let funcs = []
        for x in [10, 20, 30] {
            fn get(): number { x }
            funcs = [...funcs, get]
        }
        fn main() {
            for j in 0..<3 {
                print(funcs[j]())
            }
        }
        "#,
    );
    assert_eq!(out, ["10", "20", "30"]);
}

/// A closure nested deeper in the body (inside an `if` branch) still captures
/// its own round: the recycled-scope guard must scan the whole subtree.
#[test]
fn closure_in_nested_block_captures_its_own_iteration() {
    let out = run_raw(
        r#"
        let funcs = []
        for i in 0..<3 {
            if i < 3 {
                fn get(): number { i }
                funcs = [...funcs, get]
            }
        }
        fn main() {
            for j in 0..<3 {
                print(funcs[j]())
            }
        }
        "#,
    );
    assert_eq!(out, ["0", "1", "2"]);
}

/// An anonymous `fn` value held in a `let` also forces per-round scopes: the
/// value's closure must read its own iteration count.
#[test]
fn closure_bound_by_let_captures_its_own_iteration() {
    let out = run_raw(
        r#"
        let funcs = []
        for i in 0..<3 {
            let f = fn(): number { i }
            funcs = [...funcs, f]
        }
        fn main() {
            for j in 0..<3 {
                print(funcs[j]())
            }
        }
        "#,
    );
    assert_eq!(out, ["0", "1", "2"]);
}

/// A body with no closures goes through the recycled, reset-per-round scope
/// (P3). It must evaluate exactly like the per-round scope: `let` bindings stay
/// local to their iteration and the loop variable updates every round.
#[test]
fn closure_free_loop_recycles_its_iteration_scope() {
    let out = run_raw(
        r#"
        fn main() {
            let acc = 0
            for i in 0..<3 {
                let step = i + 1
                acc = acc + step
            }
            print(acc)
            let doubled = []
            for x in [1, 2, 3] {
                doubled = [...doubled, x * 2]
            }
            print(doubled)
        }
        "#,
    );
    assert_eq!(out, ["6", "[2, 4, 6]"]);
}
