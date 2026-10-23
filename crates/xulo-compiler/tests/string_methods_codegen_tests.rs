//! String-method lowering: the ten reshaping/querying members compile to the
//! runtime FFI entry points, and `split` produces a real list value.

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

/// The reshaping, querying, padding and repeating members run identically on
/// the compiled path (string results print as strings, predicates as bools).
#[test]
fn test_string_reshape_and_query_methods_run() {
    let out = compile_and_run(
        r#"
fn main() {
    print("  Hi There  ".trim())
    print("ABC".to_lowercase())
    print("hello".length())
    print("héllo".length())
    print("a,b,c".contains("b"))
    print("a,b,c".contains("z"))
    print("a,b,c".starts_with("a,"))
    print("a,b,c".ends_with(",c"))
    print("ab".repeat(3))
    print("hi".pad_start(5))
    print("hi".pad_end(5))
    print("hi".pad_start(1))
}
"#,
    );
    assert_eq!(
        out,
        vec![
            "Hi There".to_string(),
            "abc".to_string(),
            "5".to_string(),
            "5".to_string(),
            "true".to_string(),
            "false".to_string(),
            "true".to_string(),
            "true".to_string(),
            "ababab".to_string(),
            "   hi".to_string(),
            "hi   ".to_string(),
            "hi".to_string(),
        ]
    );
}

/// `split` produces a list: pieces are addressable by index, empty pieces
/// included; the empty separator splits into characters.
#[test]
fn test_string_split_produces_a_list() {
    let out = compile_and_run(
        r#"
fn main() {
    let parts = "a,,b,c".split(",")
    print(parts[0])
    print(parts[1])
    print(parts[2])
    print(parts[3])
    let chars = "héllo".split("")
    print(chars[1])
    print(chars[4])
}
"#,
    );
    assert_eq!(
        out,
        vec![
            "a".to_string(),
            "".to_string(),
            "b".to_string(),
            "c".to_string(),
            "é".to_string(),
            "o".to_string(),
        ]
    );
}

/// The chained form routes by the split receiver: `.length()` on the list is
/// not a string method, so the compile fails the same way `list.length()`
/// does today instead of calling the string FFI on an array pointer.
#[test]
fn test_split_chained_length_is_not_a_string_call() {
    let err = compile_to_ir(
        r#"
fn main() {
    print("a,b".split(",").length())
}
"#,
        Path::new("test.xulo"),
    );
    assert!(
        err.is_err(),
        "list.length must not lower as a string method"
    );
}

/// AOT object file references every string-method runtime symbol (link-time
/// wiring regression).
#[test]
fn test_aot_object_references_string_method_symbols() {
    let bytes = object_bytes(
        r#"
fn main() {
    "x".trim()
    "x".to_lowercase()
    "x".length()
    "x".contains("y")
    "x".starts_with("y")
    "x".ends_with("y")
    "x".split("y")
    "x".pad_start(3)
    "x".pad_end(3)
    "x".repeat(2)
}
"#,
    );
    for symbol in [
        "xulo_string_trim",
        "xulo_string_to_lowercase",
        "xulo_string_length",
        "xulo_string_contains",
        "xulo_string_starts_with",
        "xulo_string_ends_with",
        "xulo_string_split",
        "xulo_string_pad_start",
        "xulo_string_pad_end",
        "xulo_string_repeat",
    ] {
        assert!(references_symbol(&bytes, symbol), "object missing {symbol}");
    }
}

/// The list side of the `contains`/`length` name collision still does not
/// route into the string FFI: list receivers keep the old user-fn resolution.
#[test]
fn test_list_contains_and_length_do_not_route_to_string_ffi() {
    let err = compile_to_ir(
        r#"
fn main() {
    let xs = [1, 2, 3]
    print(xs.contains(2))
}
"#,
        Path::new("test.xulo"),
    );
    assert!(
        err.is_err(),
        "list.contains must not lower as a string method"
    );
}
