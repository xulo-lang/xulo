//! AOT 语法覆盖：补足 codegen 路径（JIT 与 AOT 共用 lowering）尚未覆盖的
//! 语法面——位运算/取模/幂、continue、const、解构、默认与命名参数、下标与
//! 字段赋值、match 绑定/具名 payload/泛型枚举、多模块执行，以及 irgen 明确
//! 拒绝的语法（match guard / or-pattern / range-pattern / UI 声明）的报错。

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

fn compile_err(source: &str) -> xulo_core::error::XuloError {
    compile_to_ir(source, Path::new("test.xulo")).expect_err("expected compile error")
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

// ── 算术 / 位运算 / 一元 ──────────────────────────────────────────────────

#[test]
fn mod_and_pow_operators() {
    let out = compile_and_run(
        r#"
        fn main() {
            print(17 % 5)
            print(2 ** 10)
            print(2 * 3 % 4)
            print(2 ** 3 ** 2)
        }
        "#,
    );
    assert_eq!(out, vec!["2", "1024", "2", "512"]);
}

#[test]
fn bitwise_and_shift_operators() {
    let out = compile_and_run(
        r#"
        fn main() {
            print(6 & 3)
            print(6 | 3)
            print(6 ^ 3)
            print(1 << 4)
            print(256 >> 4)
        }
        "#,
    );
    assert_eq!(out, vec!["2", "7", "5", "16", "16"]);
}

#[test]
fn unary_not_and_bitnot() {
    let out = compile_and_run(
        r#"
        fn main() {
            print(!true)
            print(!false)
            print(~0)
            print(~5)
            print(-17)
        }
        "#,
    );
    assert_eq!(out, vec!["false", "true", "-1", "-6", "-17"]);
}

#[test]
fn move_and_copy_are_value_aliases() {
    let out = compile_and_run(
        r#"
        fn main() {
            let x = 5
            let a = move x
            let y = 6
            let b = copy y
            print(a + b)
            print(y)
        }
        "#,
    );
    assert_eq!(out, vec!["11", "6"]);
}

// ── 循环控制 ──────────────────────────────────────────────────────────────

#[test]
fn continue_skips_for_iteration() {
    let out = compile_and_run(
        r#"
        fn main() {
            let mut sum = 0
            for i in [0, 1, 2, 3, 4] {
                if i % 2 == 0 { continue }
                sum = sum + i
            }
            print(sum)
        }
        "#,
    );
    assert_eq!(out, vec!["4"]);
}

#[test]
fn continue_skips_while_iteration() {
    let out = compile_and_run(
        r#"
        fn main() {
            let mut x = 0
            let mut sum = 0
            while x < 5 {
                x = x + 1
                if x % 2 == 0 { continue }
                sum = sum + x
            }
            print(sum)
        }
        "#,
    );
    assert_eq!(out, vec!["9"]);
}

#[test]
fn early_return_from_function() {
    let out = compile_and_run(
        r#"
        fn clamp(n: number): number {
            if n < 0 { return 0 }
            if n > 10 { return 10 }
            n
        }
        fn main() {
            print(clamp(-3))
            print(clamp(7))
            print(clamp(99))
        }
        "#,
    );
    assert_eq!(out, vec!["0", "7", "10"]);
}

// ── 绑定：const / 类型标注 / 解构 ─────────────────────────────────────────

#[test]
fn const_binding_and_typed_let() {
    let out = compile_and_run(
        r#"
        fn main() {
            const LIMIT: number = 3
            let typed: number = 4
            print(LIMIT + typed)
        }
        "#,
    );
    assert_eq!(out, vec!["7"]);
}

#[test]
fn tuple_destructure_from_list_and_call() {
    let out = compile_and_run(
        r#"
        fn pair(): List<number> { return [100, 200] }
        fn main() {
            let (a, b) = [1, 2]
            let (x, y) = pair()
            print(a)
            print(b)
            print(x + y)
        }
        "#,
    );
    assert_eq!(out, vec!["1", "2", "300"]);
}

#[test]
fn object_destructure_binds_fields() {
    let out = compile_and_run(
        r#"
        fn main() {
            let obj = { name: "Alice", age: 30 }
            let { name, age } = obj
            print(name)
            print(age)
        }
        "#,
    );
    assert_eq!(out, vec!["Alice", "30"]);
}

// ── 调用：默认参数 / 命名参数 / println ───────────────────────────────────

#[test]
fn default_and_named_parameters() {
    let out = compile_and_run(
        r#"
        fn greet(name: string, punctuation: string = "!"): string {
            "Hello, " + name + punctuation
        }
        fn sub(a: number, b: number): number { a - b }
        fn main() {
            print(greet("lyy"))
            print(greet(name: "lyy", punctuation: "?"))
            print(sub(b: 3, a: 10))
        }
        "#,
    );
    assert_eq!(out, vec!["Hello, lyy!", "Hello, lyy?", "7"]);
}

#[test]
fn println_alias_prints_line() {
    let out = compile_and_run(r#"fn main() { println("hi") }"#);
    assert_eq!(out, vec!["hi"]);
}

// ── 下标 / 字段赋值 ───────────────────────────────────────────────────────

#[test]
fn index_assignment_updates_list() {
    let out = compile_and_run(
        r#"
        fn main() {
            let mut xs = [1, 2, 3]
            xs[0] = 99
            print(xs)
            print(xs[0])
        }
        "#,
    );
    assert_eq!(out, vec!["[99, 2, 3]", "99"]);
}

#[test]
fn member_assignment_updates_object_field() {
    let out = compile_and_run(
        r#"
        fn main() {
            let mut p = { x: 1, y: 2 }
            p.x = 10
            print(p.x + p.y)
        }
        "#,
    );
    assert_eq!(out, vec!["12"]);
}

// ── match：绑定 / 具名 payload / 泛型枚举 ─────────────────────────────────

#[test]
fn match_binding_pattern_catches_anything() {
    let out = compile_and_run(
        r#"
        fn main() {
            let label = match 42 {
                n => `got ${str(n)}`
            }
            print(label)
        }
        "#,
    );
    assert_eq!(out, vec!["got 42"]);
}

#[test]
fn match_binding_after_literal_arms() {
    let out = compile_and_run(
        r#"
        fn main() {
            let out = match 2 {
                1 => "one"
                n => `other(${str(n)})`
            }
            print(out)
        }
        "#,
    );
    assert_eq!(out, vec!["other(2)"]);
}

#[test]
fn match_enum_with_named_payload_fields() {
    let out = compile_and_run(
        r#"
        enum Shape {
            Circle(number)
            Rect(width: number, height: number)
        }
        fn area(s: Shape): number {
            match s {
                Shape::Circle(r) => 3 * r * r
                Shape::Rect(w, h) => w * h
            }
        }
        fn main() {
            print(area(Shape::Circle(2)))
            print(area(Shape::Rect(3, 4)))
        }
        "#,
    );
    assert_eq!(out, vec!["12", "12"]);
}

#[test]
fn match_generic_enum_over_instantiations() {
    let out = compile_and_run(
        r#"
        enum Result<T> {
            Success(T)
            Error(string)
        }
        fn describe<T>(r: Result<T>): string {
            match r {
                Result::Success(v) => `ok: ${str(v)}`
                Result::Error(msg) => `err: ${msg}`
            }
        }
        fn main() {
            print(describe(Result::Success(42)))
            print(describe(Result::Success("hi")))
            print(describe(Result::Error("boom")))
        }
        "#,
    );
    assert_eq!(out, vec!["ok: 42", "ok: hi", "err: boom"]);
}

// ── 可选类型 / 空合并 ─────────────────────────────────────────────────────

#[test]
fn optional_param_nullish_and_default() {
    let out = compile_and_run(
        r#"
        fn maybe(x: number?): number { if x == null { 0 } else { x } }
        fn main() {
            let maybeN: number? = null
            print(maybeN ?? 7)
            print(maybe(null))
            print(maybe(42))
        }
        "#,
    );
    assert_eq!(out, vec!["7", "0", "42"]);
}

// ── AOT 对象产物：本文件新增语法点也必须落到机器码 ───────────────────────

#[test]
fn bitwise_and_pow_reach_machine_code() {
    let bytes = object_bytes(r#"fn main() { print((6 & 3) + (2 ** 10) + (1 << 4)) }"#);
    assert!(references_symbol(&bytes, "xulo_print"));
}

#[test]
fn destructure_and_named_args_reach_machine_code() {
    let bytes = object_bytes(
        r#"
        fn sub(a: number, b: number): number { a - b }
        fn main() {
            let (x, y) = [10, 3]
            let obj = { v: 1 }
            let { v } = obj
            print(sub(b: y, a: x) + v)
        }
        "#,
    );
    assert!(references_symbol(&bytes, "xulo_print"));
}

// ── 多模块：load → analyze → 标注 → IR → JIT 执行 ─────────────────────────

#[test]
fn multi_module_program_compiles_and_runs() {
    let dir = std::env::temp_dir().join(format!(
        "xulo_aot_syn_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("mathlib.xulo"),
        r#"
        pub fn double(n: number): number { n * 2 }
        pub const OFFSET: number = 5
        "#,
    )
    .unwrap();
    std::fs::write(
        dir.join("main.xulo"),
        r#"
        mod mathlib;
        use mathlib::double;
        use mathlib::OFFSET;
        fn main() {
            print(double(20) + OFFSET)
        }
        "#,
    )
    .unwrap();

    let mut loaded = xulo_compiler::module::load(&dir.join("main.xulo")).unwrap();
    xulo_compiler::module::analyze(&mut loaded, false).unwrap();
    xulo_compiler::module::apply_trait_dispatch(&mut loaded);
    xulo_compiler::module::apply_std_calls(&mut loaded);
    let ir = xulo_compiler::compile_modules_to_ir(&loaded).expect("IR generation failed");
    let _ = std::fs::remove_dir_all(&dir);

    let mut codegen = CodeGen::new();
    let code_ptr = codegen.compile(&ir).expect("codegen failed");
    unsafe {
        let func: fn() = std::mem::transmute(code_ptr);
        func();
    }
    let out = xulo_runtime::runtime::xulo_take_output();
    assert_eq!(out, vec!["45"]);
}

// ── irgen 明确拒绝的语法：必须报出清晰错误（而不是静默降级）─────────────

#[test]
fn match_guard_is_rejected() {
    let err = compile_err(
        r#"
        fn main() {
            let out = match 5 {
                n if n > 3 => "big"
                _ => "small"
            }
            print(out)
        }
        "#,
    );
    assert!(
        err.message.contains("match guards"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn match_or_pattern_is_rejected() {
    let err = compile_err(
        r#"
        fn main() {
            let out = match 2 {
                1 | 2 => "low"
                _ => "high"
            }
            print(out)
        }
        "#,
    );
    assert!(
        err.message.contains("range, struct, type, or or-patterns"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn match_range_pattern_is_rejected() {
    let err = compile_err(
        r#"
        fn main() {
            let out = match 5 {
                0..<3 => "low"
                _ => "high"
            }
            print(out)
        }
        "#,
    );
    assert!(
        err.message.contains("range, struct, type, or or-patterns"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn ui_state_declaration_is_rejected() {
    let err = compile_err(
        r#"
        fn main(): View {
            @State let count: number = 0
            Text("x")
        }
        "#,
    );
    assert!(
        err.message.contains("UI state/effect/component"),
        "unexpected error: {}",
        err.message
    );
}
