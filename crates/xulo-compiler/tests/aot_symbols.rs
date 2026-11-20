//! The linker symbols the native backends emit for user functions.
//!
//! The object is linked against libc and Winsock, so a user `fn read(…)` or
//! `fn listen(…)` must not be exported under that bare name: the linker would
//! report `LNK2005: listen already defined in <obj>`. Only the entry point
//! (`main`) and `pub extern "C" fn` exports keep their declared name.

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

fn has_symbol(bytes: &[u8], symbol: &str) -> bool {
    bytes
        .windows(symbol.len())
        .any(|window| window == symbol.as_bytes())
}

/// A function named after a libc/Winsock call is namespaced, while the entry
/// point keeps the `main` the C runtime calls.
#[test]
fn user_functions_are_namespaced_and_the_entry_point_stays_main() {
    let bytes = object_bytes(
        r#"
fn read(path: string): string {
    return path
}

fn listen(port: number): number {
    return read("x").len + port
}

fn main() {
    print(listen(1))
}
"#,
    );
    assert!(has_symbol(&bytes, "xulo_fn_read"));
    assert!(has_symbol(&bytes, "xulo_fn_listen"));
    assert!(has_symbol(&bytes, "main"));
}

/// A `pub extern "C" fn` is the host's entry into the object, so it keeps the
/// declared name instead of the internal prefix.
#[test]
fn pub_extern_export_keeps_its_declared_name() {
    let bytes = object_bytes(
        r#"
pub extern "C" fn triple(n: i32): i32 {
    return n * 3
}

fn main() {
    print(triple(2))
}
"#,
    );
    assert!(has_symbol(&bytes, "triple"));
    assert!(
        !has_symbol(&bytes, "xulo_fn_triple"),
        "an extern export must not be namespaced"
    );
}
