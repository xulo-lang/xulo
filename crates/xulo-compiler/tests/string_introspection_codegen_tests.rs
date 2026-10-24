//! String inspection prelude functions, the integer intrinsics, and the
//! shift-domain check on the compiled path (spec `builtins/prelude.md`,
//! `builtins/intrinsic-functions.md`, `expressions/operators.md`).

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
fn string_inspection_runs_on_the_compiled_path() {
    let out = compile_and_run(
        r#"
fn main() {
    print(stringSize("hello"))
    print(stringSize("héllo"))
    print(stringCodeAt("héllo", 1))
    print(stringSlice("héllo", 1, 4))
    print(stringSlice("abc", 1, 1))
    let n = stringSize("abc")
    print(n + 1)
    print(stringCodeAt("abc", 0) + 1)
    let bytes = stringBytes("hé")
    print(bytes[0])
    print(bytes[1])
    print(bytes[2])
}
"#,
    );
    assert_eq!(
        out,
        vec![
            "5".to_string(),
            "5".to_string(),
            "233".to_string(),
            "éll".to_string(),
            "".to_string(),
            "4".to_string(),
            "98".to_string(),
            "104".to_string(),
            "195".to_string(),
            "169".to_string(),
        ]
    );
}

#[test]
fn wrapping_and_bit_intrinsics_run_on_the_compiled_path() {
    let out = compile_and_run(
        r#"
fn main() {
    print(std::math::wrappingAdd(1, 2))
    print(std::math::wrappingSub(1, 2))
    print(std::math::wrappingMul(3, 4))
    print(std::math::popCount(255))
    print(std::math::popCount(-1))
    print(std::math::countLeadingZeros(1))
    print(std::math::countTrailingZeros(8))
    print(std::math::rotateLeft(1, 1))
    print(std::math::rotateRight(2, 1))
}
"#,
    );
    assert_eq!(
        out,
        vec![
            "3".to_string(),
            "-1".to_string(),
            "12".to_string(),
            "8".to_string(),
            "64".to_string(),
            "63".to_string(),
            "3".to_string(),
            "2".to_string(),
            "1".to_string(),
        ]
    );
}

#[test]
fn shifts_with_values_in_domain_run_on_the_compiled_path() {
    let out = compile_and_run(
        r#"
fn main() {
    let n = 3
    print(1 << n)
    print(-8 >> n)
}
"#,
    );
    assert_eq!(out, vec!["8".to_string(), "-1".to_string()]);
}

/// The JIT shift check is a runtime failure (nonzero exit), so the test
/// re-executes itself in a child process and asserts on its status.
#[test]
fn out_of_range_shift_traps_on_the_compiled_path() {
    if std::env::var_os("XULO_CG_SHIFT_CHILD").is_some() {
        compile_and_run(
            r#"
fn main() {
    let n = 64
    print(1 << n)
}
"#,
        );
        panic!("shift returned");
    }
    let exe = std::env::current_exe().expect("test binary path");
    let out = std::process::Command::new(exe)
        .arg("out_of_range_shift_traps_on_the_compiled_path")
        .arg("--nocapture")
        .env("XULO_CG_SHIFT_CHILD", "1")
        .output()
        .expect("spawn child test");
    assert_eq!(
        out.status.code(),
        Some(1),
        "child stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("shift amount out of range"),
        "child stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn aot_object_references_the_new_runtime_symbols() {
    let bytes = object_bytes(
        r#"
fn main() {
    stringSize("x")
    stringCodeAt("x", 0)
    stringSlice("x", 0, 1)
    stringBytes("x")
    std::math::wrappingAdd(1, 2)
    std::math::wrappingSub(1, 2)
    std::math::wrappingMul(1, 2)
    std::math::popCount(1)
    std::math::countLeadingZeros(1)
    std::math::countTrailingZeros(1)
    std::math::rotateLeft(1, 1)
    std::math::rotateRight(1, 1)
    let n = 2
    print(1 << n)
}
"#,
    );
    for symbol in [
        "xulo_string_size",
        "xulo_string_code_at",
        "xulo_string_slice",
        "xulo_string_bytes",
        "xulo_math_wrapping_add",
        "xulo_math_wrapping_sub",
        "xulo_math_wrapping_mul",
        "xulo_math_pop_count",
        "xulo_math_count_leading_zeros",
        "xulo_math_count_trailing_zeros",
        "xulo_math_rotate_left",
        "xulo_math_rotate_right",
        "xulo_shift",
    ] {
        assert!(references_symbol(&bytes, symbol), "object missing {symbol}");
    }
}
