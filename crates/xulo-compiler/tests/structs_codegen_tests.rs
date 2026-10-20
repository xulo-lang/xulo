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

/// examples/structs.xulo 全文：AOT 编译路径的输出必须与解释器逐行一致
const STRUCTS_EXAMPLE: &str = r#"
struct Point { x: number, y: number }

impl Point {
    fn len(self): number {
        std::math::sqrt(self.x * self.x + self.y * self.y)
    }
    fn shifted(self, dx: number, dy: number): Point {
        return Point(x: self.x + dx, y: self.y + dy)
    }
    fn show(self): string {
        return `(${self.x}, ${self.y})`
    }
}

#Derive(ToString)
struct User { name: string, age: number, nick: string? }

fn describe(u: User): string {
    return `${u.name}${u.nick ?? ""} (${u.age})`
}

fn main() {
    let p = Point(x: 3, y: 4)
    print(p.x, p.y)
    print(p.len())
    print(p.shifted(1, 2).show())

    let u = User(name: "lyy", age: 30)
    print(describe(u))
    print(u.nick ?? "no nick")
    print(u)

    let mut q = Point(x: 0, y: 0)
    q.x = 5
    print(q.show())

    let checked: Point = { x: 1, y: 2 }
    print(checked.len())
}
"#;

#[test]
fn test_structs_example_matches_interpreter() {
    let out = compile_and_run(STRUCTS_EXAMPLE);
    let expected: Vec<String> = [
        "3 4",
        "5",
        "(4, 6)",
        "lyy (30)",
        "no nick",
        "{ name: lyy, age: 30, nick: null }",
        "(5, 0)",
        "2.23606797749979",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(out, expected);
}

#[test]
fn test_str_zero_is_not_null() {
    // 旧 xulo_to_string 把整数 0 当成 null（tag-blind）
    let out = compile_and_run(r#"fn main() { print(str(0)); print(`n=${0}`) }"#);
    assert_eq!(out, vec!["0".to_string(), "n=0".to_string()]);
}

#[test]
fn test_str_null_rejected_by_semantics() {
    // 语义层拒绝 str(null)（E0212），irgen 不会为其生成 tag=6 的 Stringify
    let err = compile_to_ir(r#"fn main() { print(str(null)) }"#, Path::new("test.xulo"))
        .expect_err("str(null) must be rejected");
    assert!(
        err.message.contains("null"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn test_multi_arg_print_joins_with_space() {
    let out = compile_and_run(r#"fn main() { print("a", 1, true) }"#);
    assert_eq!(out, vec!["a 1 true".to_string()]);
}

#[test]
fn test_nullish_coalesce() {
    let out = compile_and_run(r#"fn main() { print(null ?? "fb"); print(7 ?? 9) }"#);
    assert_eq!(out, vec!["fb".to_string(), "7".to_string()]);
}

#[test]
fn test_method_dispatch_with_self() {
    let out = compile_and_run(
        r#"
struct Point { x: number, y: number }
impl Point {
    fn shifted(self, dx: number): Point {
        return Point(x: self.x + dx, y: self.y)
    }
    fn show(self): string {
        return `(${self.x}, ${self.y})`
    }
}
fn main() {
    let p = Point(x: 1, y: 2)
    print(p.shifted(4).show())
}
"#,
    );
    assert_eq!(out, vec!["(5, 2)".to_string()]);
}

#[test]
fn test_struct_ctor_optional_field_defaults_to_null() {
    // 注：可选字段为 null 时裸打印（无 ?? ）在 AOT 下无法与 int 0 区分
    // （xulo_detect_tag 的已知限制），structs.xulo 全部经由 ?? 兜底。
    let out = compile_and_run(
        r#"
struct Tag { name: string, note: string? }
fn main() {
    let t = Tag(name: "a")
    print(t.note ?? "-")
    print(`${t.name}${t.note ?? ""}`)
}
"#,
    );
    assert_eq!(out, vec!["-".to_string(), "a".to_string()]);
}

#[test]
fn test_sqrt_tagged_as_float() {
    let out =
        compile_and_run(r#"fn main() { print(std::math::sqrt(4)); print(std::math::sqrt(5)) }"#);
    assert_eq!(out, vec!["2".to_string(), "2.23606797749979".to_string()]);
}

#[test]
fn test_structs_aot_object_references_runtime() {
    // AOT .o 必须引用带标签字符串路径（str()/多参数 print），而非旧的 tag-blind 接口
    let bytes = object_bytes(STRUCTS_EXAMPLE);
    assert!(
        references_symbol(&bytes, "xulo_stringify"),
        "structs example must lower str()/print args through xulo_stringify"
    );
    assert!(
        references_symbol(&bytes, "xulo_object_get"),
        "structs example must read struct fields through xulo_object_get"
    );
    assert!(
        !references_symbol(&bytes, "xulo_to_string"),
        "tag-blind xulo_to_string must no longer be referenced"
    );
}
