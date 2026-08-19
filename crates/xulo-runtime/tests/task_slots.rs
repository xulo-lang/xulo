//! Async task-slot memory regression tests for the native interpreter.
//!
//! An `await` on an async `fn` spawns one coroutine (1 MiB stack) stored in
//! `Interpreter::tasks`. Design-review finding P2: the vector only ever grew —
//! completed tasks left `None` holes that were never reused, so a program
//! churning many short-lived async calls accumulated a slot (and a stale
//! yielder pointer) per call. These tests pin the recycling behaviour.

use xulo_lexer::tokenize;
use xulo_parser::parse_program;
use xulo_runtime::interpreter::Interpreter;

/// Run a program on a held interpreter (lex + parse only, like `run_raw`) and
/// return the printed lines plus the task-slot count afterwards.
fn run_raw(src: &str) -> (Vec<String>, usize) {
    let interp = Interpreter::new();
    let tokens = tokenize(src).unwrap();
    let program = parse_program(&tokens).unwrap();
    let out = interp.run(&program).unwrap();
    let slots = interp.debug_task_slot_count();
    (out, slots)
}

/// Sequential awaits must recycle the slot of each completed call instead of
/// growing the vector by one every iteration. 200 iterations with reuse keep
/// the slot vector to a handful of entries (the async `main` plus one in-flight
/// `work` call); without recycling it would hold 201.
#[test]
fn sequential_async_calls_recycle_task_slots() {
    let (out, slots) = run_raw(
        r#"
        fn work(n: number): async number { n }
        fn main(): async {
            for i in 0..<200 {
                print(str(await work(i)))
            }
        }
        "#,
    );
    assert_eq!(out.len(), 200, "every await still prints in order");
    assert_eq!(
        out.first().map(String::as_str),
        Some("0"),
        "first iteration ran"
    );
    assert_eq!(
        out.last().map(String::as_str),
        Some("199"),
        "last iteration ran"
    );
    assert!(
        slots <= 4,
        "task slots grew to {slots} instead of recycling (P2 regression)"
    );
}

/// Fire-and-forget calls also recycle: each `work()` spawns and completes a
/// task, so a burst must not leave one slot behind per call.
#[test]
fn fire_and_forget_calls_recycle_task_slots() {
    let (out, slots) = run_raw(
        r#"
        fn work(n: number): async { print(str(n)) }
        fn main(): async {
            for i in 0..<100 {
                work(i)
            }
        }
        "#,
    );
    assert_eq!(out.len(), 100, "every fire-and-forget call ran");
    assert!(
        slots <= 4,
        "task slots grew to {slots} instead of recycling (P2 regression)"
    );
}
