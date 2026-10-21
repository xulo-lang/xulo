//! Trait 派发（`Trait::method(recv)` 与 `str(v)`）的 codegen 回归：
//! 语义层把调用标注成派发名，AOT 须静态解析到 impl 函数；派生
//! `#Derive(ToString)` 须生成与解释器一致的渲染。JIT 输出逐行对齐解释器。
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

/// `Trait::method(recv)` 静态派发：接收者是第一个实参，返回类型按目标
/// impl 查表（小数不能被当成整数位模式打印）。
#[test]
fn explicit_trait_dispatch_matches_interpreter() {
    let out = compile_and_run(include_str!("../../../examples/trait_shapes.xulo"));
    assert_eq!(
        out,
        lines(&[
            "rect area=12",
            "rect perimeter=14",
            "circle area=12.56636",
            "circle perimeter=12.56636",
        ])
    );
}

/// 手写 `impl ToString for T`：`str(v)`、模板与 `Trait::method` 三条派发
/// 路径与成员调用 `v.toString()` 都走同一个实现。
#[test]
fn handwritten_tostring_dispatch_matches_interpreter() {
    let out = compile_and_run(
        r#"
struct Point { x: number, y: number }
impl ToString for Point {
    fn toString(self): string { "Point(" + str(self.x) + ", " + str(self.y) + ")" }
}
fn main() {
    let p = Point(x: 1.5, y: 2)
    print(str(p))
    print(p.toString())
    print(ToString::toString(p))
    print(`${p}`)
}
"#,
    );
    assert_eq!(out, lines(&["Point(1.5, 2)"; 4]));
}

/// `#Derive(ToString)` struct：字段按声明序 `Type(name: v, …)`，小数字段
/// 按对象里存的真实标签渲染，可选字段缺省为 null，Named 字段递归走派生。
#[test]
fn derived_struct_tostring_matches_interpreter() {
    let out = compile_and_run(
        r#"
#Derive(ToString)
struct P { x: number }
#Derive(ToString)
struct User { name: string, age: number, nick: string?, home: P }
fn main() {
    let u = User(name: "lyy", age: 30.5, home: P(x: 1))
    print(str(u))
    print(`${u}`)
    print(u.toString())
}
"#,
    );
    assert_eq!(
        out,
        lines(&["User(name: lyy, age: 30.5, nick: null, home: P(x: 1))"; 3])
    );
}

/// `#Derive(ToString)` enum：`Type::Variant` 与 `Type::Variant(p0, …)`，
/// payload 按各自声明类型（Named 递归）渲染。
#[test]
fn derived_enum_tostring_matches_interpreter() {
    let out = compile_and_run(
        r#"
#Derive(ToString)
struct P { x: number }
#Derive(ToString)
enum Shape { Dot, Circle(number), Pair(string, number), Named(P) }
fn main() {
    print(str(Shape::Dot))
    print(str(Shape::Circle(3.5)))
    print(str(Shape::Pair("a", 2)))
    print(str(Shape::Named(P(x: 9))))
    print(`${Shape::Circle(4)}`)
}
"#,
    );
    assert_eq!(
        out,
        lines(&[
            "Shape::Dot",
            "Shape::Circle(3.5)",
            "Shape::Pair(a, 2)",
            "Shape::Named(P(x: 9))",
            "Shape::Circle(4)",
        ])
    );
}

/// AOT 对象文件引用新增的字段标签读取符号（派生渲染依赖它）
#[test]
fn aot_object_references_object_get_tag() {
    let bytes = object_bytes(
        r#"
#Derive(ToString)
struct P { x: number }
fn main() { print(str(P(x: 1))) }
"#,
    );
    assert!(
        references_symbol(&bytes, "xulo_object_get_tag"),
        "object missing xulo_object_get_tag"
    );
}

/// 泛型参数上的派发是运行时决议（`dyn:` 标注）：AOT 目前不支持，
/// 必须给出明确错误而不是「undefined function」。
#[test]
fn generic_trait_dispatch_rejected_with_clear_error() {
    let err = compile_to_ir(
        r#"
trait Area { fn area(self): number }
struct Rect { w: number }
impl Area for Rect { fn area(self): number { self.w } }
fn measure<T: Area>(x: T): number { return Area::area(x) }
fn main() { print(measure(Rect(w: 2))) }
"#,
        Path::new("test.xulo"),
    )
    .expect_err("dynamic dispatch through a generic parameter must be rejected");
    assert!(
        err.message.contains("dynamic trait dispatch"),
        "unexpected error: {}",
        err.message
    );
}
