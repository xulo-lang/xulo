//! The `Process` namespace semantics in isolation — `args()`, the one-shot
//! `run`/`exec` captures, the kept `launch` handle, and the `Command` spec
//! against the local POSIX shell, no Xulo pipeline involved. `exit` is not
//! called here (it would take the test process with it); its end-to-end
//! behaviour lives in the CLI and codegen exit tests.

#![cfg(unix)]

use std::thread;
use std::time::Duration;

use xulo_std::process::{self, CommandSpec};

#[test]
fn run_captures_stdout_and_reports_a_zero_code() {
    let out = process::run("echo hi", None, None).expect("sh spawn");
    assert_eq!(out.stdout, "hi\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.code, 0);
}

#[test]
fn run_treats_a_failing_command_as_data_not_an_error() {
    let out = process::run("echo bad 1>&2; exit 3", None, None).expect("sh spawn");
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "bad\n");
    assert_eq!(out.code, 3);
}

#[test]
fn run_commands_go_through_the_shell() {
    // Exposed shell features prove `sh -c`, not a bare exec of `cmd`.
    let out = process::run("echo $((2 + 3))", None, None).expect("sh spawn");
    assert_eq!(out.stdout, "5\n");
    let out = process::run("printf '%s-%s' a b", None, None).expect("sh spawn");
    assert_eq!(out.stdout, "a-b");
}

#[test]
fn run_pipes_the_string_input_to_stdin() {
    let out = process::run("cat", Some("ping"), None).expect("sh spawn");
    assert_eq!(out.stdout, "ping");
    assert_eq!(out.code, 0);
    // Input reaches the command, so a filter sees it: `wc -c` counts bytes.
    let out = process::run("wc -c", Some("abcd"), None).expect("sh spawn");
    assert_eq!(out.stdout.trim(), "4");
}

#[test]
fn run_without_input_leaves_stdin_alone() {
    // No input: the child inherits stdin (we never write to it); `true`
    // exits cleanly regardless of what our stdin holds.
    let out = process::run("true", None, None).expect("sh spawn");
    assert_eq!(out.code, 0);
}

#[test]
fn run_reports_a_timeout_when_the_budget_expires() {
    // A child far past its budget is killed and the call fails in the
    // timeout class; a non-zero exit would have been data, so this Err is
    // specifically the budget.
    let err = process::run("sleep 10", None, Some(300)).expect_err("budget should kill the child");
    assert!(err.contains("timed out after 300 ms"), "{err}");
    // The budget is a bound, not an expectation: a child that finishes
    // first returns Ok even when it had a timeout set.
    let out = process::run("echo edge", None, Some(60_000)).expect("finishes early");
    assert_eq!(out.stdout, "edge\n");
}

#[test]
fn oversized_captures_are_an_error_and_do_not_wedge() {
    // ~17 MiB of stdout crosses the 16 MiB cap: the reader kills the child
    // instead of deadlocking on a full pipe, and the caller sees an Err.
    let result = process::run("head -c 17825792 /dev/zero", None, None);
    let err = result.expect_err("cap should reject the capture");
    assert!(err.contains("exceeds"), "{err}");
}

#[test]
fn exec_runs_a_program_directly_without_a_shell() {
    // `echo` as a program takes the first arg literally — no `$((...))`
    // arithmetic, no pipes: the shell never sees the command.
    let args = vec!["$((2 + 3))".to_string()];
    let out = process::exec("echo", &args, None, None).expect("direct spawn");
    assert_eq!(out.stdout, "$((2 + 3))\n");
    // Through the shell the same text is arithmetic (proven above).
    // An unknown program fails in the start class, with the name in the Err.
    let err = process::exec("/nonexistent/xulo-probe", &[], None, None)
        .expect_err("missing program must fail");
    assert!(err.contains("cannot run"), "{err}");
    assert!(err.contains("/nonexistent/xulo-probe"), "{err}");
}

#[test]
fn exec_honors_input_and_timeout() {
    let out = process::exec("cat", &[], Some("payload"), None).expect("direct spawn");
    assert_eq!(out.stdout, "payload");
    let err = process::exec("sleep", &["10".to_string()], None, Some(300))
        .expect_err("budget should kill the child");
    assert!(err.contains("timed out after 300 ms"), "{err}");
}

