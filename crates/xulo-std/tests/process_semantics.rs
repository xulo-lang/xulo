//! The `Process` namespace semantics in isolation — `args()` and `run`
//! captures against the local POSIX shell, no Xulo pipeline involved.
//! `exit` is not called here (it would take the test process with it); its
//! end-to-end behaviour lives in the CLI and codegen exit tests.

use xulo_std::process;

#[test]
fn run_captures_stdout_and_reports_a_zero_code() {
    let out = process::run("echo hi", None).expect("sh spawn");
    assert_eq!(out.stdout, "hi\n");
    assert_eq!(out.stderr, "");
    assert_eq!(out.code, 0);
}

#[test]
fn run_treats_a_failing_command_as_data_not_an_error() {
    let out = process::run("echo bad 1>&2; exit 3", None).expect("sh spawn");
    assert_eq!(out.stdout, "");
    assert_eq!(out.stderr, "bad\n");
    assert_eq!(out.code, 3);
}

#[test]
fn run_commands_go_through_the_shell() {
    // Exposed shell features prove `sh -c`, not a bare exec of `cmd`.
    let out = process::run("echo $((2 + 3))", None).expect("sh spawn");
    assert_eq!(out.stdout, "5\n");
    let out = process::run("printf '%s-%s' a b", None).expect("sh spawn");
    assert_eq!(out.stdout, "a-b");
}

#[test]
fn run_pipes_the_string_input_to_stdin() {
    let out = process::run("cat", Some("ping")).expect("sh spawn");
    assert_eq!(out.stdout, "ping");
    assert_eq!(out.code, 0);
    // Input reaches the command, so a filter sees it: `wc -c` counts bytes.
    let out = process::run("wc -c", Some("abcd")).expect("sh spawn");
    assert_eq!(out.stdout.trim(), "4");
}

#[test]
fn run_without_input_leaves_stdin_alone() {
    // No input: the child inherits stdin (we never write to it); `true`
    // exits cleanly regardless of what our stdin holds.
    let out = process::run("true", None).expect("sh spawn");
    assert_eq!(out.code, 0);
}

#[test]
fn oversized_captures_are_an_error_and_do_not_wedge() {
    // ~17 MiB of stdout crosses the 16 MiB cap: the reader kills the child
    // instead of deadlocking on a full pipe, and the caller sees an Err.
    let result = process::run("head -c 17825792 /dev/zero", None);
    let err = result.expect_err("cap should reject the capture");
    assert!(err.contains("exceeds"), "{err}");
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
