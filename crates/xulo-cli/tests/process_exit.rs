//! `std::process::exit` end-to-end through the CLI in a child process —
//! the only honest way to test termination: the script's buffered `print`
//! lines must reach stdout *before* the process dies with the requested
//! code (the interpreter drains them exactly as a normal end would).

use std::io::Write;
use std::path::PathBuf;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_xulo");

fn temp_file(name: &str, content: &str) -> PathBuf {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "xulo_exit_{}_{}_{name}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut f = std::fs::File::create(&path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
    path
}

#[test]
fn exit_reports_the_requested_code_after_flushing_print_output() {
    let file = temp_file(
        "exit7.xulo",
        r#"
fn main() {
    print("before exit")
    std::process::exit(7)
}
"#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(7),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The buffered prelude `print` line reached stdout before the process
    // died — an exit that swallowed it would print nothing here.
    assert_eq!(String::from_utf8_lossy(&out.stdout), "before exit\n");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn exit_interleaves_immediate_and_buffered_output_in_source_order() {
    let file = temp_file(
        "exit5.xulo",
        r#"
fn main() {
    std::io::println("io line")
    print("prelude line")
    std::process::exit(5)
}
"#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(5));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "io line\nprelude line\n"
    );
    let _ = std::fs::remove_file(&file);
}

#[test]
fn exit_zero_succeeds_with_no_output() {
    let file = temp_file("exit0.xulo", "fn main() { std::process::exit(0) }");
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    let _ = std::fs::remove_file(&file);
}

#[test]
fn run_members_reach_the_cli_pipeline_alongside_exit() {
    // A script that both captures a child and exits: guards the full path
    // (semantic `Result` object payload → interpreter → exit code).
    let file = temp_file(
        "run_then_exit.xulo",
        r#"
fn main() {
    match std::process::run("printf '%s' captured") {
        Result::Ok(r) => print(r.stdout)
        Result::Err(e) => print(e.message)
    }
    std::process::exit(2)
}
"#,
    );
    let out = Command::new(BIN).arg("run").arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "captured\n");
    let _ = std::fs::remove_file(&file);
}
