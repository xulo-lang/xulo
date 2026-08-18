//! Memory-behaviour regression tests for the native interpreter.
//!
//! Pins the reference relationship between an interpreter and its shared root
//! environment. A top-level `fn` is bound into its *defining* environment as a
//! function value that captures that same environment, so a bare
//! `env -> "f" -> FunctionValue { closure: env }` Rc cycle keeps the grand
//! root alive after the interpreter is dropped (known issue D11).

use std::rc::Rc;

use xulo_lexer::tokenize;
use xulo_parser::parse_program;
use xulo_runtime::interpreter::Interpreter;

/// Run a program and drop the interpreter, returning a weak reference to the
/// shared root environment taken beforehand. Callers assert on
/// [`Rc::upgrade`](std::rc::Rc::upgrade) afterwards.
fn root_alive_after_drop(src: &str) -> bool {
    let interp = Interpreter::new();
    let weak = Rc::downgrade(&interp.root_env());
    let tokens = tokenize(src).unwrap();
    let program = parse_program(&tokens).unwrap();
    interp.run(&program).unwrap();
    drop(interp);
    weak.upgrade().is_some()
}

/// Same shape as the D11 leak, minus the cycle: a bare top-level `print`
/// registers no function, so the root env is reclaimed on drop. Guards the
/// `#[ignore]`d D11 test's premise — that a cycle, not interpreter lifetime, is
/// the cause.
#[test]
fn dropping_interpreter_releases_global_env_without_cycle() {
    assert!(!root_alive_after_drop("print(1)"));
}

/// Known D11: registering any top-level `fn` ties its captured closure env
/// back into the env that binds it, so the root env survives the drop. Fine
/// for the CLI (process exit reclaims it), a leak in a long-running
/// REPL/library host.
///
/// `#[ignore]`d: it asserts the *fixed* behavior (nothing left alive), so it
/// fails while the cycle exists. Remove the `#[ignore]` once the cycle is
/// broken (e.g. via `Weak<RefCell<Env>>`).
#[test]
#[ignore]
fn dropping_interpreter_releases_global_env() {
    assert!(
        !root_alive_after_drop("fn a(): number { return 1 }"),
        "global env still alive after interpreter dropped (Rc cycle: fn value \
         captures its defining env, which binds the fn)"
    );
}