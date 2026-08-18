//! Loop-closure capture regression tests.
//!
//! `for` re-creates a fresh iteration environment every round (`exec_for`), so
//! an `fn` declared in the body captures *that round's* loop variable — JS
//! `let` semantics (`funcs[0]()/.../funcs[2]()` read 0/1/2), not a shared
//! `var` binding (which would read 3 from every closure). Pins the semantic
//! decision that defers P3: recycling the iteration `Env` would silently turn
//! this into the `var i` trap.

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