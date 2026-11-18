//! P1 regression: aggregate module-level `const`s are materialized once in the
//! AOT path (before `main`), not re-evaluated at every reference.

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
fn aggregate_const_initializer_runs_once_before_main() {
    let out = compile_and_run(
        r#"pub const values: List<number> = build()

fn build(): List<number> {
    print("init")
    return [1, 2]
}

fn main() {
    print("before")
    print(values)
    print(values)
}
"#,
    );
    assert_eq!(out, vec!["init", "before", "[1, 2]", "[1, 2]"]);
}

#[test]
fn aggregate_const_references_share_one_value() {
    let out = compile_and_run(
        r#"pub const values: List<number> = [1, 2]

fn main() {
    let alias = values
    alias.push(3)
    print(values)
}
"#,
    );
    assert_eq!(out, vec!["[1, 2, 3]"]);
}
