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
    // `Map`/`Set` are a separate AOT gap (§6.6, WP3); the semantic shape is
    // covered by the interpreter test.
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
