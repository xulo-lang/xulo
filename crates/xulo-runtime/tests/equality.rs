//! L13b / R9: `==` and `!=` are structural for every value-carrying type
//! (spec `expressions/operators.md`, "Equality"), not handle identity.

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

#[test]
fn aggregates_compare_structurally() {
    assert_eq!(
        out(r#"struct P { x: int, y: int }
enum E { A, B(int) }
fn main() {
    print([1, 2] == [1, 2])
    print([1, 2] != [1, 3])
    print([1, [2, 3]] == [1, [2, 3]])
    print((1, "a") == (1, "a"))
    print((1, "a") != (2, "a"))
    print(P(x: 1, y: 2) == P(x: 1, y: 2))
    print(P(x: 1, y: 2) == P(x: 2, y: 1))
    print(E::A == E::A)
    print(E::B(1) == E::B(1))
    print(E::B(1) == E::B(2))
}"#,),
        vec![
            "true", "true", "true", "true", "true", "true", "false", "true", "true", "false",
        ]
    );
}

#[test]
fn maps_and_sets_compare_by_content() {
    assert_eq!(
        out(r#"fn main() {
    let left: Map<string, int> = { a: 1, b: 2 }
    let right: Map<string, int> = { b: 2, a: 1 }
    print(left == right)
    print(left == { a: 1, b: 3 })
    let one = Set::from_list([1, 2, 3])
    let two = Set::from_list([3, 2, 1])
    print(one == two)
    print(one == Set::from_list([1, 2]))
}"#,),
        vec!["true", "false", "true", "false"]
    );
}

#[test]
fn cyclic_values_terminate() {
    assert_eq!(
        out(r#"fn main() {
    let mut a: List<unknown> = [1]
    a.push(a)
    let mut b: List<unknown> = [1]
    b.push(b)
    print(a == a)
    print(a == b)
}"#,),
        vec!["true", "true"]
    );
}
