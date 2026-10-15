use xulo_core::ast::Statement;
use xulo_parser::parse_program;

#[test]
fn test_fn_parses_with_fixed_shape() {
    let src = r#"
fn helper(): number { return 42 }

test fn basic_works() {
  let x = 1
  assert(x == 1, "x is 1")
}

test async fn async_test_works() {
  await Time.sleep(0)
}

fn main() {
  print(helper())
}
"#;
    let program = parse_program(src).unwrap();
    let tests: Vec<_> = program
        .statements
        .iter()
        .filter_map(|s| match s {
            Statement::Test(f) => Some((
                f.name.as_str(),
                f.is_async,
                f.params.is_empty(),
                f.return_type.is_none(),
                f.type_params.is_empty(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        tests,
        vec![
            ("basic_works", false, true, true, true),
            ("async_test_works", true, true, true, true),
        ]
    );
}

#[test]
fn test_keyword_is_not_an_identifier() {
    assert!(parse_program("fn main() { let test = 1 }").is_err());
    assert!(parse_program("fn test() {}").is_err());
}

#[test]
fn test_fn_rejects_parameters_return_pub_and_generics() {
    assert!(parse_program("test fn t(x: number) {}").is_err());
    assert!(parse_program("test fn t(): number { return 1 }").is_err());
    assert!(parse_program("pub test fn t() {}").is_err());
    assert!(parse_program("test fn t<T>() {}").is_err());
    // `test` precedes `async`, never the other way around
    assert!(parse_program("async test fn t() {}").is_err());
}

#[test]
fn test_fn_body_is_a_normal_block() {
    let src = r#"
test fn body_works() {
  let xs = [1, 2, 3]
  for x in xs {
    if x > 1 { print(x) }
  }
}
"#;
    let program = parse_program(src).unwrap();
    assert!(program
        .statements
        .iter()
        .any(|s| matches!(s, Statement::Test(f) if f.name == "body_works")));
}
