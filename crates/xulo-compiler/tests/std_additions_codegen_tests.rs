//! The first standard-library expansion batch on the compiled path:
//! `std::math` helpers, `std::path` inspection, `std::io` probes, and
//! `std::json` validation/indentation.

use std::path::Path;
use xulo_compiler::{codegen::CodeGen, compile_to_ir};

fn compile_and_run(source: &str) -> Vec<String> {
    let ir = compile_to_ir(source, Path::new("test.xulo")).expect("IR generation failed");
    let mut codegen = CodeGen::new();
    let code_ptr = codegen.compile(&ir).expect("codegen failed");
    unsafe {
        let func: fn() = std::mem::transmute(code_ptr);
        func();
    }
    xulo_runtime::runtime::xulo_take_output()
}

#[test]
fn math_helpers_run_on_the_compiled_path() {
    let out = compile_and_run(
        r#"
fn main() {
    print(std::math::sign(-3.5))
    print(std::math::hypot(3, 4))
    print(std::math::clamp(12, 0, 10))
    print(std::math::log1p(0))
    print(std::math::expm1(0))
}
"#,
    );
    assert_eq!(out, vec!["-1", "5", "10", "0", "0"]);
}

#[test]
fn path_inspection_runs_on_the_compiled_path() {
    let out = compile_and_run(
        r#"
fn main() {
    print(std::path::isAbsolute("/a"))
    print(std::path::isAbsolute("a"))
    print(std::path::stem("archive.tar.gz"))
    print(std::path::withExt("report.txt", ".md"))
}
"#,
    );
    assert_eq!(out, vec!["true", "false", "archive.tar", "report.md"]);
}

#[test]
fn io_terminal_probes_run_on_the_compiled_path() {
    let out = compile_and_run(
        r#"
fn main() {
    let a = std::io::stdinIsTTY()
    let b = std::io::stdoutIsTTY()
    print(a == a)
    print(b == b)
}
"#,
    );
    assert_eq!(out, vec!["true", "true"]);
}

#[test]
fn json_valid_and_indent_run_on_the_compiled_path() {
    let out = compile_and_run(
        r#"
fn main() {
    print(std::json::valid("{\"a\": 1}"))
    print(std::json::valid("{\"a\": }"))
    match std::json::stringify({ n: 3 }, 2) {
        Result::Ok(t) => print(t)
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out, vec!["true", "false", "{\n  \"n\": 3\n}"]);
}
