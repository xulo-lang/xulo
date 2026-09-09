use xulo_parser_pest::{parse, print_tree};

#[test]
fn test_parse_let() {
    let pairs = parse("let x = 42").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_if_else() {
    let pairs = parse("fn f() { if true { print(1) } else { print(0) } }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_for_loop() {
    let pairs = parse("fn f() { for i in [1, 2, 3] { print(i) } }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_while_loop() {
    let pairs = parse("fn f() { while true { break } }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_function_def() {
    let src = r#"fn add(a: number, b: number): number { return a + b }"#;
    let pairs = parse(src).unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_match() {
    let pairs = parse("fn f() { match x { 1 => \"one\", _ => \"other\" } }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_binary_expr_in_let() {
    let pairs = parse("let x = 1 + 2 * 3").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_string_literal() {
    let pairs = parse(r#"let s = "hello""#).unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_error() {
    let result = parse("let = ");
    assert!(result.is_err());
}

#[test]
fn test_parse_complex() {
    let src = r#"
        import { foo, bar } from "module"

        type ID = number

        enum Color { Red, Green, Blue }

        fn main(): number {
            let x = 42
            if x > 0 {
                print("positive")
            } else {
                print("non-positive")
            }
            return x
        }
    "#;
    let pairs = parse(src).unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_ui_component() {
    let src = r#"
        fn view() {
            Button {
                label: "Click me"
                onClick: fn() { print("clicked") }
            }
        }
    "#;
    let pairs = parse(src).unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_try_catch() {
    let pairs = parse("fn f() { try { throw \"error\" } catch (e) { print(e) } }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_closure() {
    let pairs = parse("let add = fn(a: number, b: number) { a + b }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_index_access() {
    let pairs = parse("let x = arr[0]").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_method_call() {
    let pairs = parse("let x = obj.method(arg)").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_enum() {
    let pairs = parse("enum Direction { North, South, East, West }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_type_alias() {
    let pairs = parse("type StringOrNumber = string | number").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_return() {
    let pairs = parse("fn f() { return 42 }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_throw() {
    let pairs = parse("fn f() { throw \"error\" }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_ternary() {
    let pairs = parse("fn f() { let x = if true { 1 } else { 0 } }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_const() {
    let pairs = parse("const PI = 3.14").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_async_fn() {
    let pairs = parse("async fn fetch(): string { return \"data\" }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_multiple_statements() {
    let src = r#"
        fn f() {
            let x = 1
            let y = 2
            print(x + y)
        }
    "#;
    let pairs = parse(src).unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_nested_expressions() {
    let pairs = parse("let x = (1 + 2) * 3").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_list_literal() {
    let pairs = parse("let xs = [1, 2, 3]").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_object_literal() {
    let pairs = parse("let obj = { x: 1, y: 2 }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_import() {
    let pairs = parse(r#"import { foo } from "module""#).unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_trait() {
    let pairs = parse("trait Drawable { fn draw(self): void; }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_parse_impl() {
    let pairs = parse("impl Drawable for Circle { fn draw(self): void { } }").unwrap();
    assert!(!pairs.is_empty());
}

#[test]
fn test_print_tree() {
    let src = r#"
        fn main() {
            let x = 42
            print(x)
        }
    "#;
    let pairs = parse(src).unwrap();
    print_tree(&pairs, 0);
}
