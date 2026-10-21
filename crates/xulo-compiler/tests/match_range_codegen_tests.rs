//! match 字面量 pattern 与区间表达式（Range as value）的 codegen 回归：
//! JIT 执行输出必须与解释器逐行一致，AOT 产物须引用对应运行时符号。
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

fn lines(expected: &[&str]) -> Vec<String> {
    expected.iter().map(|s| s.to_string()).collect()
}

/// 数字字面量 pattern：0/1 命中各自臂，其余走通配
#[test]
fn literal_number_pattern_matches_interpreter() {
    let out = compile_and_run(
        r#"
fn main() {
    let a = 0
    print(match a { 0 => "zero" 1 => "one" _ => "many" })
    let b = 1
    print(match b { 0 => "zero" 1 => "one" _ => "many" })
    let c = 7
    print(match c { 0 => "zero" 1 => "one" _ => "many" })
}
"#,
    );
    assert_eq!(out, lines(&["zero", "one", "many"]));
}

/// 字符串 / 布尔 / null / 小数字面量 pattern
#[test]
fn literal_string_bool_null_float_patterns_match_interpreter() {
    let out = compile_and_run(
        r#"
fn main() {
    let s = "b"
    print(match s { "a" => "A" "b" => "B" _ => "?" })
    let t = true
    print(match t { true => "yes" false => "no" })
    let u: string? = null
    print(match u { null => "nil" _ => "set" })
    let v: string? = "x"
    print(match v { null => "nil" _ => "set" })
    let f = 1.5
    print(match f { 1.5 => "half" 2.5 => "two" _ => "other" })
}
"#,
    );
    assert_eq!(out, lines(&["B", "yes", "nil", "set", "half"]));
}

/// 数字 pattern 用在枚举 payload 绑定上（伴随标签来自构造侧）
#[test]
fn literal_pattern_on_enum_payload_binding_matches_interpreter() {
    let out = compile_and_run(
        r#"
enum Box { One(number) }
fn main() {
    let b = Box::One(5)
    print(match b { Box::One(v) => match v { 5 => "five" _ => "other" } _ => "?" })
    let c = Box::One(9)
    print(match c { Box::One(v) => match v { 5 => "five" _ => "other" } _ => "?" })
}
"#,
    );
    assert_eq!(out, lines(&["five", "other"]));
}

/// 区间表达式求值为列表：直接打印、存变量、与列表拼接、空区间
#[test]
fn range_expression_prints_lists_like_interpreter() {
    let out = compile_and_run(
        r#"
fn main() {
    print(0..<3)
    print(0...3)
    print(5...7)
    let r = 0..<3
    print(r)
    print(r + [9])
    let e = 3..<3
    print(e)
}
"#,
    );
    assert_eq!(
        out,
        lines(&[
            "[0, 1, 2]",
            "[0, 1, 2, 3]",
            "[5, 6, 7]",
            "[0, 1, 2]",
            "[0, 1, 2, 9]",
            "[]",
        ])
    );
}

/// while 循环的条件块入口与语句块收尾都必须以终结指令结尾（codegen 验证器）
#[test]
fn while_loop_control_flow_runs_like_interpreter() {
    let out = compile_and_run(
        r#"
fn main() {
    let mut n = 1
    while n * n < 50 {
        n = n + 1
    }
    print(n)
    let mut c = 0
    while true {
        c = c + 1
        if c > 2 { break }
    }
    print(c)
}
"#,
    );
    assert_eq!(out, lines(&["8", "3"]));
}

/// examples/control_flow.xulo 全文：JIT 输出与解释器逐行一致
#[test]
fn control_flow_example_matches_interpreter() {
    let out = compile_and_run(include_str!("../../../examples/control_flow.xulo"));
    assert_eq!(
        out,
        lines(&[
            "[0, 1, 2]",
            "[0, 1, 2, 3]",
            "[5, 6, 7]",
            "i=0",
            "i=1",
            "i=2",
            "i=3",
            "i=4",
            "ch=a",
            "ch=b",
            "ch=c",
            "5050",
            "8",
            "big",
            "pos",
            "many",
            "true",
            "true",
            "true",
            "true",
        ])
    );
}

/// AOT 对象文件引用新增的两个运行时符号（链接期接线回归）
#[test]
fn aot_object_references_literal_match_and_range_list() {
    let bytes = object_bytes(
        r#"
fn main() {
    print(0..<3)
    print(match 1 { 1 => "a" _ => "b" })
}
"#,
    );
    assert!(
        references_symbol(&bytes, "xulo_literal_match"),
        "object missing xulo_literal_match"
    );
    assert!(
        references_symbol(&bytes, "xulo_range_list"),
        "object missing xulo_range_list"
    );
}
