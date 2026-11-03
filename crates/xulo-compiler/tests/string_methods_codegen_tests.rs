//! Built-in `string` member lowering on the compiled path (spec
//! `types/primitive-types.md`): `.len`, `.byte_len`, `.code_at`, `.slice`, and
//! `.bytes()`, plus the AOT runtime symbols they reference.

use std::path::Path;
use xulo_compiler::aot::AotCodeGen;
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

fn object_bytes(source: &str) -> Vec<u8> {
    let ir = compile_to_ir(source, Path::new("test.xulo")).expect("IR generation failed");
    let product = AotCodeGen::new()
        .expect("aot codegen init failed")
        .compile(&ir)
        .expect("aot compile failed");
    let mut buf = Vec::new();
    product.object.write_stream(&mut buf).expect("write failed");
    buf
}

fn references_symbol(bytes: &[u8], symbol: &str) -> bool {
    bytes
        .windows(symbol.len())
        .any(|window| window == symbol.as_bytes())
}

#[test]
fn string_members_run_on_the_compiled_path() {
    let out = compile_and_run(
        r#"
fn main() {
    print("hello".len)
    print("héllo".byte_len)
    print("hel".code_at(0))
    print("hello".slice(1, 4))
    let bs = "hi".bytes()
    print(bs[0])
    print(bs[1])
}
"#,
    );
    assert_eq!(
        out,
        vec![
            "5".to_string(),
            "6".to_string(),
            "104".to_string(),
            "ell".to_string(),
            "104".to_string(),
            "105".to_string(),
        ]
    );
}

#[test]
fn list_len_runs_on_the_compiled_path() {
    let out = compile_and_run(
        r#"
fn main() {
    let xs = [1, 2, 3]
    print(xs.len)
}
"#,
    );
    assert_eq!(out, vec!["3".to_string()]);
}

#[test]
fn aot_object_references_string_member_symbols() {
    let bytes = object_bytes(
        r#"
fn main() {
    "x".len
    "x".byte_len
    "x".code_at(0)
    "x".slice(0, 1)
    "x".bytes()
}
"#,
    );
    for symbol in [
        "xulo_string_size",
        "xulo_string_byte_len",
        "xulo_string_code_at",
        "xulo_string_slice",
        "xulo_string_bytes",
    ] {
        assert!(references_symbol(&bytes, symbol), "object missing {symbol}");
    }
}
