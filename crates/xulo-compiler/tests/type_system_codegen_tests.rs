//! WP2 regression in the compiled path: the §6.2–6.5 probes must agree with
//! the interpreter (spec `types/primitive-types.md`,
//! `expressions/control-flow.md`, `types/composite-types.md`,
//! `types/generics.md`).

use std::path::Path;
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

/// §6.2: `unknown` accepts any value in parameter/return positions, list and
/// map element positions; the probe narrows before printing (E0212).
#[test]
fn unknown_top_type_compiles() {
    let out = compile_and_run(
        r#"fn show(v: unknown): unknown { return v }
fn main() {
    let v = show("x")
    if v is string { print(v) }
    let xs: List<unknown> = ["a", 1, true]
    print(xs.len)
}"#,
    );
    assert_eq!(out, vec!["x", "3"]);
}

/// §6.3: `is` narrows the then-branch, `is not` the else-branch.
#[test]
fn is_narrowing_compiles() {
    let out = compile_and_run(
        r#"fn f(v: int | string): int {
    if v is int { return v + 1 } else { return 0 }
}
fn g(v: int | string): int {
    if v is not int { return 0 } else { return v + 1 }
}
fn main() {
    print(f(41))
    print(f("x"))
    print(g(41))
    print(g("x"))
}"#,
    );
    assert_eq!(out, vec!["42", "0", "42", "0"]);
}

/// §6.4: tuple positional read and write.
#[test]
fn tuple_positional_access_compiles() {
    let out = compile_and_run(
        r#"fn main() {
    let p: (int, string) = (10, "ten")
    print(p.0)
    print(p.1)
    let mut r = (1, 2)
    r.0 = 5
    print(r.0)
    print(r.1)
}"#,
    );
    assert_eq!(out, vec!["10", "ten", "5", "2"]);
}

/// §6.5: generic struct construction infers the type arguments.
#[test]
fn generic_struct_construction_compiles() {
    let out = compile_and_run(
        r#"struct Pair<A, B> { first: A, second: B }
fn main() {
    let p = Pair(first: 1, second: "x")
    print(p.first)
    print(p.second)
}"#,
    );
    assert_eq!(out, vec!["1", "x"]);
}

/// §6.6 / C2: `Map<K, V>` members and iteration in the compiled path.
#[test]
fn map_members_compile() {
    let out = compile_and_run(
        r#"fn main() {
    let mut m: Map<string, int> = { a: 1 }
    m.set("b", 2)
    print(m.get("a"))
    print(m.get("z"))
    print(m.has("b"))
    print(m.size)
    print(m.remove("a"))
    print(m.size)
}"#,
    );
    assert_eq!(out, vec!["1", "null", "true", "2", "1", "1"]);
}

#[test]
fn map_projection_and_iteration_compile() {
    let out = compile_and_run(
        r#"fn main() {
    let m: Map<string, int> = { a: 1, b: 2 }
    print(m.keys())
    print(m.values())
    print(m.entries())
    for k in m { print(k) }
}"#,
    );
    assert_eq!(out, vec!["[a, b]", "[1, 2]", "[[a, 1], [b, 2]]", "a", "b"]);
}

#[test]
fn map_from_entries_and_typed_literal_compile() {
    let out = compile_and_run(
        r#"fn main() {
    let m = Map::from_entries([("a", 1), ("a", 2)])
    print(m.get("a"))
    print(m.size)
    let ages = Map<int, string>{ 30: "thirty" }
    print(ages.get(30))
    print(ages.keys())
}"#,
    );
    assert_eq!(out, vec!["2", "1", "thirty", "[30]"]);
}

/// §6.6 / C3: `Set<T>` members and iteration in the compiled path.
#[test]
fn set_members_compile() {
    let out = compile_and_run(
        r#"fn main() {
    let mut s = Set::from_list([1, 2, 2])
    print(s.size)
    print(s.has(2))
    print(s.insert(2))
    print(s.insert(3))
    print(s.remove(1))
    print(s.size)
    for v in s { print(v) }
}"#,
    );
    assert_eq!(
        out,
        vec!["2", "true", "false", "true", "true", "2", "2", "3"]
    );
}

#[test]
fn set_enum_values_compile() {
    let out = compile_and_run(
        r#"enum Color { Red, Green }
fn main() {
    let mut s = Set::from_list([Color::Red, Color::Green, Color::Red])
    print(s.size)
    print(s.has(Color::Red))
    print(s.insert(Color::Green))
    print(s.remove(Color::Red))
    print(s.size)
}"#,
    );
    assert_eq!(out, vec!["2", "true", "false", "true", "1"]);
}

#[test]
fn set_string_values_compile() {
    let out = compile_and_run(
        r#"fn main() {
    let mut s = Set::from_list(["a", "b", "a"])
    print(s.size)
    print(s.has("b"))
    print(s.remove("b"))
    print(s.has("b"))
    for v in s { print(v) }
}"#,
    );
    assert_eq!(out, vec!["2", "true", "true", "false", "a"]);
}

/// C1: list and object spread preserve order and later overrides.
#[test]
fn list_and_object_spread_compile() {
    let out = compile_and_run(
        r#"fn main() {
    let head = [1, 2]
    let tail = [3, 4]
    let all = [...head, ...tail]
    let with_extra = [...all, 9]
    print(all)
    print(with_extra)
    let base = { a: 1, b: 2 }
    let merged = { ...base, a: 3, c: 4 }
    print(merged.a)
    print(merged.b)
    print(merged.c)
}"#,
    );
    assert_eq!(out, vec!["[1, 2, 3, 4]", "[1, 2, 3, 4, 9]", "3", "2", "4"]);
}

#[test]
fn map_optional_get_keeps_runtime_tag() {
    let out = compile_and_run(
        r#"fn main() {
    let m: Map<string, string> = { a: "x" }
    print(m.get("a"))
    print(m.get("z"))
    let hit = m.get("a")
    let miss = m.get("z")
    print(hit)
    print(miss)
}"#,
    );
    assert_eq!(out, vec!["x", "null", "x", "null"]);
}
