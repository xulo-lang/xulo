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

/// L15: `Array<T, N>.len` uses the array header in the AOT path.
#[test]
fn fixed_array_len_compile() {
    let out = compile_and_run(
        r#"fn count(items: Array<int, 3>): number {
    items.len
}
fn main() {
    let numbers: Array<int, 3> = [1, 2, 3]
    let words: Array<string, 2> = ["a", "b"]
    print(numbers.len)
    print(words.len)
    print(numbers[1])
    print(count(numbers))
}"#,
    );
    assert_eq!(out, vec!["3", "2", "2", "3"]);
}

/// R9b: right-nested ternaries evaluate exactly one branch and print strings.
#[test]
fn nested_ternary_compile() {
    let out = compile_and_run(
        r#"fn mark(name: string): string {
    print(name)
    name
}
fn main() {
    let score = 85
    let grade = score >= 90 ? "A" : score >= 60 ? "B" : "C"
    print(grade)
    let low = 40
    print(low >= 90 ? "A" : low >= 60 ? "B" : "C")
    let pass = true
    print(pass ? "yes" : "no")
    print(true ? mark("then") : mark("else"))
    print(false ? mark("then") : mark("else"))
}"#,
    );
    assert_eq!(out, vec!["B", "C", "yes", "then", "then", "else", "else"]);
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

/// T4: a generic return keeps the argument's static tag, so `identity(true)`
/// prints `true` instead of the integer `1`.
#[test]
fn generic_return_keeps_bool_tag() {
    let out = compile_and_run(
        r#"fn identity<T>(value: T): T { value }
fn first<T>(list: List<T>): T { list[0] }
fn main() {
    print(identity(true))
    print(identity("x"))
    print(first(["hi"]))
    print(first([true]))
}"#,
    );
    assert_eq!(out, vec!["true", "x", "hi", "true"]);
}

/// L13b / R9: `==` and `!=` lower to the runtime's structural comparison for
/// lists, tuples, structs, maps, sets, and enums instead of pointer identity.
#[test]
fn structural_equality_compiles() {
    let out = compile_and_run(
        r#"struct P { x: int, y: int }
enum E { A, B(int) }
fn main() {
    print([1, 2] == [1, 2])
    print([1, 2] != [1, 3])
    print((1, "a") == (1, "a"))
    print(P(x: 1, y: 2) == P(x: 1, y: 2))
    print(P(x: 1, y: 2) == P(x: 2, y: 1))
    let left: Map<string, int> = { a: 1, b: 2 }
    let right: Map<string, int> = { b: 2, a: 1 }
    print(left == right)
    print(Set::from_list([1, 2, 3]) == Set::from_list([3, 2, 1]))
    print(E::A == E::A)
    print(E::B(1) == E::B(1))
    print(E::B(1) == E::B(2))
}"#,
    );
    assert_eq!(
        out,
        vec![
            "true", "true", "true", "true", "false", "true", "true", "true", "true", "false",
        ]
    );
}

/// L6: an unannotated lambda passed to a generic callback infers the
/// callback's type parameter from the other arguments instead of binding it
/// to the lambda's provisional `any`.
#[test]
fn generic_closure_argument_compiles() {
    let out = compile_and_run(
        r#"fn apply<T>(f: fn(T): T, x: T): T { f(x) }
fn main() {
    print(apply(fn(v) { v * 2 }, 21))
    print(apply(fn(v) { v + "!" }, "hi"))
}"#,
    );
    assert_eq!(out, vec!["42", "hi!"]);
}

/// C4: `Array::filled<T, N>` lowers to a runtime fill in the compiled path,
/// matching the interpreter's length, elements, and `.len`.
#[test]
fn array_filled_compiles() {
    let out = compile_and_run(
        r#"fn main() {
    let a = Array::filled<int, 4>(7)
    print(a)
    print(a.len)
    let words = Array::filled<string, 2>("x")
    print(words)
}"#,
    );
    assert_eq!(out, vec!["[7, 7, 7, 7]", "4", "[x, x]"]);
}
