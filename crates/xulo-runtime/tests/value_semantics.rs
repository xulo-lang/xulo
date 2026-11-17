//! R8: plain binding and assignment follow the value/reference table in
//! `spec/memory-and-runtime.md` — `struct`, `enum`, `Array`, and tuple values
//! get their own container, while `List`, `Map`, `Set`, and `StringBuilder`
//! keep sharing the one underlying value.

use xulo_parser::parse_program;
use xulo_runtime::interpreter::Interpreter;

fn run_ok(src: &str) -> Vec<String> {
    let program = parse_program(src).unwrap();
    xulo_semantic::analyze(&program).unwrap_or_else(|e| panic!("{}", e.message));
    let interp = Interpreter::new();
    interp
        .run(&program)
        .unwrap_or_else(|e| panic!("{}", e.message))
}

#[test]
fn let_binding_copies_a_struct() {
    let out = run_ok(
        r#"
        struct Point { x: int, y: int }
        fn main() {
            let a = Point(x: 1, y: 2)
            let mut b = a
            b.x = 9
            print(a.x)
            print(b.x)
        }
        "#,
    );
    assert_eq!(out, vec!["1", "9"]);
}

#[test]
fn assignment_copies_a_struct() {
    let out = run_ok(
        r#"
        struct Point { x: int, y: int }
        fn main() {
            let a = Point(x: 1, y: 2)
            let mut b = Point(x: 0, y: 0)
            b = a
            b.x = 7
            print(a.x)
            print(b.x)
        }
        "#,
    );
    assert_eq!(out, vec!["1", "7"]);
}

#[test]
fn nested_struct_fields_copy_by_category() {
    let out = run_ok(
        r#"
        struct Inner { n: int }
        struct Outer { inner: Inner, items: List<int> }
        fn main() {
            let inner = Inner(n: 1)
            let list = [1, 2]
            let a = Outer(inner: inner, items: list)
            let mut b = a
            b.inner.n = 9
            b.items[0] = 8
            print(a.inner.n)
            print(b.inner.n)
            print(a.items[0])
        }
        "#,
    );
    // A nested struct field is value category; a List field stays shared.
    assert_eq!(out, vec!["1", "9", "8"]);
}

#[test]
fn list_and_map_bindings_stay_shared() {
    let out = run_ok(
        r#"
        fn main() {
            let xs = [1, 2]
            let mut ys = xs
            ys[0] = 9
            print(xs[0])

            let m = { a: 1 }
            let mut n = m
            n.a = 7
            print(m.a)
        }
        "#,
    );
    assert_eq!(out, vec!["9", "7"]);
}

#[test]
fn tuple_and_array_bindings_copy() {
    let out = run_ok(
        r#"
        fn main() {
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
        }
        "#,
    );
    assert_eq!(out, vec!["1", "9", "1", "8"]);
}

#[test]
fn function_parameters_borrow_a_struct() {
    let out = run_ok(
        r#"
        struct Point { x: int }
        fn bump(p: mut Point) {
            p.x = 9
        }
        fn main() {
            let mut a = Point(x: 1)
            bump(a)
            print(a.x)
        }
        "#,
    );
    // Parameters borrow, so the callee mutates the caller's value.
    assert_eq!(out, vec!["9"]);
}

#[test]
fn destructuring_copies_struct_elements() {
    let out = run_ok(
        r#"
        struct Point { x: int }
        fn main() {
            let a = Point(x: 1)
            let (p1, p2) = (a, a)
            let mut r = p1
            r.x = 9
            print(a.x)
            print(p1.x)
            print(p2.x)
            print(r.x)

            let obj = { p3: a }
            let { p3 } = obj
            let mut mutation = p3
            mutation.x = 7
            print(a.x)
        }
        "#,
    );
    assert_eq!(out, vec!["1", "1", "1", "9", "1"]);
}