#[test]
fn launch_returns_a_handle_that_waits_and_answers_pid_total() {
    let mut child = process::launch("printf captured", None, None).expect("sh spawn");
    let pid = child.pid();
    assert!(pid > 0, "pid: {pid}");
    let out = child.wait().expect("wait");
    assert_eq!(out.stdout, "captured");
    assert_eq!(out.code, 0);
    // Spent after one wait — with Ok or Err alike — while pid keeps answering.
    let err = child.wait().expect_err("handle is spent");
    assert!(err.contains("spent"), "{err}");
    assert_eq!(child.pid(), pid);
    let err = child.kill().expect_err("spent handle cannot kill");
    assert!(err.contains("spent"), "{err}");
}

#[test]
fn launch_timeout_budget_is_observed_on_wait() {
    let mut child = process::launch("sleep 10", None, Some(300)).expect("sh spawn");
    let err = child.wait().expect_err("budget kills at wait");
    assert!(err.contains("timed out after 300 ms"), "{err}");
    assert!(child.wait().expect_err("spent").contains("spent"));
}

#[test]
fn kill_stops_a_live_child_and_wait_reports_the_signal_as_minus_one() {
    let mut child = process::launch("sleep 10", None, None).expect("sh spawn");
    child.kill().expect("live child can be killed");
    let out = child.wait().expect("reap after kill");
    assert_eq!(out.code, -1, "SIGKILL has no exit code");
}

#[test]
fn kill_on_an_exited_child_is_an_error_but_wait_still_answers() {
    let mut child = process::launch("true", None, None).expect("sh spawn");
    // Give the child time to exit on its own — killing a corpse is refused.
    thread::sleep(Duration::from_millis(300));
    let err = child.kill().expect_err("already exited");
    assert!(err.contains("already exited"), "{err}");
    let out = child.wait().expect("status was collected by the check");
    assert_eq!(out.code, 0);
}

#[test]
fn a_late_finish_wins_over_a_passed_budget() {
    // The budget is checked when `wait` runs; the child finishing first
    // always reports its record, even after the deadline has elapsed.
    let mut child = process::launch("echo late", None, Some(50)).expect("sh spawn");
    thread::sleep(Duration::from_millis(300));
    let out = child.wait().expect("finished child reports its record");
    assert_eq!(out.stdout, "late\n");
}

#[test]
fn command_spec_setters_are_pure_and_overlay_the_environment() {
    let base = CommandSpec::shell("printf '%s' \"${XULO_STD_PROBE:-}\"");
    let with_env = base.clone().env("XULO_STD_PROBE", "yes");
    let out = with_env.run().expect("overlay reaches the child");
    assert_eq!(out.stdout, "yes");
    // The receiver never changed: re-running `base` sees no overlay.
    let out = base.run().expect("base unchanged");
    assert_eq!(out.stdout, "");
    // Direct spec: argv as given, no shell wordsplitting.
    let direct = CommandSpec::direct("printf", vec!["%s-%s".into(), "a".into(), "b".into()]);
    let out = direct.run().expect("direct spawn");
    assert_eq!(out.stdout, "a-b");
    // cwd failure lands in the start class with the command named.
    let err = CommandSpec::shell("true")
        .cwd("/nonexistent/xulo-cwd-probe")
        .run()
        .expect_err("missing cwd must fail");
    assert!(err.contains("cannot run `true`"), "{err}");
}

#[test]
fn command_spec_launch_and_input_reach_the_child() {
    let mut child = CommandSpec::shell("wc -c")
        .input("abcd")
        .launch()
        .expect("spawn");
    let out = child.wait().expect("wait");
    assert_eq!(out.stdout.trim(), "4");
    // input(null) equivalent: no input set on a fresh spec.
    let mut plain = CommandSpec::direct("true", vec![]).launch().expect("spawn");
    let out = plain.wait().expect("wait");
    assert_eq!(out.code, 0);
}

#[test]
fn args_starts_with_the_program_path_and_is_non_empty() {
    let argv = process::args();
    assert!(!argv.is_empty());
    assert!(!argv[0].is_empty(), "argv[0] is the program path");
    // This test binary's own path is absolute on unix (its file name at
    // least contains a path separator somewhere in argv).
    assert!(
        argv.iter().any(|a| a.contains('/')),
        "no path-like arg: {argv:?}"
    );
}
