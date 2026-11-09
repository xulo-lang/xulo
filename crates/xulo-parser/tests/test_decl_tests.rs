use xulo_core::ast::Statement;
use xulo_parser::parse_program;

#[test]
fn test_fn_parses_with_fixed_shape() {
    let src = r#"
fn helper(): number { return 42 }

#[test]
fn basic_works() {
  let x = 1
  assert(x == 1, "x is 1")
}

#[test]
async fn async_test_works() {
  await std::time::sleep(0)
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
            Statement::Test(t) => Some((
                t.func.name.as_str(),
                t.func.is_async,
                t.func.params.is_empty(),
                t.func.return_type.is_none(),
                t.func.type_params.is_empty(),
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
    // like any other word, and `#[test]` alone marks a test function.
    assert!(parse_program("fn main() { let test = 1 }").is_ok());
    assert!(parse_program("fn test() {}").is_ok());
}

#[test]
fn test_fn_rejects_parameters_return_pub_and_generics() {
    // A `#[test]` on a non-`test_decl` shape falls back to `fn_decl` with
    // the marker attached (spec grammar: `TestDecl` is its own production);
    // the semantic pass rejects those with E0110 — the parser keeps no
    // registry knowledge.
    for src in [
        "#[test] fn t(x: number) {}",
        "#[test] fn t(): number { return 1 }",
        "#[test] fn t<T>() {}",
    ] {
        let program = parse_program(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(
            matches!(&program.statements[0], Statement::Fn(f)
                if f.attributes.iter().any(|a| a.name == "test")),
            "{src}"
        );
    }
    // Attribute order is fixed: `#[test]` comes before `pub`/`async`, so
    // these are parse errors outright.
    assert!(parse_program("pub #[test] fn t() {}").is_err());
    // `#[test]` precedes `async`, never the other way around.
    assert!(parse_program("async #[test] fn t() {}").is_err());
}

#[test]
fn test_fn_body_is_a_normal_block() {
    let src = r#"
#[test]
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
            .any(|s| matches!(s, Statement::Test(t) if t.func.name == "body_works"))
    );
}

#[test]
fn malformed_test_group_falls_back_to_an_attribute() {
    // A group that is not a `TestOptions` list does not form a `TestDecl`; it
    // falls back to `fn_decl` as an ordinary `#[test(...)]` attribute (a
    // checker `E0110`). This covers empty groups, positional values, unknown
    // option names, and wrong value types.
    for src in [
        "#[test()] fn t() {}",
        "#[test(name)] fn t() {}",
        r#"#[test("adds two numbers")] fn t() {}"#,
        "#[test(1)] fn t() {}",
        r#"#[test(category = "slow")] fn t() {}"#,
        "#[test(focus)] fn t() {}",
        r#"#[test(should_fail = "msg")] fn t() {}"#,
        r#"#[test(timeout = "100")] fn t() {}"#,
        "#[test(timeout = 1.5)] fn t() {}",
    ] {
        let program = parse_program(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(
            matches!(&program.statements[0], Statement::Fn(f)
                if f.attributes.iter().any(|a| a.name == "test" && a.has_parens)),
            "{src}"
        );
    }
    // A negative value is not grammatical under `AttributeValue`; it stays a
    // syntax error rather than becoming an `E0110`.
    assert!(parse_program("#[test(-1)] fn t() {}").is_err());
}

#[test]
fn test_options_form_a_test_decl() {
    use xulo_core::ast::{TestMode, TestOption};
    let src = r#"
#[test(should_fail)]
fn a() {}
#[test(should_panic)]
fn b() {}
#[test(should_panic = "division by zero")]
fn c() {}
#[test(skip)]
fn d() {}
#[test(skip = "not yet")]
fn e() {}
#[test(timeout = 100)]
fn f() {}
#[test(should_panic, timeout = 100)]
async fn g() {}
#[test(timeout = 5, should_fail,)]
fn h() {}
"#;
    let program = parse_program(src).unwrap();
    let tests: Vec<_> = program
        .statements
        .iter()
        .filter_map(|s| match s {
            Statement::Test(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(tests.len(), 8);

    assert_eq!(tests[0].mode(), TestMode::ShouldFail);
    assert_eq!(tests[1].mode(), TestMode::ShouldPanic);
    assert_eq!(tests[2].mode(), TestMode::ShouldPanic);
    assert_eq!(tests[2].expected_message(), Some("division by zero"));
    assert!(tests[3].is_skipped());
    assert_eq!(tests[3].skip_reason(), None);
    assert_eq!(tests[4].skip_reason(), Some("not yet"));
    assert_eq!(tests[5].timeout_ms(), Some(100));
    assert_eq!(tests[6].mode(), TestMode::ShouldPanic);
    assert_eq!(tests[6].timeout_ms(), Some(100));
    assert!(tests[6].func.is_async);
    assert_eq!(tests[7].mode(), TestMode::ShouldFail);
    assert_eq!(tests[7].timeout_ms(), Some(5));
    assert!(matches!(
        tests[7].options.first(),
        Some(TestOption::Timeout { value: 5, .. })
    ));
}

#[test]
fn syntactically_valid_but_invalid_groups_are_test_decls() {
    // These parse as `TestDecl` option lists; their combination rules are a
    // checking rule that reports `E0110` (see the semantic tests).
    for src in [
        "#[test(should_fail, should_panic)] fn t() {}",
        "#[test(timeout = 0)] fn t() {}",
        "#[test(timeout = 1, timeout = 2)] fn t() {}",
        "#[test(skip, timeout = 1)] fn t() {}",
        "#[test(skip, skip)] fn t() {}",
    ] {
        let program = parse_program(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(
            matches!(&program.statements[0], Statement::Test(_)),
            "{src}"
        );
    }
}

#[test]
fn old_test_attribute_spelling_is_rejected() {
    for src in [
        "#Test\nfn t() {}",
        "#Test()\nfn t() {}",
        r#"#Test("adds two numbers") fn t() {}"#,
    ] {
        assert!(parse_program(src).is_err(), "{src}");
    }
}
