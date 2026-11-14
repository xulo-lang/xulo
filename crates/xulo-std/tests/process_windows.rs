//! Windows shell integration for `std::process`: the `run` form goes through
//! `cmd /C`, so Windows-specific syntax and exit codes must behave like the
//! Unix `sh -c` tests (which are gated to Unix).

#![cfg(windows)]

use xulo_std::process;

#[test]
fn run_uses_cmd_c_and_captures_stdout() {
    let out = process::run("echo hi", None, None).expect("cmd spawn");
    assert_eq!(out.stdout.trim(), "hi");
    assert_eq!(out.stderr, "");
    assert_eq!(out.code, 0);
}

#[test]
fn run_reports_a_nonzero_exit_as_data() {
    let out = process::run("exit /b 3", None, None).expect("cmd spawn");
    assert_eq!(out.code, 3);
}

#[test]
fn run_exposes_cmd_shell_features() {
    let out = process::run("echo one && echo two", None, None).expect("cmd spawn");
    assert_eq!(out.stdout, "one\r\ntwo\r\n");
}
