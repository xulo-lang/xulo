//! AOT 语法覆盖：补足 codegen 路径（JIT 与 AOT 共用 lowering）尚未覆盖的
//! 语法面——位运算/取模/幂、continue、const、解构、默认与命名参数、下标与
//! 字段赋值、match 绑定/具名 payload/泛型枚举/guard/or/区间/类型模式、多模块
//! 执行，以及 irgen 仍明确拒绝的语法（带绑定名的 or-pattern）的报错。

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
fn string_builder_appends_and_builds() {
    let out = compile_and_run(
        r#"
        fn main() {
            let mut b = StringBuilder::new()
            b.append("héllo")
            print(b.len)
            print(b.byte_len)
            print(b)
            b.clear()
            print(b.len)
            print(b == StringBuilder::new())
            b.append("ab")
            b.append("cd")
            println(b.build())
            print(b.build().len)
            let mut same = StringBuilder::new()
            same.append("abcd")
            print(b == same)
            let dynamic = "a" + "é"
            b.append(dynamic)
            print(b.len)
            print(b.byte_len)
            print(b.build())
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "5", "6", "héllo", "0", "true", "abcd", "4", "true", "6", "7", "abcdaé"
        ]
    );
}

#[test]
fn tail_expression_evaluated_once() {
    let out = compile_and_run(
        r#"
        fn tick(): number {
            print("tick")
            1
        }
        fn via_call(): number { tick() }
        fn via_if(n: number): number {
            if n > 0 { tick() } else { 0 }
        }
        fn main() {
            print(via_call())
            print(via_if(1))
        }
        "#,
    );
    assert_eq!(out, vec!["tick", "1", "tick", "1"]);
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

/// `copy` deep-copies reference values (spec `memory-and-runtime.md`,
/// "Ownership with `copy`"). `move` keeps the operand's slot, including the
/// F64 slot a copied `float` needs; before R8's fix the compiled path aliased
/// lists, maps, sets, and builders, and a copied float panicked in codegen.
#[test]
fn copy_detaches_reference_values_and_keeps_float_slots() {
    let out = compile_and_run(
        r#"
        enum Holder { Holder(List<number>) }

        fn main() {
            let xs = [1, 2, 3]
            let ys = copy xs
            let mut zs = ys
            zs[0] = 9
            print(xs[0])
            print(ys[0])
            print(zs[0])

            let base = { a: 1, inner: [1, 2] }
            let mut dup = copy base
            dup.a = 5
            dup.inner[0] = 7
            print(base.a)
            print(dup.a)
            print(base.inner[0])
            print(dup.inner[0])

            let mut builder = StringBuilder::new()
            builder.append("x")
            let mut detached = copy builder
            detached.append("y")
            print(builder.build())
            print(detached.build())

            let set = Set::from_list([1, 2, 2])
            let mut set_copy = copy set
            set_copy.insert(3)
            print(set.size)
            print(set_copy.size)

            let h = Holder::Holder([1])
            let mut h2 = copy h
            match h2 { Holder::Holder(items) => { let mut owned = items; owned[0] = 8 } }
            match h { Holder::Holder(items) => { print(items[0]) } }
            match h2 { Holder::Holder(items) => { print(items[0]) } }

            let f = 1.5
            let g = copy f
            print(g)
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "1", "9", "9", "1", "5", "1", "7", "x", "xy", "2", "3", "1", "8", "1.5",
        ]
    );
}

/// R9c: `Task::resolve`/`all`/`race` lower to their already-settled values
/// because the compiled async model runs calls eagerly, and `Task::timeout`
/// decides from the measured runtime (spec `concurrency.md`, combinators).
#[test]
fn task_combinators_follow_the_compiled_async_model() {
    let out = compile_and_run(
        r#"
        async fn fast(id: number): number {
            id + 1
        }

        async fn slow(): number {
            await std::time::sleep(30)
            7
        }

        async fn main() {
            print(await Task::all([fast(1), fast(2)]))
            print(await Task::race([fast(3), fast(4)]))
            print(await Task::resolve("ready"))
            print(await Task::timeout(fast(5), 100))
            print(await Task::timeout(slow(), 1))
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "[2, 3]",
            "4",
            "ready",
            "Result.Ok(6)",
            "Result.Err({ message: timed out })",
        ]
    );
}

/// R8: plain `let`/assignment follows the value/reference table (spec
/// `memory-and-runtime.md`): struct, enum, tuple, and `Array` values get their
/// own container, while `List`, `Map`, `Set`, and `StringBuilder` stay shared.
#[test]
fn bindings_copy_value_aggregates_and_share_references() {
    let out = compile_and_run(
        r#"
        struct Inner { n: int }
        struct Point { x: int, inner: Inner, items: List<int> }
        fn bump(p: mut Point) {
            p.x = 9
        }
        fn main() {
            let a = Point(x: 1, inner: Inner(n: 1), items: [1, 2])
            let mut b = a
            b.x = 9
            b.inner.n = 8
            b.items[0] = 7
            print(a.x)
            print(b.x)
            print(a.inner.n)
            print(b.inner.n)
            print(a.items[0])

            let mut point = Point(x: 0, inner: Inner(n: 0), items: [])
            point = a
            point.x = 5
            print(a.x)
            print(point.x)

            let mut t = (1, 2)
            let mut u = t
            u.0 = 9
            print(t.0)
            print(u.0)

            let mut xs: Array<number, 2> = [1, 2]
            let mut ys = xs
            ys[0] = 8
            print(xs[0])
            print(ys[0])

            let xs2 = [1, 2]
            let mut ys2 = xs2
            ys2[0] = 9
            print(xs2[0])

            let mut p = Point(x: 1, inner: Inner(n: 1), items: [])
            bump(p)
            print(p.x)
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "1", "9", "1", "8", "7", "1", "5", "1", "9", "1", "8", "9", "9"
        ]
    );
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

/// Tuple positional access must resolve through nested tuple types, and a
/// tuple that crosses a call boundary (argument and return) must keep its
/// array tag instead of printing the raw pointer.
#[test]
fn nested_tuple_access_and_tuple_pass_return() {
    let out = compile_and_run(
        r#"
        fn pair(): (int, string) { (1, "one") }
        fn swap(t: (int, string)): (string, int) { (t.1, t.0) }
        fn main() {
            print(pair().0)
            let (n, word) = pair()
            print(n, word)
            let nested = ((1, 2), "x")
            print(nested.0.0, nested.0.1, nested.1)
            print(swap((3, "three")))
        }
        "#,
    );
    assert_eq!(out, vec!["1", "1 one", "1 2 x", "[three, 3]"]);
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
            let maybe_n: number? = null
            print(maybe_n ?? 7)
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
fn match_guards_or_ranges_and_type_patterns() {
    let out = compile_and_run(
        r#"
        enum Shape {
            Circle(radius: number)
            Rect(width: number, height: number)
        }
        fn classify(n: int): string {
            match n {
                0 => "zero"
                1 | 2 => "small"
                3..<10 => "medium"
                10..=99 => "large"
                _ => "huge"
            }
        }
        fn graded(score: int): string {
            match score {
                s if s >= 90 => "A"
                s if s >= 80 => "B"
                _ => "C"
            }
        }
        fn area(shape: Shape): number {
            match shape {
                Shape::Circle(r) => 3 * r * r
                Shape::Rect(w, h) => w * h
            }
        }
        fn describe(value: unknown): string {
            match value {
                null => "nothing"
                bool b => `bool ${b}`
                int i => `int ${i}`
                float f => `float ${f}`
                string s => s
                _ => "other"
            }
        }
        fn main() {
            print(classify(0), classify(2), classify(7), classify(42), classify(1000))
            print(graded(95), graded(85), graded(70))
            print(area(Shape::Circle(2)))
            print(area(Shape::Rect(3, 4)))
            print(describe(null))
            print(describe(true))
            print(describe(7))
            print(describe(1.5))
            print(describe("xulo"))
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "zero small medium large huge",
            "A B C",
            "12",
            "12",
            "nothing",
            "bool true",
            "int 7",
            "float 1.5",
            "xulo",
        ]
    );
}

#[test]
fn or_pattern_with_bindings_is_rejected() {
    let err = compile_err(
        r#"
        fn main() {
            let value: unknown = 1
            let out = match value {
                int x | string x => "nope"
                _ => "other"
            }
            print(out)
        }
        "#,
    );
    assert!(
        err.message
            .contains("or-pattern alternatives that bind names"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn defer_runs_at_block_exit_in_reverse() {
    let out = compile_and_run(
        r#"
        fn work(): int {
            defer print("defer 1")
            defer {
                print("defer 2")
            }
            print("body")
            7
        }
        fn early(flag: bool): int {
            defer print("early defer")
            if flag {
                print("returning early")
                return 1
            }
            print("falling through")
            2
        }
        fn main() {
            print(work())
            print(early(true))
            print(early(false))
            for i in 0..<3 {
                defer print(`iteration ${i} done`)
                print(`iteration ${i}`)
            }
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "body",
            "defer 2",
            "defer 1",
            "7",
            "returning early",
            "early defer",
            "1",
            "falling through",
            "early defer",
            "2",
            "iteration 0",
            "iteration 0 done",
            "iteration 1",
            "iteration 1 done",
            "iteration 2",
            "iteration 2 done",
        ]
    );
}

#[test]
fn defer_runs_on_question_propagation() {
    let out = compile_and_run(
        r#"
        fn parse(v: int): Result<int, Error> {
            if v < 0 {
                return Result::Err(Error("bad"))
            }
            Result::Ok(v)
        }
        fn double(v: int): Result<int, Error> {
            defer print("cleanup")
            let x = parse(v)?
            print("body")
            Result::Ok(x * 2)
        }
        fn main() {
            match double(21) {
                Result::Ok(v) => print(v)
                Result::Err(e) => print(e.message)
            }
            match double(-1) {
                Result::Ok(_) => print("ok")
                Result::Err(e) => print(e.message)
            }
        }
        "#,
    );
    assert_eq!(out, vec!["body", "cleanup", "42", "cleanup", "bad"]);
}

#[test]
fn defer_runs_on_break_continue_and_return() {
    let out = compile_and_run(
        r#"
        fn early(flag: bool): int {
            defer print("outer defer")
            for i in 0..<3 {
                defer print(`loop ${i}`)
                if flag and i == 1 {
                    return 9
                }
                print(`step ${i}`)
            }
            4
        }
        fn main() {
            for i in 0..<4 {
                defer print(`d${i}`)
                if i == 1 {
                    continue
                }
                if i == 3 {
                    break
                }
                print(`b${i}`)
            }
            print(early(true))
            print(early(false))
        }
        "#,
    );
    assert_eq!(
        out,
        vec![
            "b0",
            "d0",
            "d1",
            "b2",
            "d2",
            "d3",
            "step 0",
            "loop 0",
            "loop 1",
            "outer defer",
            "9",
            "step 0",
            "loop 0",
            "step 1",
            "loop 1",
            "step 2",
            "loop 2",
            "outer defer",
            "4",
        ]
    );
}

#[test]
fn chained_assignment_lowers_inner_first() {
    let out = compile_and_run(
        r#"
        fn main() {
            let mut a = 0
            let mut b = 0
            a = b = 5
            print(a, b)
            let mut o = { x: 0 }
            let mut xs = [0]
            o.x = xs[0] = 3
            print(o.x, xs[0])
        }
        "#,
    );
    assert_eq!(out, vec!["5 5", "3 3"]);
}

#[test]
fn map_members_reach_machine_code() {
    let bytes = object_bytes(
        r#"
        fn main() {
            let mut m: Map<string, int> = { a: 1 }
            m.set("b", 2)
            print(m.get("a"))
            print(m.has("b"))
            print(m.remove("a"))
            print(m.size)
            print(m.keys())
            for k in m { print(k) }
            let n = Map::from_entries([("a", 1)])
            print(n.get("a"))
        }
        "#,
    );
    assert!(references_symbol(&bytes, "xulo_object_set"));
    assert!(references_symbol(&bytes, "xulo_object_get"));
    assert!(references_symbol(&bytes, "xulo_object_get_tag"));
    assert!(references_symbol(&bytes, "xulo_object_has"));
    assert!(references_symbol(&bytes, "xulo_object_remove"));
    assert!(references_symbol(&bytes, "xulo_object_size"));
    assert!(references_symbol(&bytes, "xulo_object_keys"));
    assert!(references_symbol(&bytes, "xulo_map_from_entries"));
}

#[test]
fn fixed_array_len_compiles() {
    let out = compile_and_run(
        r#"
        fn main() {
            let numbers: Array<int, 3> = [1, 2, 3]
            print(numbers.len)
            print(numbers[2])
        }
        "#,
    );
    assert_eq!(out, vec!["3", "3"]);
}

#[test]
fn nested_ternary_compiles() {
    let out = compile_and_run(
        r#"
        fn main() {
            let score = 85
            let grade = score >= 90 ? "A" : score >= 60 ? "B" : "C"
            print(grade)
        }
        "#,
    );
    assert_eq!(out, vec!["B"]);
}

#[test]
fn spread_lowering_reaches_machine_code() {
    let bytes = object_bytes(
        r#"
        fn main() {
            let head = [1, 2]
            let all = [...head, 3]
            let base = { a: 1 }
            let merged = { ...base, b: 2 }
            print(all)
            print(merged.b)
        }
        "#,
    );
    assert!(references_symbol(&bytes, "xulo_array_concat"));
    assert!(references_symbol(&bytes, "xulo_object_extend"));
}

#[test]
fn set_members_reach_machine_code() {
    let bytes = object_bytes(
        r#"
        fn main() {
            let mut s = Set::from_list([1, 2, 2])
            print(s.has(2))
            print(s.insert(3))
            print(s.remove(1))
            print(s.size)
            for v in s { print(v) }
        }
        "#,
    );
    assert!(references_symbol(&bytes, "xulo_set_from_list"));
    assert!(references_symbol(&bytes, "xulo_set_has"));
    assert!(references_symbol(&bytes, "xulo_set_insert"));
    assert!(references_symbol(&bytes, "xulo_set_remove"));
    assert!(references_symbol(&bytes, "xulo_array_len"));
}
