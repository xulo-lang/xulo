use std::path::Path;
use xulo_compiler::aot::AotCodeGen;
use xulo_compiler::compile_to_ir;

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
fn panic_call_reaches_machine_code() {
    let bytes = object_bytes(r#"fn main() { panic("boom") }"#);
    assert!(
        references_symbol(&bytes, "xulo_panic"),
        "object must reference xulo_panic"
    );
}

#[test]
fn panic_after_print_reaches_machine_code() {
    let bytes = object_bytes(r#"fn main() { print("a"); panic("boom") }"#);
    assert!(
        references_symbol(&bytes, "xulo_panic"),
        "panic behind another statement must not be dropped"
    );
}
