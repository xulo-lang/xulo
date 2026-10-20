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

/// examples/math_time.xulo 全文：AOT 编译路径的输出必须与解释器逐行一致
const MATH_TIME_EXAMPLE: &str = r#"
// 命名空间 Math:: / Time::：常量、常用数学函数、时钟与计时
fn main() {
    // 常量
    print(Math::PI, Math::E, Math::TAU)
    print(Math::INFINITY)

    // 取整与整数运算
    print(Math::floor(3.7), Math::ceil(3.2), Math::round(3.5))
    print(Math::abs(-42))
    print(Math::min(3, 9), Math::max(3, 9))
    print(Math::pow(2.0, 10.0))
    print(Math::sqrt(2.0))

    // 定点小数格式化
    print(Math::toFixed(3.14159, 2))

    // 三角函数与对数
    print(Math::sin(0.0), Math::cos(0.0))
    print(Math::log(Math::E))

    // 时钟
    print(Time::now() >= 0)
    let start = Time::monotonic_nanos()
    print(Time::monotonic() >= 0)
    print(start >= 0)
}
"#;

#[test]
fn test_math_time_example_matches_interpreter() {
    let out = compile_and_run(MATH_TIME_EXAMPLE);
    let expected: Vec<String> = [
        "3.141592653589793 2.718281828459045 6.283185307179586",
        "Infinity",
        "3 4 4",
        "42",
        "3 9",
        "1024",
        "1.4142135623730951",
        "3.14",
        "0 1",
        "1",
        "true",
        "true",
        "true",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(out, expected);
}

#[test]
fn test_math_float_group_rounding_and_abs() {
    // floor/ceil/round/trunc/abs/min/max：结果按 F64 位模式进 I64 槽、tag=2 打印，
    // 整数值的 float 经 format_number 呈现为 "3" 这样的整数形态
    let out = compile_and_run(
        r#"fn main() {
    print(Math::floor(-3.7), Math::ceil(-3.2), Math::round(-3.5), Math::trunc(-3.7))
    print(Math::abs(-42.5))
    print(Math::min(9, 3), Math::max(9, 3))
}"#,
    );
    let expected: Vec<String> = ["-4 -3 -4 -3", "42.5", "3 9"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(out, expected);
}

#[test]
fn test_math_constants_print() {
    let out = compile_and_run(
        r#"fn main() {
    print(Math::PI)
    print(Math::E)
    print(Math::TAU)
    print(Math::INFINITY)
    print(Math::NAN)
}"#,
    );
    let expected: Vec<String> = [
        "3.141592653589793",
        "2.718281828459045",
        "6.283185307179586",
        "Infinity",
        "NaN",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(out, expected);
}

#[test]
fn test_tofixed_string_result() {
    // toFixed 返回字符串槽（tag=0）；特殊值忽略 digits 的规则与解释器一致
    let out = compile_and_run(
        r#"fn main() {
    print(Math::toFixed(3.14159, 2))
    print(Math::toFixed(2.5, 0))
    print(Math::toFixed(-1.05, 1))
    print(Math::toFixed(0.5, 3))
    print(Math::toFixed(Math::INFINITY, 2))
}"#,
    );
    let expected: Vec<String> = ["3.14", "3", "-1.1", "0.500", "Infinity"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(out, expected);
}

#[test]
fn test_time_clock_comparisons() {
    let out = compile_and_run(
        r#"fn main() {
    print(Time::now() >= 0)
    print(Time::now_nanos() > 0)
    print(Time::monotonic() >= 0)
    print(Time::monotonic_nanos() >= 0)
}"#,
    );
    let expected: Vec<String> = ["true", "true", "true", "true"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(out, expected);
}

#[test]
fn test_bound_float_printed_as_float() {
    // number 绑定若落在 F64 槽（小数字面量），打印必须按位模式走 tag=2
    let out = compile_and_run(r#"fn main() { let x = 3.7; print(x); print(Math::floor(x)) }"#);
    let expected: Vec<String> = ["3.7", "3"].iter().map(|s| s.to_string()).collect();
    assert_eq!(out, expected);
}

#[test]
fn test_float_array_elements_roundtrip() {
    // 实参/入槽边界统一按声明类型对齐后，f64 值可以位模式存进数组槽
    let out = compile_and_run(r#"fn main() { print([1.5, 2]); print([Math::sqrt(2.0)]) }"#);
    let expected: Vec<String> = ["[1.5, 2]", "[1.4142135623730951]"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(out, expected);
}

#[test]
fn test_unknown_math_member_rejected() {
    // 语义层按 builtin_member 拦截未知成员；irgen 兜底同一错误
    let err = compile_to_ir(
        r#"fn main() { print(Math::frobnicate(1)) }"#,
        Path::new("test.xulo"),
    )
    .expect_err("unknown Math member must be rejected");
    assert!(
        err.message.contains("frobnicate") || err.message.contains("undefined"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn test_math_time_aot_references_runtime_symbols() {
    let bytes = object_bytes(MATH_TIME_EXAMPLE);
    assert!(references_symbol(&bytes, "xulo_math_floor"));
    assert!(references_symbol(&bytes, "xulo_math_to_fixed"));
    assert!(references_symbol(&bytes, "xulo_time_now"));
    assert!(references_symbol(&bytes, "xulo_time_monotonic_nanos"));
    assert!(!references_symbol(&bytes, "xulo_to_string"));
}
