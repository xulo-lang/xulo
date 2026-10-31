//! Const generic parameters on the compiled path (spec `types/generics.md`):
//! a const argument is a compile-time value, carried as a hidden parameter in
//! this implementation and substituted before the body runs.

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
fn const_generic_functions_run_on_the_compiled_path() {
    let out = compile_and_run(
        r#"
fn count<const N: int>(): int { N }
fn repeat<const N: int>(value: int): int { N * value }
fn outer<const N: int>(): int { count<N>() + 1 }
const M = -1
fn main() {
    print(count<3>())
    print(repeat<2>(5))
    print(outer<4>())
    print(count<M>())
}
"#,
    );
    assert_eq!(out, vec!["3", "10", "5", "-1"]);
}
