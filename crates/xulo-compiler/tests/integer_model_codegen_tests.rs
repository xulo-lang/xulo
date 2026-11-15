//! WP1 regression: the compiled path agrees with the interpreter on the
//! 64-bit integer model (spec `types/primitive-types.md`,
//! `expressions/operators.md`).

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

/// §6.1 in `stage1-readiness.md`: exact comparison above `2^53`, an exact
/// left shift, and the `u64` maximum literal.
#[test]
fn wp1_probe_passes_in_the_compiled_path() {
    let out = compile_and_run(
        r#"fn main() {
    print(9007199254740993 == 9007199254740992)
    print(1 << 62)
    let max: u64 = 18446744073709551615
    print(max)
}"#,
    );
    assert_eq!(
        out,
        vec!["false", "4611686018427387904", "18446744073709551615"]
    );
}

/// `u64` shares the slot bits with `int`; arithmetic, comparison, and the
/// index of a `u64` list must all use the unsigned domain.
#[test]
fn u64_arithmetic_comparison_and_index_print_unsigned() {
    let out = compile_and_run(
        r#"fn main() {
    let max: u64 = 18446744073709551615
    print(max - 1)
    print(max / 2)
    print(max % 10)
    print(max > 1)
    print((0 - 1) == 18446744073709551615)
    let xs = [max]
    print(xs[0])
    print(xs[0] > 1)
}"#,
    );
    assert_eq!(
        out,
        vec![
            "18446744073709551614",
            "9223372036854775807",
            "5",
            "true",
            "false",
            "18446744073709551615",
            "true"
        ]
    );
}

/// Integral operators lower to the checked runtime helpers (so overflow,
/// division by zero, and an out-of-range shift trap instead of wrapping).
#[test]
fn integer_operators_reach_the_checked_runtime() {
    let bytes = object_bytes(
        r#"fn main() {
    let a: int = 9223372036854775807
    print(a + 1)
    print(7 / 0)
    print(1 << 62)
    print(18446744073709551615 > 1)
    print(-1)
}"#,
    );
    for symbol in [
        "xulo_int_add",
        "xulo_int_div",
        "xulo_int_neg",
        "xulo_num_compare",
        "xulo_shift",
    ] {
        assert!(
            references_symbol(&bytes, symbol),
            "object must reference {symbol}"
        );
    }
}
