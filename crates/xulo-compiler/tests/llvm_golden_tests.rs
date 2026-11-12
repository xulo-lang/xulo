//! LLVM emitter golden tests.
//!
//! These tests require the `llvm` feature (and an LLVM 15 development
//! installation), so the default `cargo test` stays free of LLVM.
#![cfg(feature = "llvm")]

use std::path::Path;

use xulo_compiler::aot::{AotBackend, BuildProfile, build_executable_with_profile};
use xulo_compiler::{compile_to_ir, llvm};

fn emit(source: &str) -> String {
    let ir = compile_to_ir(source, Path::new("test.xulo")).expect("IR generation failed");
    llvm::emit_text(&ir).expect("LLVM emission failed")
}

#[test]
fn hello_world_declares_main_and_runtime_calls() {
    let ll = emit(r#"fn main() { print("hello") }"#);
    assert!(
        ll.contains("define"),
        "expected function definitions:\n{ll}"
    );
    assert!(ll.contains("@main"), "expected main symbol:\n{ll}");
    assert!(
        ll.contains("@xulo_print") || ll.contains("@xulo_print_value"),
        "expected runtime print call:\n{ll}"
    );
    assert!(
        ll.contains("str_"),
        "expected interned string global:\n{ll}"
    );
}

#[test]
fn closure_entry_uses_nine_i64_signature() {
    let ll = emit(
        r#"
        fn main() {
            let f = fn(x: number): number { x + 1 }
            print(f(2))
        }
        "#,
    );
    // Closure entries are emitted as `(i64 × 9) -> i64`.
    assert!(
        ll.contains("define i64 @closure"),
        "expected closure entry:\n{ll}"
    );
    assert!(
        ll.matches(", i64 %").count() >= 8,
        "expected a 9-i64 closure entry signature:\n{ll}"
    );
}

#[test]
fn extern_import_is_declared() {
    let ll = emit(
        r#"
        extern "C" {
            fn abs(x: i32): i32
        }
        fn main() { print(abs(-3)) }
        "#,
    );
    assert!(ll.contains("declare"), "expected extern declaration:\n{ll}");
    assert!(ll.contains("@abs"), "expected abs import:\n{ll}");
}

#[test]
fn control_flow_and_string_constants_are_emitted() {
    let ll = emit(
        r#"
        fn main() {
            let mut i = 0
            let mut total = 0
            while i < 3 {
                if i > 1 {
                    total = total + i
                }
                i = i + 1
            }
            print("flow")
            print(total)
        }
        "#,
    );
    assert!(
        ll.contains("icmp") && ll.contains("br "),
        "expected compare and branch instructions:\n{ll}"
    );
    assert!(
        ll.contains("c\"flow\\00\""),
        "expected NUL-terminated string global:\n{ll}"
    );
}

#[test]
fn end_to_end_build_and_run() {
    let dir = std::env::temp_dir().join(format!("xulo-llvm-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("main.xulo");
    let exe = dir.join("main");
    std::fs::write(&src, "fn main() {\n print(\"hello llvm\")\n}\n").unwrap();

    let output = build_executable_with_profile(&src, &exe, AotBackend::Llvm, BuildProfile::Debug)
        .expect("LLVM build failed");
    assert_eq!(output.backend, AotBackend::Llvm);

    let run = std::process::Command::new(&output.executable)
        .output()
        .expect("failed to run built executable");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(run.status.success(), "process failed: {run:?}");
    assert!(
        stdout.contains("hello llvm"),
        "unexpected stdout: {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
