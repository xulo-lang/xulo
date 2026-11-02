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
const MATH_TIME_EXAMPLE: &str = include_str!("../../../examples/math_time.xulo");

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
        "2026-10-26T12:00:00.000Z",
        "true",
        "2026 10 26 12",
        "1",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(out, expected);
}

#[test]
fn test_math_float_group_rounding_and_abs() {
    // floor/ceil/round/trunc/abs/min/max：结果槽是 F64、tag=2 打印，
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
    print(std::time::nowNanos() > 0)
    print(std::time::monotonic() >= 0)
    print(std::time::monotonicNanos() >= 0)
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

/// `Math::random`：零参 F64 返回落 F64 槽；打印按 tag=2 还原成
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

/// `std::net` 编译路径全链路：loopback 回显（127.0.0.1:0 → 读回端口 →
/// connect 经 backlog 完成 → accept → 双向收发）。数字载荷（句柄、字节数）
/// 经 match 绑定的动态 tag 按整数打印；断言与解释器同构。
#[test]
fn test_std_net_loopback_echo_round_trip() {
    let out = compile_and_run(
        r#"
fn main() {
    match std::net::tcpListen("127.0.0.1:0") {
        Result::Ok(server) => {
            match std::net::sockLocalAddr(server) {
                Result::Ok(addr) => {
                    match std::net::tcpConnect(addr) {
                        Result::Ok(client) => {
                            match std::net::tcpAccept(server) {
                                Result::Ok(accepted) => {
                                    match std::net::sockWrite(client, "ping") {
                                        Result::Ok(n) => print(n)
                                        Result::Err(e) => print(e.message)
                                    }
                                    match std::net::sockRead(accepted, 64) {
                                        Result::Ok(text) => print(text ?? "eof")
                                        Result::Err(e) => print(e.message)
                                    }
                                    match std::net::sockWrite(accepted, "pong") {
                                        Result::Ok(n) => print(n)
                                        Result::Err(e) => print(e.message)
                                    }
                                    match std::net::sockRead(client, 64) {
                                        Result::Ok(text) => print(text ?? "eof")
                                        Result::Err(e) => print(e.message)
                                    }
                                    std::net::sockClose(client)
                                    std::net::sockClose(accepted)
                                    std::net::sockClose(server)
                                }
                                Result::Err(e) => print(e.message)
                            }
                        }
                        Result::Err(e) => print(e.message)
                    }
                }
                Result::Err(e) => print(e.message)
            }
        }
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out, vec!["4", "ping", "4", "pong"], "unexpected: {out:?}");
}

/// `std::net` 错误载荷：句柄种类错配 / 死句柄在编译路径同样是干净的
/// `Result::Err(e.message)`，与解释器一致（不崩、不读废内存）。
#[test]
fn test_std_net_error_payloads_surface_messages() {
    let out = compile_and_run(
        r#"
fn main() {
    match std::net::tcpListen("127.0.0.1:0") {
        Result::Ok(server) => {
            match std::net::sockWrite(server, "x") {
                Result::Ok(_) => print("wrote")
                Result::Err(e) => print(e.message)
            }
            match std::net::sockRead(999999, 8) {
                Result::Ok(_) => print("read")
                Result::Err(e) => print(e.message)
            }
            std::net::sockClose(server)
        }
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out.len(), 2, "unexpected: {out:?}");
    assert!(out[0].contains("listener, not a stream"), "{:?}", out[0]);
    assert!(out[1].contains("not open"), "{:?}", out[1]);
}

/// AOT 对象文件引用 `std::net` 全部九个运行时符号（链接期接线回归）。
/// 只编译不运行：accept/read 会阻塞等待对端。
#[test]
fn test_aot_object_references_net_symbols() {
    let bytes = object_bytes(
        r#"
fn main() {
    std::net::tcpListen("127.0.0.1:0")
    std::net::tcpConnect("127.0.0.1:1")
    std::net::tcpAccept(1)
    std::net::sockRead(1, 8)
    std::net::sockWrite(1, "x")
    std::net::sockShutdown(1)
    std::net::sockShutdownWrite(1)
    std::net::sockClose(1)
    std::net::sockLocalAddr(1)
    std::net::sockPeerAddr(1)
}
"#,
    );
    for symbol in [
        "xulo_net_tcp_listen",
        "xulo_net_tcp_accept",
        "xulo_net_tcp_connect",
        "xulo_net_sock_read",
        "xulo_net_sock_write",
        "xulo_net_sock_shutdown",
        "xulo_net_sock_shutdown_write",
        "xulo_net_sock_close",
        "xulo_net_sock_local_addr",
        "xulo_net_sock_peer_addr",
    ] {
        assert!(references_symbol(&bytes, symbol), "object missing {symbol}");
    }
}

/// `std::net` 裸名经 group import 与限定形式走同一条 lowering
/// （`apply_std_calls` 改写 → `std::net::` 前缀 → `Net::*` 表键）。
#[test]
fn test_std_net_bare_import_lowers_like_the_qualified_form() {
    let out = compile_and_run(
        r#"
use std::net::*;

fn main() {
    match tcpListen("127.0.0.1:0") {
        Result::Ok(server) => {
            match sockLocalAddr(server) {
                Result::Ok(addr) => print(addr)
                Result::Err(e) => print(e.message)
            }
            sockClose(server)
        }
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out.len(), 1, "unexpected: {out:?}");
    assert!(
        out[0].starts_with("127.0.0.1:") && !out[0].ends_with(":0"),
        "ephemeral port read back: {:?}",
        out[0]
    );
}

/// `std::process::run` 编译路径：子进程捕获的 Result 对象载荷（stdout/
/// stderr/code 字段）经 match 绑定与字段访问打印，与解释器同构。
#[test]
fn test_std_process_run_captures_object_payload() {
    let out = compile_and_run(
        r#"
fn main() {
    match std::process::run("echo hi; echo bad 1>&2; exit 3") {
        Result::Ok(r) => {
            print(r.stdout)
            print(r.stderr)
            print(r.code)
        }
        Result::Err(e) => print(e.message)
    }
    match std::process::run("printf '%s' one-arg") {
        Result::Ok(r) => print(r.stdout)
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(
        out,
        vec!["hi\n", "bad\n", "3", "one-arg"],
        "unexpected: {out:?}"
    );
}

/// `std::process::args()` 的 List<string> 返回在编译路径可索引、可迭代
/// （元素 tag=0 驻留字符串）。
#[test]
fn test_std_process_args_indexes_and_iterates() {
    let out = compile_and_run(
        r#"
fn main() {
    let argv = std::process::args()
    print(argv[0] != "")
    let mut n = 0
    for a in argv {
        n = n + 1
    }
    print(n > 0)
}
"#,
    );
    assert_eq!(out, vec!["true", "true"], "unexpected: {out:?}");
}

/// AOT 对象文件引用 `std::process` 的全部四个运行时符号（链接期接线回归）。
#[test]
fn test_aot_object_references_process_symbols() {
    let bytes = object_bytes(
        r#"
fn main() {
    std::process::args()
    std::process::run("true")
    std::process::run("true", "")
    std::process::exec("true", [])
}
"#,
    );
    for symbol in [
        "xulo_process_args",
        "xulo_process_run",
        "xulo_process_exec",
        "xulo_process_exit",
    ] {
        assert!(references_symbol(&bytes, symbol), "object missing {symbol}");
    }
}

/// `std::process::exec` 编译路径：直接执行（不经 shell）、可选 input/
/// timeout 的 IR 补齐，与解释器同构。
#[test]
fn test_std_process_exec_runs_directly_and_pads_optionals() {
    let out = compile_and_run(
        r#"
fn main() {
    match std::process::exec("echo", ["$((2 + 3))"]) {
        Result::Ok(r) => print(r.stdout)
        Result::Err(e) => print(e.message)
    }
    match std::process::exec("cat", [], "payload") {
        Result::Ok(r) => print(r.stdout)
        Result::Err(e) => print(e.message)
    }
    match std::process::exec("/nonexistent/xulo-probe", []) {
        Result::Ok(r) => print(r.stdout)
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out.len(), 3, "unexpected: {out:?}");
    assert_eq!(out[0], "$((2 + 3))\n", "no shell: literal argument");
    assert_eq!(out[1], "payload");
    assert!(out[2].contains("cannot run"), "unexpected: {out:?}");
}

/// `run` 的可选 `timeout` 也在编译路径生效（-1 → 不限时，数值预算按毫秒）。
#[test]
fn test_std_process_run_timeout_budget_in_the_aot_path() {
    let out = compile_and_run(
        r#"
fn main() {
    match std::process::run("sleep 10", null, 300) {
        Result::Ok(r) => print(r.stdout)
        Result::Err(e) => print(e.message)
    }
    match std::process::run("echo edge", null, 60000) {
        Result::Ok(r) => print(r.stdout)
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out.len(), 2, "unexpected: {out:?}");
    assert!(out[0].contains("timed out after 300 ms"), "unexpected: {out:?}");
    assert_eq!(out[1], "edge\n");
}

/// AOT gap：`launch`/`Command`/`Child` 保留子进程句柄，只有解释器能跑——
/// 编译路径在 IR 生成就以 "undefined function" 拒绝（与 fs 字节成员同列）。
#[test]
fn test_aot_gap_launch_and_command_need_the_interpreter() {
    for src in [
        "fn main() { std::process::launch(\"make\") }",
        "fn main() { std::process::Command::shell(\"make\") }",
        "use std::process::Command;\nfn main() { Command::shell(\"make\").run() }",
    ] {
        let err = compile_to_ir(src, Path::new("test.xulo"))
            .expect_err("handle members have no AOT lowering");
        assert!(
            err.message.contains("undefined function"),
            "{src}: {}",
            err.message
        );
    }
}

/// JIT 路径的 `std::process::exit`：进程在 `print` 输出落地后带着指定
/// 退出码死亡——所以本测试重新执行自身（子进程带环境标记进入该分支），
/// 父进程断言子进程的状态码与 stdout。就地调用会带走测试宿主。
#[test]
fn test_jit_exit_terminates_with_the_requested_code() {
    if std::env::var_os("XULO_CG_EXIT_CHILD").is_some() {
        compile_and_run(
            r#"
fn main() {
    print("bye from jit")
    std::process::exit(7)
}
"#,
        );
        // Unreachable: exit(7) ends the child test process first.
        panic!("exit returned");
    }
    let exe = std::env::current_exe().expect("test binary path");
    let out = std::process::Command::new(exe)
        .arg("test_jit_exit_terminates_with_the_requested_code")
        // The harness otherwise captures `println!` per test and would
        // only print it when the test finishes — exit kills the child
        // first, so the line must go straight to fd 1.
        .arg("--nocapture")
        .env("XULO_CG_EXIT_CHILD", "1")
        .output()
        .expect("spawn child test");
    assert_eq!(
        out.status.code(),
        Some(7),
        "child stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("bye from jit"),
        "child stdout missed the print: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// `std::json` 的 JIT 落地：parse 载荷按 tag 走成员访问，stringify 带
/// 调用点推断的值标签（对象 4 / 列表 3 / 浮点位模式 2），失败路径
/// （语法错误、NaN）是 Result::Err 数据。
#[test]
fn test_std_json_parse_and_stringify_roundtrip() {
    let out = compile_and_run(
        r#"
fn main() {
    match std::json::parse("{\"a\": [1, \"two\"], \"b\": 2.5}") {
        Result::Ok(v) => {
            print(v.b)
            match std::json::stringify(v) {
                Result::Ok(s) => print(s)
                Result::Err(e) => print(e.message)
            }
        }
        Result::Err(e) => print(e.message)
    }
    match std::json::stringify([1, true, null]) {
        Result::Ok(s) => print(s)
        Result::Err(e) => print(e.message)
    }
    match std::json::parse("{oops") {
        Result::Ok(v) => match std::json::stringify(v) {
            Result::Ok(s) => print(s)
            Result::Err(e) => print(e.message)
        }
        Result::Err(e) => print(e.message)
    }
    // 浮点内建链式传参：determine_element_tag 按调用返回 tag=2，
    // 值在传参边界按位模式进入 json 的 I64 载荷槽
    match std::json::stringify(std::math::sqrt(-1.0)) {
        Result::Ok(s) => print(s)
        Result::Err(e) => print(e.message)
    }
}
"#,
    );
    assert_eq!(out.len(), 5, "unexpected: {out:?}");
    assert_eq!(out[0], "2.5");
    assert_eq!(out[1], r#"{"a":[1,"two"],"b":2.5}"#);
    assert_eq!(out[2], "[1,true,null]");
    // serde 的语法诊断带列号，按前缀断言免得钉死版本措辞
    assert!(
        out[3].starts_with("invalid JSON: "),
        "unexpected: {}",
        out[3]
    );
    assert_eq!(out[4], "cannot serialize NaN as JSON");
}

/// AOT 对象文件引用 `std::json` 的两个运行时符号（链接期接线回归）。
#[test]
fn test_aot_object_references_json_symbols() {
    let bytes = object_bytes(
        r#"
fn main() {
    std::json::parse("[1, 2]")
    std::json::stringify("x")
}
"#,
    );
    for symbol in ["xulo_json_parse", "xulo_json_stringify"] {
        assert!(references_symbol(&bytes, symbol), "object missing {symbol}");
    }
}

/// `std::fmt` 的 JIT 落地：模板加 tag 化载荷的参数数组（字符串 0 / 整数
/// 1 / 浮点 2 / 列表 3 / 布尔 5 / null 6），输出与解释器逐行一致。
#[test]
fn test_std_fmt_format_matches_interpreter() {
    let out = compile_and_run(
        r#"
fn main() {
    print(std::fmt::format("%6.2f", [3.14159]))
    print(std::fmt::format("100%%"))
    print(std::fmt::format("%-8s|%d", ["hi", 42]))
    print(std::fmt::format("%#08x", [255]))
    print(std::fmt::format("%.3g, %q", [0.1, "hi"]))
    print(std::fmt::format("%d", [-42]))
    print(std::fmt::format("%x / %#X", [255, 255]))
    print(std::fmt::format("%b", [5]))
    print(std::fmt::format("%f", [2.5]))
    print(std::fmt::format("%e", [1234.5]))
    print(std::fmt::format("%.3g", [0.1]))
    print(std::fmt::format("%g", [0.0000001]))
    print(std::fmt::format("%q", ["a\n"]))
    print(std::fmt::format("%c", [8364]))
    print(std::fmt::format("%t / %v", [true, null]))
    print(std::fmt::format("%-6s|", ["hi"]))
    print(std::fmt::format("%+8.2f", [3.1]))
    print(std::fmt::format("%.3s", ["hello"]))
    print(std::fmt::format("%s", [[1, 2]]))
    print(std::fmt::format("%v", [2.5]))
    print(std::fmt::format("%d", [2.0]))
    // 负零经浮点槽进入（整数字面量取负走 I64 槽，是既有降级约定）
    print(std::fmt::format("%f", [-std::math::sqrt(0.0)]))
}
"#,
    );
    assert_eq!(
        out,
        vec![
            "  3.14",
            "100%",
            "hi      |42",
            "0x0000ff",
            "0.1, \"hi\"",
            "-42",
            "ff / 0XFF",
            "101",
            "2.500000",
            "1.234500e+03",
            "0.1",
            "1e-07",
            "\"a\\n\"",
            "€",
            "true / null",
            "hi    |",
            "   +3.10",
            "hel",
            "[1, 2]",
            "2.5",
            "2",
            "-0.000000",
        ]
    );
}

/// examples/fmt.xulo 全文：AOT/JIT 编译路径的输出必须与解释器逐行一致
const FMT_EXAMPLE: &str = include_str!("../../../examples/fmt.xulo");

#[test]
fn test_fmt_example_matches_interpreter() {
    let out = compile_and_run(FMT_EXAMPLE);
    let expected: Vec<String> = [
        "hello, world",
        "42 / 2.5",
        "\"a\\tb\"",
        "€ / true",
        "Point(x: 1, y: 2)",
        "12345:2:false",
        "12345:1:false",
        "12345:2:true",
        "255 ff FF 100 101",
        "0xff 0XFF 0o100 0b101",
        "-42",
        "2.500000",
        "3.14",
        "1.234500e+03",
        "0.1",
        "1e-07",
        "NaN",
        "  Infinity",
        "hi      |",
        "-0003.10",
        "   +3.10",
        "0x0000ff",
        "hel",
        "[1, 2]",
        "100%",
        "42",
        "ff",
        "no directives",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(out, expected, "unexpected output for examples/fmt.xulo");
}

/// `std::fmt` 的文本动词咨询用户 `Display`/`Debug`：`Format` 来自指令的
/// precision/#，返回值不再二次截断，表渲染才截断（std-spec fmt change）。
#[test]
fn test_fmt_text_verbs_consult_user_types() {
    let out = compile_and_run(
        r#"
#Derive(Debug)
struct Point { x: int, y: int }
struct Money { cents: int }
impl Display for Money {
    fn display(self, format: Format): string {
        let p = format.precision ?? 2
        return `${self.cents}:${p}:${format.alternate}`
    }
}
fn main() {
    print(std::fmt::format("%v", [Point(x: 1, y: 2)]))
    print(std::fmt::format("%s", [Money(cents: 12345)]))
    print(std::fmt::format("%.1s", [Money(cents: 12345)]))
    print(std::fmt::format("%#s", [Money(cents: 12345)]))
    print(std::fmt::format("%.3s", [[1, 2, 3]]))
}
"#,
    );
    assert_eq!(
        out,
        vec![
            "Point(x: 1, y: 2)",
            "12345:2:false",
            "12345:1:false",
            "12345:2:true",
            "[1,",
        ]
    );
}

/// AOT 对象文件引用 `std::fmt` 的运行时符号（链接期接线回归）。
#[test]
fn test_aot_object_references_fmt_symbol() {
    let bytes = object_bytes(
        r#"
fn main() {
    std::fmt::format("%d", [1])
    std::fmt::format("100%%")
}
"#,
    );
    assert!(
        references_symbol(&bytes, "xulo_fmt_format"),
        "object missing xulo_fmt_format"
    );
}

/// f64 内建结果落在 F64 槽：绑定、注解绑定、算术、比较都走浮点域
#[test]
fn test_math_float_result_slot_feeds_arithmetic() {
    let out = compile_and_run(
        r#"fn main() {
    let a = std::math::sqrt(9.0)
    print(a)
    let qa: number = std::math::pow(2.0, 0.0)
    print(qa)
    print(std::math::sqrt(4.0) + 1)
    print(std::math::sqrt(4.0) == 2)
    print(std::math::sqrt(4.0) * 2)
    print(std::math::floor(3.7) + std::math::ceil(3.2))
    print(-std::math::sqrt(4.0))
}
"#,
    );
    assert_eq!(out, vec!["3", "1", "3", "true", "4", "7", "-2"]);
}

/// std::math float 常量参与算术/比较（EnumRef 与裸名都要识别成 F64 操作数）
#[test]
fn test_math_const_arithmetic_and_comparison() {
    let out = compile_and_run(
        r#"fn main() {
    print(std::math::PI + 1)
    print(std::math::PI > 3, std::math::PI == 3.14)
    print(std::math::E * 2)
}
"#,
    );
    assert_eq!(
        out,
        vec!["4.141592653589793", "true false", "5.43656365691809"]
    );
}

/// 浮点操作数的 `%` / 位运算 / `~`：对齐解释器 `as i64` 截断与 f64 rem 语义
#[test]
fn test_float_mod_bitwise_and_bitnot() {
    let out = compile_and_run(
        r#"fn main() {
    print(3.7 % 2)
    print(2.5 & 3)
    print(~2.5)
    print(std::math::sqrt(4.0) % 2)
    print(std::math::sqrt(4.0) & 1)
    let m = std::math::sqrt(9.0)
    print(m % 2, m & 1)
    print(-3.7 % 2, 7.5 % -2)
}
"#,
    );
    assert_eq!(
        out,
        vec![
            "1.7000000000000002",
            "2",
            "-3",
            "0",
            "0",
            "1 1",
            "-1.7000000000000002 1.5",
        ]
    );
}

/// 浮点索引按整数语义截断（`a[sqrt(4.0)]` == `a[2]`），不再触发调用实参类型错误
#[test]
fn test_float_index_truncates_to_integer() {
    let out = compile_and_run(
        r#"fn main() {
    let a = [10, 20, 30]
    print(a[std::math::sqrt(4.0)])
    let mut b = [1, 2, 3]
    b[std::math::sqrt(4.0)] = 99
    print(b[2])
}
"#,
    );
    assert_eq!(out, vec!["30", "99"]);
}

/// 浮点上界的区间：F64 槽直接进 xulo_range_list（此前 I64 位模式会被当数值）
#[test]
fn test_float_range_bound() {
    let out = compile_and_run(
        r#"fn main() {
    let e = std::math::sqrt(4.0)
    print(0..<e)
}
"#,
    );
    assert_eq!(out, vec!["[0, 1]"]);
}

/// AOT 对象文件引用 `std::http` 客户端的两个运行时符号（链接期接线回归）。
#[test]
fn test_aot_object_references_http_symbols() {
    let bytes = object_bytes(
        r#"
fn main() {
    std::http::fetch("http://127.0.0.1:1/")
    std::http::request("GET", "http://127.0.0.1:1/")
}
"#,
    );
    for symbol in ["xulo_http_fetch", "xulo_http_request"] {
        assert!(references_symbol(&bytes, symbol), "object missing {symbol}");
    }
}

/// `std::http::fetch` 编译路径：载荷对象的字段访问（status/headers/body）
/// 与解释器同构；可选 timeout 的 IR 补齐（-1 → 不限时）。
#[test]
fn test_std_http_fetch_reads_the_answer_fields() {
    let server = xulo_std::http::listen("127.0.0.1:0").expect("bind");
    let addr = server.addr().expect("addr");
    std::thread::spawn(move || {
        let _ = server.serve(|_req| xulo_std::http::Response::text("from aot", 200));
    });
    let out = compile_and_run(&format!(
        r#"
fn main() {{
    match std::http::fetch("http://{addr}/health") {{
        Result::Ok(res) => {{
            print(res.status)
            print(res.body)
        }}
        Result::Err(e) => print(e.message)
    }}
    match std::http::fetch("http://{addr}/x", 60000) {{
        Result::Ok(res) => print(res.body)
        Result::Err(e) => print(e.message)
    }}
}}
"#
    ));
    assert_eq!(
        out,
        vec!["200".to_string(), "from aot".to_string(), "from aot".to_string()],
        "unexpected: {out:?}"
    );
}

/// `std::http::request` 编译路径：method/headers/body 三个可选参的补齐
/// 与载荷往返。
#[test]
fn test_std_http_request_roundtrip_in_the_aot_path() {
    let server = xulo_std::http::listen("127.0.0.1:0").expect("bind");
    let addr = server.addr().expect("addr");
    std::thread::spawn(move || {
        let _ = server.serve(|req| {
            let who = req.form_value("name").unwrap_or("stranger".to_string());
            xulo_std::http::Response::text(&format!("{} {}", req.method, who), 201)
        });
    });
    let out = compile_and_run(&format!(
        r#"
fn main() {{
    match std::http::request(
        "POST",
        "http://{addr}/greet",
        [{{ name: "content-type", value: "application/x-www-form-urlencoded" }}],
        "name=aot"
    ) {{
        Result::Ok(res) => {{
            print(res.status)
            print(res.body)
        }}
        Result::Err(e) => print(e.message)
    }}
    match std::http::request("GET", "http://{addr}/plain") {{
        Result::Ok(res) => print(res.body)
        Result::Err(e) => print(e.message)
    }}
}}
"#
    ));
    assert_eq!(
        out,
        vec![
            "201".to_string(),
            "POST aot".to_string(),
            "GET stranger".to_string()
        ],
        "unexpected: {out:?}"
    );
}

/// AOT gap：`listen`/`Server` 与对象上的方法/构造器只有解释器能跑——
/// 编译路径在 IR 生成就拒绝（与 fs 字节成员、process 的 launch 同列）。
#[test]
fn test_aot_gap_http_server_needs_the_interpreter() {
    for (src, needle) in [
        (
            "fn main() { std::http::listen(\"127.0.0.1:1\") }",
            "undefined function",
        ),
        (
            "fn main() { std::http::Response.text(\"x\") }",
            "undefined variant",
        ),
        (
            "fn main() { std::http::Response(status: 200, headers: [], body: \"ok\") }",
            "undefined function",
        ),
    ] {
        let err = compile_to_ir(src, Path::new("test.xulo"))
            .expect_err("server/object members have no AOT lowering");
        assert!(
            err.message.contains(needle),
            "{src}: {}",
            err.message
        );
    }
}

/// AOT 对象文件引用 `std::time` 日历的全部五个运行时符号（链接期接线回归）。
#[test]
fn test_aot_object_references_time_calendar_symbols() {
    let bytes = object_bytes(
        r#"
fn main() {
    std::time::format(0)
    std::time::parse("1970-01-01T00:00:00.000Z")
    std::time::parts(0)
    std::time::fromParts(2026, 10, 26)
    std::time::weekday(0)
}
"#,
    );
    for symbol in [
        "xulo_time_format",
        "xulo_time_parse",
        "xulo_time_parts",
        "xulo_time_from_parts",
        "xulo_time_weekday",
    ] {
        assert!(references_symbol(&bytes, symbol), "object missing {symbol}");
    }
}

/// `std::time` 日历的编译路径：format/parse 交换、parts 字段读取、
/// fromParts 的可选时间字段补齐（缺省 0.0），与解释器同构。
#[test]
fn test_std_time_calendar_roundtrips_in_the_aot_path() {
    let out = compile_and_run(
        r#"
fn main() {
    let noon = std::time::fromParts(2026, 10, 26, 12)
    print(std::time::format(noon))
    match std::time::parse("2026-10-26T12:00:00.000Z") {
        Result::Ok(ms) => print(ms == noon)
        Result::Err(e) => print(e.message)
    }
    let f = std::time::parts(noon)
    print(f.year)
    print(f.day)
    print(str(std::time::weekday(noon)))
    print(std::time::format(std::time::fromParts(2026, 10, 26)))
}
"#,
    );
    assert_eq!(
        out,
        vec![
            "2026-10-26T12:00:00.000Z".to_string(),
            "true".to_string(),
            "2026".to_string(),
            "26".to_string(),
            "1".to_string(),
            "2026-10-26T00:00:00.000Z".to_string(),
        ],
        "unexpected: {out:?}"
    );
}
