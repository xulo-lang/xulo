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
// 命名空间 std::math:: / std::time::：常量、常用数学函数、时钟与计时
fn main() {
    // 常量
    print(std::math::PI, std::math::E, std::math::TAU)
    print(std::math::INFINITY)

    // 取整与整数运算
    print(std::math::floor(3.7), std::math::ceil(3.2), std::math::round(3.5))
    print(std::math::abs(-42))
    print(std::math::min(3, 9), std::math::max(3, 9))
    print(std::math::pow(2.0, 10.0))
    print(std::math::sqrt(2.0))

    // 定点小数格式化
    print(std::math::toFixed(3.14159, 2))

    // 三角函数与对数
    print(std::math::sin(0.0), std::math::cos(0.0))
    print(std::math::log(std::math::E))

    // 时钟
    print(std::time::now() >= 0)
    let start = std::time::monotonic_nanos()
    print(std::time::monotonic() >= 0)
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
    print(std::math::floor(-3.7), std::math::ceil(-3.2), std::math::round(-3.5), std::math::trunc(-3.7))
    print(std::math::abs(-42.5))
    print(std::math::min(9, 3), std::math::max(9, 3))
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
    print(std::math::PI)
    print(std::math::E)
    print(std::math::TAU)
    print(std::math::INFINITY)
    print(std::math::NAN)
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
    print(std::math::toFixed(3.14159, 2))
    print(std::math::toFixed(2.5, 0))
    print(std::math::toFixed(-1.05, 1))
    print(std::math::toFixed(0.5, 3))
    print(std::math::toFixed(std::math::INFINITY, 2))
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
    print(std::time::now() >= 0)
    print(std::time::now_nanos() > 0)
    print(std::time::monotonic() >= 0)
    print(std::time::monotonic_nanos() >= 0)
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
    let out = compile_and_run(r#"fn main() { let x = 3.7; print(x); print(std::math::floor(x)) }"#);
    let expected: Vec<String> = ["3.7", "3"].iter().map(|s| s.to_string()).collect();
    assert_eq!(out, expected);
}

#[test]
fn test_float_array_elements_roundtrip() {
    // 实参/入槽边界统一按声明类型对齐后，f64 值可以位模式存进数组槽
    let out = compile_and_run(r#"fn main() { print([1.5, 2]); print([std::math::sqrt(2.0)]) }"#);
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
        r#"fn main() { print(std::math::frobnicate(1)) }"#,
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

/// `std::fs` 的 AOT lowering：write / read / exists / remove 都落到运行时，
/// Result 的 Ok/Err 与解释器同构（Err 携带 message）。
#[test]
fn test_std_fs_roundtrip() {
    let base = std::env::temp_dir().join(format!(
        "xulo_cg_fs_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = base.to_string_lossy().replace('\\', "\\\\");
    let missing = format!("{path}-missing");
    let out = compile_and_run(&format!(
        r#"
fn main() {{
    let path = "{path}"
    match std::fs::write(path, "xulo fs") {{
        Result::Ok(_) => print("written")
        Result::Err(e) => print(e.message)
    }}
    print(std::fs::exists(path))
    match std::fs::read(path) {{
        Result::Ok(text) => print(text)
        Result::Err(_) => print("unexpected")
    }}
    match std::fs::remove(path) {{
        Result::Ok(_) => print("removed")
        Result::Err(e) => print(e.message)
    }}
    print(std::fs::exists(path))
    match std::fs::read("{missing}") {{
        Result::Ok(_) => print("unexpected")
        Result::Err(_) => print("missing")
    }}
}}
"#
    ));
    assert_eq!(
        out,
        vec![
            "written".to_string(),
            "true".to_string(),
            "xulo fs".to_string(),
            "removed".to_string(),
            "false".to_string(),
            "missing".to_string(),
        ]
    );
}

/// AOT 对象文件引用 `std::fs` 的运行时符号（链接期接线回归）
#[test]
fn test_aot_object_references_fs_symbols() {
    let bytes = object_bytes(
        r#"
fn main() {
    print(std::fs::exists("x"))
    std::fs::write("x", "y")
    std::fs::read("x")
    std::fs::remove("x")
    std::fs::mkdir("x")
}
"#,
    );
    for symbol in [
        "xulo_fs_read",
        "xulo_fs_write",
        "xulo_fs_exists",
        "xulo_fs_remove",
        "xulo_fs_mkdir",
    ] {
        assert!(references_symbol(&bytes, symbol), "object missing {symbol}");
    }
}

/// `std::io` 的 lowering：写入成员经运行时落真实流，`Result` 的 Ok/Err 与
/// 解释器同构（断言走缓冲的 `print` 标记，空载荷不打扰测试输出）。
#[test]
fn test_std_io_result_dispatch() {
    let out = compile_and_run(
        r#"
fn main() {
    match std::io::print("") {
        Result::Ok(_) => print("print ok")
        Result::Err(e) => print(e.message)
    }
    match std::io::eprint("") {
        Result::Ok(_) => print("eprint ok")
        Result::Err(e) => print(e.message)
    }
    match std::io::flush() {
        Result::Ok(_) => print("flush ok")
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(
        out,
        vec![
            "print ok".to_string(),
            "eprint ok".to_string(),
            "flush ok".to_string(),
        ]
    );
}

/// AOT 对象文件引用 `std::io` 的全部七个运行时符号（链接期接线回归）。
/// 只编译不运行：两个读取成员会消费调用方的 stdin。
#[test]
fn test_aot_object_references_io_symbols() {
    let bytes = object_bytes(
        r#"
fn main() {
    std::io::print("")
    std::io::println("")
    std::io::eprint("")
    std::io::eprintln("")
    std::io::flush()
    std::io::readLine()
    std::io::read()
}
"#,
    );
    for symbol in [
        "xulo_io_print",
        "xulo_io_println",
        "xulo_io_eprint",
        "xulo_io_eprintln",
        "xulo_io_flush",
        "xulo_io_read_line",
        "xulo_io_read",
    ] {
        assert!(references_symbol(&bytes, symbol), "object missing {symbol}");
    }
}

/// 裸名（group import）与别名头（`use std::io as io`）经 `apply_std_calls`
/// 改写成同一个限定键，IR 侧与解释器看到同一批成员。标记用 prelude 的
/// `println`——io 的 `print`/`println` 不进 import 面，裸名不会撞 prelude。
#[test]
fn test_bare_and_alias_io_heads_lower_like_the_qualified_form() {
    let bare = compile_and_run(
        r#"
use std::io::{flush, eprint};

fn main() {
    match eprint("") {
        Result::Ok(_) => println("bare ok")
        Result::Err(e) => println(e.message)
    }
    match flush() {
        Result::Ok(_) => println("bare flush")
        Result::Err(e) => println(e.message)
    }
}
"#,
    );
    assert_eq!(bare, vec!["bare ok".to_string(), "bare flush".to_string()]);

    let alias = compile_and_run(
        r#"
use std::io as io;

fn main() {
    match io::flush() {
        Result::Ok(_) => println("alias flush")
        Result::Err(e) => println(e.message)
    }
}
"#,
    );
    assert_eq!(alias, vec!["alias flush".to_string()]);

    let bytes = object_bytes(
        r#"
use std::io as io;

fn main() {
    io::flush()
    io::read()
}
"#,
    );
    assert!(references_symbol(&bytes, "xulo_io_flush"));
    assert!(references_symbol(&bytes, "xulo_io_read"));
}

/// `std::os` 的 lowering：可失败成员的 Result 与解释器同构，getenv 的
/// `string?` 载荷区分 null（未设置）与字符串，pid/platform/arch 是原始返回。
#[test]
fn test_std_os_getenv_setenv_and_totals() {
    let key = format!("XULO_CG_OS_TEST_{}", std::process::id());
    // SAFETY: single test process, unique plain key, restored below.
    unsafe { std::env::remove_var(&key) };
    let out = compile_and_run(&format!(
        r#"
fn main() {{
    match std::os::getenv("{key}") {{
        Result::Ok(v) => print(v ?? "unset")
        Result::Err(e) => print(e.message)
    }}
    match std::os::setenv("{key}", "from-compiled") {{
        Result::Ok(_) => print("set")
        Result::Err(e) => print(e.message)
    }}
    match std::os::getenv("{key}") {{
        Result::Ok(v) => print(v ?? "unset")
        Result::Err(e) => print(e.message)
    }}
    print(std::os::pid() > 0)
    print(std::os::platform() != "")
    print(std::os::arch() != "")
    match std::os::setenv("A=B", "v") {{
        Result::Ok(_) => print("unexpected")
        Result::Err(e) => print("rejected")
    }}
}}
"#
    ));
    // SAFETY: cleanup of this test's own variable, same thread.
    unsafe { std::env::remove_var(&key) };
    assert_eq!(
        out,
        vec![
            "unset".to_string(),
            "set".to_string(),
            "from-compiled".to_string(),
            "true".to_string(),
            "true".to_string(),
            "true".to_string(),
            "rejected".to_string(),
        ]
    );
}

/// AOT 对象文件引用 `std::os` 的全部七个运行时符号（链接期接线回归）。
#[test]
fn test_aot_object_references_os_symbols() {
    let bytes = object_bytes(
        r#"
fn main() {
    std::os::getenv("X")
    std::os::setenv("X", "y")
    std::os::cwd()
    std::os::chdir("/nonexistent-xulo-os")
    std::os::pid()
    std::os::platform()
    std::os::arch()
}
"#,
    );
    for symbol in [
        "xulo_os_getenv",
        "xulo_os_setenv",
        "xulo_os_cwd",
        "xulo_os_chdir",
        "xulo_os_pid",
        "xulo_os_platform",
        "xulo_os_arch",
    ] {
        assert!(references_symbol(&bytes, symbol), "object missing {symbol}");
    }
}

/// `std::path` 的 lowering：纯字符串成员经 FFI 返回驻留字符串（tag=0），
/// 编译路径与解释器逐行一致。
#[test]
fn test_std_path_members_run() {
    let out = compile_and_run(
        r#"
fn main() {
    print(std::path::join("/var", "log/app.log"))
    print(std::path::join("opt", "/abs"))
    print(std::path::dirname("/var/log/app.log"))
    print(std::path::basename("/var/log/app.log"))
    print(std::path::ext("app.log"))
    print(std::path::normalize("/a/./b//../c"))
}
"#,
    );
    assert_eq!(
        out,
        vec![
            "/var/log/app.log".to_string(),
            "/abs".to_string(),
            "/var/log".to_string(),
            "app.log".to_string(),
            ".log".to_string(),
            "/a/c".to_string(),
        ]
    );
}

/// `std::path` 裸名经 `use` 导入后与限定形式等价（语义层改写 + 同一 lowering）。
#[test]
fn test_std_path_bare_import_lowers_to_the_same_symbols() {
    let out = compile_and_run(
        r#"
use std::path::{join, dirname};

fn main() {
    print(join("a", "b"))
    print(dirname("a/b"))
}
"#,
    );
    assert_eq!(out, vec!["a/b".to_string(), "a".to_string()]);
}

/// `Math::random`：零参 F64 返回经 I64 位模式槽落地；打印按 tag=2 还原成
/// [0, 1) 区间的小数（连续两次抽样不同）。
#[test]
fn test_math_random_prints_unit_interval_float() {
    let out = compile_and_run(
        r#"
fn main() {
    print(std::math::random())
    print(std::math::random())
}
"#,
    );
    assert_eq!(out.len(), 2, "unexpected: {out:?}");
    let a: f64 = out[0].parse().expect("first draw is a float");
    let b: f64 = out[1].parse().expect("second draw is a float");
    assert!((0.0..1.0).contains(&a), "first draw out of range: {a}");
    assert!((0.0..1.0).contains(&b), "second draw out of range: {b}");
    assert!(a != b, "two consecutive draws collided: {a}");
}

/// AOT 对象文件引用 `std::path` 全部五个符号与 `Math::random`（链接期接线回归）。
#[test]
fn test_aot_object_references_path_and_random_symbols() {
    let bytes = object_bytes(
        r#"
fn main() {
    std::path::join("a", "b")
    std::path::dirname("a")
    std::path::basename("a")
    std::path::ext("a")
    std::path::normalize("a")
    std::math::random()
}
"#,
    );
    for symbol in [
        "xulo_path_join",
        "xulo_path_dirname",
        "xulo_path_basename",
        "xulo_path_ext",
        "xulo_path_normalize",
        "xulo_math_random",
    ] {
        assert!(references_symbol(&bytes, symbol), "object missing {symbol}");
    }
}
