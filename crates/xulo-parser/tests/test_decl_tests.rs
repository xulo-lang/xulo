use xulo_core::ast::Statement;
use xulo_parser::parse_program;

#[test]
fn test_fn_parses_with_fixed_shape() {
    let src = r#"
fn helper(): number { return 42 }

#Test
fn basic_works() {
  let x = 1
  assert(x == 1, "x is 1")
}

#Test
async fn async_test_works() {
  await Time::sleep(0)
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
fn test_is_a_plain_identifier() {
    // `test` is not a keyword (spec lexical-structure keyword list): it binds
    // like any other word, and `#Test` alone marks a test function.
    assert!(parse_program("fn main() { let test = 1 }").is_ok());
    assert!(parse_program("fn test() {}").is_ok());
}

#[test]
fn test_fn_rejects_parameters_return_pub_and_generics() {
    // A well-formed `#Test` on a non-`test_decl` shape falls back to
    // `fn_decl` with the attribute attached (spec grammar: `TestDecl` is its
    // own production); the semantic pass rejects those with E0110 — the
    // parser keeps no registry knowledge.
    for src in [
        "#Test fn t(x: number) {}",
        "#Test fn t(): number { return 1 }",
        "#Test fn t<T>() {}",
    ] {
        let program = parse_program(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(
            matches!(&program.statements[0], Statement::Fn(f)
                if f.attributes.iter().any(|a| a.name == "Test")),
            "{src}"
        );
    }
    // Attribute order is fixed: the `#` comes before `pub`/`async`, so these
    // are parse errors outright.
    assert!(parse_program("pub #Test fn t() {}").is_err());
    // `#Test` precedes `async`, never the other way around.
    assert!(parse_program("async #Test fn t() {}").is_err());
}

#[test]
fn test_fn_body_is_a_normal_block() {
    let src = r#"
#Test
fn body_works() {
  let xs = [1, 2, 3]
  for x in xs {
    if x > 1 { print(x) }
  }
}
"#;
    let program = parse_program(src).unwrap();
    assert!(
        program
            .statements
            .iter()
            .any(|s| matches!(s, Statement::Test(f) if f.name == "body_works"))
    );
}

#[test]
fn test_decl_accepts_an_optional_string_title() {
    // spec `testing.md`: the parenthesised group carries a display title; an
    // empty group is the same as writing none, and the title decodes without
    // its quotes.
    let src = r#"
#Test("adds two numbers")
fn titled_works() {
  assert(1 + 1 == 2, "math")
}

#Test()
fn empty_group_works() {}

#Test
async fn async_titled() {}
"#;
    let program = parse_program(src).unwrap_or_else(|e| panic!("{e}"));
    let tests: Vec<_> = program
        .statements
        .iter()
        .filter_map(|s| match s {
            Statement::Test(f) => Some((f.name.as_str(), f.title.as_deref(), f.is_async)),
            _ => None,
        })
        .collect();
    assert_eq!(
        tests,
        vec![
            ("titled_works", Some("adds two numbers"), false),
            ("empty_group_works", None, false),
            ("async_titled", None, true),
        ]
    );
}

#[test]
fn test_decl_title_decodes_escapes() {
    let program =
        parse_program(r#"#Test("say \"hi\"") fn t() {}"#).unwrap_or_else(|e| panic!("{e}"));
    let Statement::Test(f) = &program.statements[0] else {
        panic!("expected a test declaration");
    };
    assert_eq!(f.title.as_deref(), Some(r#"say "hi""#));
}
