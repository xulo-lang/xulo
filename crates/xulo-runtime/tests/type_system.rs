//! WP2 regression: the type-system convergence probes in
//! `docs/stage1-readiness.md` §6.2–6.5 (spec `types/primitive-types.md`,
//! `expressions/control-flow.md`, `types/composite-types.md`,
//! `types/generics.md`).

use xulo_core::error::XuloError;
use xulo_parser::parse_program;
use xulo_runtime::interpreter::Interpreter;

fn run(src: &str) -> Result<Vec<String>, XuloError> {
    let program = parse_program(src)?;
    xulo_semantic::analyze(&program)?;
    Interpreter::new().run(&program)
}

fn out(src: &str) -> Vec<String> {
    run(src).unwrap_or_else(|e| panic!("{}", e.message))
}

fn error(src: &str) -> XuloError {
    match run(src) {
        Ok(lines) => panic!("expected an error, got {lines:?}"),
        Err(e) => e,
    }
}

/// §6.2: `unknown` is the top type, so any value fits an `unknown`
/// parameter/return position, `List<unknown>`, and `Map<string, unknown>`.
/// `print` still rejects `unknown` itself (E0212), so the probe narrows first.
#[test]
fn unknown_is_the_top_type() {
    assert_eq!(
        out("fn show(v: unknown): unknown { return v }
             fn main() {
                 let v = show(\"x\")
                 if v is string { print(v) }
                 let xs: List<unknown> = [\"a\", 1, true]
                 print(xs.len)
                 let m: Map<string, unknown> = { a: 1, b: \"x\" }
                 print(m.size)
             }"),
        vec!["x", "3", "2"]
    );
}

/// §6.3: `if v is T` narrows the then-branch, `is not T` the else-branch,
/// and a union loses the covered member in the other branch.
#[test]
fn is_narrowing_covers_both_branches() {
    assert_eq!(
        out("fn f(v: int | string): int {
                 if v is int { return v + 1 } else { return 0 }
             }
             fn g(v: int | string): int {
                 if v is not int { return 0 } else { return v + 1 }
             }
             fn main() {
                 print(f(41))
                 print(f(\"x\"))
                 print(g(41))
                 print(g(\"x\"))
             }"),
        vec!["42", "0", "42", "0"]
    );
}

/// L18b: an assignment inside a narrowed branch ends the narrowing, so the
/// branch may write a value of another union member and the next test sees the
/// declared type again (spec `expressions/control-flow.md`).
#[test]
fn assignment_ends_chronological_narrowing() {
    assert_eq!(
        out("fn f(initial: int | string): string {
                 let mut v: int | string = initial
                 if v is int {
                     v = \"changed\"
                 }
                 if v is string {
                     return v
                 }
                 return \"other\"
             }
             fn main() {
                 print(f(1))
                 print(f(\"already\"))
             }"),
        vec!["changed", "already"]
    );
}

/// The right operand must be testable and fit the subject (`E0222`).
#[test]
fn untestable_and_mismatched_types_are_e0222() {
    let e = error("fn main() { let x: string = \"a\"; print(x is int) }");
    assert_eq!(e.code, Some("E0222"), "{}", e.message);
    let e = error("fn main() { let xs: List<int> = [1]; print(xs is List<int>) }");
    assert_eq!(e.code, Some("E0222"), "{}", e.message);
}

/// §6.4: tuple positional access reads `p.i` and writes through `let mut`.
#[test]
fn tuple_positional_access_reads_and_writes() {
    assert_eq!(
        out("fn main() {
                 let p: (int, string) = (10, \"ten\")
                 print(p.0)
                 print(p.1)
                 let mut r = (1, 2)
                 r.0 = 5
                 print(r.0)
                 print(r.1)
             }"),
        vec!["10", "ten", "5", "2"]
    );
}

/// A position outside the tuple's arity is `E0220`.
#[test]
fn tuple_position_out_of_range_is_e0220() {
    let e = error(
        "fn main() {
             let p: (int, string) = (10, \"ten\")
             print(p.2)
         }",
    );
    assert_eq!(e.code, Some("E0220"), "{}", e.message);
}

/// §6.5: a generic struct constructor infers its type arguments from the
/// named field arguments, and member access substitutes them.
#[test]
fn generic_struct_construction_infers_type_arguments() {
    assert_eq!(
        out("struct Pair<A, B> { first: A, second: B }
             fn main() {
                 let p = Pair(first: 1, second: \"x\")
                 print(p.first)
                 print(p.second)
             }"),
        vec!["1", "x"]
    );
}

/// T12: `f32` is binary32 — the binding rounds its literal and every
/// arithmetic result stays in f32, while `f64` keeps the double's digits.
#[test]
fn f32_rounds_at_binding_and_arithmetic() {
    assert_eq!(
        out("struct Holder { value: f32 }
             fn main() {
                 let narrow: f32 = 0.8
                 print(narrow)
                 let one: f32 = 1.0
                 let three: f32 = 3.0
                 print(one / three)
                 print(narrow * 4.0)
                 let wide: f64 = 0.8
                 print(wide)
                 let mut holder = Holder(value: 0.8)
                 print(holder.value)
                 holder.value = 0.3
                 print(holder.value)
             }"),
        vec![
            "0.800000011920929",
            "0.3333333432674408",
            "3.200000047683716",
            "0.8",
            "0.800000011920929",
            "0.30000001192092896",
        ]
    );
}
