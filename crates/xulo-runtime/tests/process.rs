//! End-to-end `Process` namespace through the full pipeline (lex → parse →
//! semantic → native interpreter): `args` shape, `run` capture semantics,
//! the optional input argument, and the interpreter's arity/type guards.
//! `exit` is deliberately absent — it would take the test harness with it;
//! the CLI's `process_exit` test covers it in a child process.

#![cfg(unix)]

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
fn args_is_a_non_empty_string_list() {
    let out = run_ok(
        r#"
        fn main() {
            let argv = std::process::args()
            print(argv[0] != "")
            let mut n = 0
            for a in argv {
                n = n + 1
            }
            print(n > 0)
        }
        "#,
    );
    assert_eq!(out, vec!["true".to_string(), "true".to_string()]);
}

#[test]
fn run_captures_streams_and_codes_as_an_object_payload() {
    let out = run_ok(
        r#"
        fn main() {
            match std::process::run("echo hi; echo oops 1>&2; exit 4") {
                Result::Ok(r) => {
                    print(r.stdout)
                    print(r.stderr)
                    print(r.code)
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(
        out,
        vec!["hi\n".to_string(), "oops\n".to_string(), "4".to_string(),]
    );
}

#[test]
fn run_pipes_the_optional_input_argument() {
    let out = run_ok(
        r#"
        fn main() {
            match std::process::run("cat", "pong") {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["pong".to_string()]);
}

#[test]
fn glob_import_binds_run_and_args_bare() {
    let out = run_ok(
        r#"
        use std::process::*;

        fn main() {
            print(args()[0] != "")
            match run("printf '%s' ok") {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["true".to_string(), "ok".to_string()]);
}

#[test]
fn run_arity_is_one_to_three_arguments() {
    // Zero arguments: rejected with the fixed-range message.
    let zero = run(r#"
        fn main() {
            std::process::run()
        }
        "#)
    .expect_err("run() has no command");
    assert!(
        zero.message.contains("expected 1 to 3 arguments, found 0"),
        "{}",
        zero.message
    );

    // Four: the same bound rejects it.
    let four = run(r#"
        fn main() {
            std::process::run("echo", "a", "b", "c")
        }
        "#)
    .expect_err("run() takes at most input and timeout");
    assert!(
        four.message.contains("expected 1 to 3 arguments, found 4"),
        "{}",
        four.message
    );

    // Three arguments pass the arity gate — the third is the timeout
    // budget, so a string in its place is a type error (`E0202`), not an
    // arity error.
    let three = run(r#"
        fn main() {
            std::process::run("echo", "a", "b")
        }
        "#)
    .expect_err("the timeout must be numeric");
    assert!(three.message.contains("must be `u64`"), "{}", three.message);

    // One and two both pass the semantic gate (the one-arg form is covered
    // by the qualified-call test above; two here).
    let out = run_ok(
        r#"
        fn main() {
            match std::process::run("cat", "x") {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["x".to_string()]);
}

#[test]
fn run_argument_types_are_enforced() {
    let err = run(r#"
        fn main() {
            std::process::run(42)
        }
        "#)
    .expect_err("cmd must be a string");
    assert!(err.message.contains("must be `string`"), "{}", err.message);
}

#[test]
fn run_timeout_budget_kills_the_child_and_reports_the_timeout() {
    // A budget that expires kills the child and fails in the timeout class;
    // a child that finishes first returns Ok however tight the budget.
    let out = run_ok(
        r#"
        fn main() {
            match std::process::run("sleep 10", null, 300) {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
            match std::process::run("echo edge", null, 60000) {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out.len(), 2, "{out:?}");
    assert!(out[0].contains("timed out after 300 ms"), "{}", out[0]);
    assert_eq!(out[1], "edge\n");
}

#[test]
fn exec_runs_programs_directly_without_a_shell() {
    // `echo` as a program takes its argument literally — no arithmetic, no
    // pipes: the shell never sees the command. An unknown program is a
    // clean Err in the start class.
    let out = run_ok(
        r#"
        fn main() {
            match std::process::exec("echo", ["$((2 + 3))"]) {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
            match std::process::exec("cat", [], "payload") {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
            match std::process::exec("/nonexistent/xulo-probe", []) {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out[0], "$((2 + 3))\n");
    assert_eq!(out[1], "payload");
    assert!(out[2].contains("cannot run"), "{}", out[2]);
}

#[test]
fn launch_keeps_the_child_handle_and_wait_captures() {
    // `wait` is `run` delayed: the same capture record, and the handle is
    // spent after it answers (Ok or Err alike) while `pid` keeps answering.
    let out = run_ok(
        r#"
        fn main() {
            match std::process::launch("printf captured") {
                Result::Ok(child) => {
                    print(str(child.pid() > 0))
                    match child.wait() {
                        Result::Ok(r) => {
                            print(r.stdout)
                            print(r.code)
                        }
                        Result::Err(e) => print(e.message)
                    }
                    print(str(child.pid() > 0))
                    match child.wait() {
                        Result::Ok(r) => print(r.stdout)
                        Result::Err(e) => print(e.message)
                    }
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out.len(), 5, "{out:?}");
    assert_eq!(out[0], "true");
    assert_eq!(out[1], "captured");
    assert_eq!(out[2], "0");
    assert_eq!(out[3], "true");
    assert!(out[4].contains("spent"), "{}", out[4]);
}

#[test]
fn kill_stops_the_child_and_its_budget_observes_the_deadline() {
    let out = run_ok(
        r#"
        fn main() {
            match std::process::launch("sleep 10") {
                Result::Ok(child) => {
                    match child.kill() {
                        Result::Ok(u) => print("killed")
                        Result::Err(e) => print(e.message)
                    }
                    match child.wait() {
                        Result::Ok(r) => print(r.code)
                        Result::Err(e) => print(e.message)
                    }
                }
                Result::Err(e) => print(e.message)
            }
            match std::process::launch("sleep 10", null, 300) {
                Result::Ok(child) => {
                    match child.wait() {
                        Result::Ok(r) => print(r.stdout)
                        Result::Err(e) => print(e.message)
                    }
                }
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out.len(), 3, "{out:?}");
    assert_eq!(out[0], "killed");
    assert_eq!(out[1], "-1", "SIGKILL has no exit code");
    assert!(out[2].contains("timed out after 300 ms"), "{}", out[2]);
}

#[test]
fn command_spec_setters_are_pure_and_overlay_the_environment() {
    let out = run_ok(
        r#"
        use std::process::Command;
        fn main() {
            let base = Command::shell("printf '%s' \"${XULO_STD_PROCESS_PROBE:-}\"")
            match base.run() {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
            match base.env("XULO_STD_PROCESS_PROBE", "yes").run() {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
            match base.run() {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["", "yes", ""], "{out:?}");
}

#[test]
fn command_direct_launch_and_timeout_reach_the_child() {
    let out = run_ok(
        r#"
        use std::process::Command;
        fn main() {
            match Command::direct("printf", ["%s-%s", "a", "b"]).run() {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
            match Command::direct("sleep", ["10"]).timeout(300).launch() {
                Result::Ok(child) => {
                    match child.wait() {
                        Result::Ok(r) => print(r.stdout)
                        Result::Err(e) => print(e.message)
                    }
                }
                Result::Err(e) => print(e.message)
            }
            match Command::shell("printf via-cwd").cwd(".").run() {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out.len(), 3, "{out:?}");
    assert_eq!(out[0], "a-b");
    assert!(out[1].contains("timed out after 300 ms"), "{}", out[1]);
    assert_eq!(out[2], "via-cwd");
}

#[test]
fn glob_and_alias_imports_reach_the_process_surface() {
    let out = run_ok(
        r#"
        use std::process::*;
        fn main() {
            match run("printf glob") {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
            match Command::shell("printf via-glob").run() {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(
        out,
        vec!["glob".to_string(), "via-glob".to_string()],
        "{out:?}"
    );
    let aliased = run_ok(
        r#"
        use std::process as proc;
        fn main() {
            match proc::Command::shell("printf alias").run() {
                Result::Ok(r) => print(r.stdout)
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(aliased, vec!["alias".to_string()], "{aliased:?}");
}

/// `examples/process.xulo` 全文经解释器运行的输出逐行固定（示例即测试）。
#[test]
fn process_example() {
    let out = run_ok(include_str!("../../../examples/process.xulo"));
    assert_eq!(
        out,
        vec![
            "true".to_string(),
            "true".to_string(),
            "hi".to_string(),
            "bad".to_string(),
            "3".to_string(),
            "ping".to_string(),
            "$((2 + 3))".to_string(),
            "true".to_string(),
            "captured".to_string(),
            "`printf captured` child handle is spent".to_string(),
            "true".to_string(),
            "yes".to_string(),
            "`sleep` timed out after 300 ms".to_string(),
            "killed".to_string(),
            "-1".to_string(),
        ]
    );
}
