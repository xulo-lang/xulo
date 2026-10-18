use xulo_core::ast::Statement;
use xulo_parser::parse_program;

/// Attributes parse onto every declaration the grammar allows them on (spec
/// `grammar.md`): `Attribute = "#" Identifier [ "(" [ AttributeArgs ] ")" ]`.
#[test]
fn attributes_parse_on_every_attributed_declaration() {
    let program = parse_program(
        r#"
#Test fn t() {}
#Derive(ToString)
struct S { a: string }
#Derive(ToString)
enum E { A B }
#Known
trait Named { fn name(self): string }
#Known
impl Named for S { fn name(self): string { return self.a } }
"#,
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    let attrs_of = |i: usize| -> Vec<(String, Vec<String>, bool)> {
        let attrs = match &program.statements[i] {
            Statement::Test(f) => &f.attributes,
            Statement::Fn(f) => &f.attributes,
            Statement::Struct(s) => &s.attributes,
            Statement::Enum(e) => &e.attributes,
            Statement::Trait(t) => &t.attributes,
            Statement::Impl(im) => &im.attributes,
            other => panic!("unexpected statement {other:?}"),
        };
        attrs
            .iter()
            .map(|a| {
                (
                    a.name.clone(),
                    a.args.iter().map(|(n, _)| n.clone()).collect(),
                    a.has_parens,
                )
            })
            .collect()
    };
    // `#Test fn t()` is a `TestDecl`: the sigil is the declaration's own
    // form, not an attribute entry (spec `grammar.md` `TestDecl`).
    assert_eq!(attrs_of(0), Vec::<(String, Vec<String>, bool)>::new());
    assert_eq!(
        attrs_of(1),
        vec![("Derive".to_string(), vec!["ToString".to_string()], true)]
    );
    assert_eq!(
        attrs_of(2),
        vec![("Derive".to_string(), vec!["ToString".to_string()], true)]
    );
    assert_eq!(attrs_of(3), vec![("Known".to_string(), vec![], false)]);
    assert_eq!(attrs_of(4), vec![("Known".to_string(), vec![], false)]);
}

#[test]
fn derive_argument_lists_carry_every_name() {
    let program = parse_program("#Derive(ToString, Display)\nstruct S { a: string }")
        .unwrap_or_else(|e| panic!("{}", e.message));
    let Statement::Struct(s) = &program.statements[0] else {
        panic!("expected struct");
    };
    let args: Vec<&str> = s.attributes[0]
        .args
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(args, vec!["ToString", "Display"]);
    assert!(s.attributes[0].has_parens);
    assert_eq!(s.attributes[0].span, 0..26);
}

#[test]
fn empty_attribute_parens_are_recorded_as_parens() {
    let program = parse_program("#Derive()\nstruct S { a: string }")
        .unwrap_or_else(|e| panic!("{}", e.message));
    let Statement::Struct(s) = &program.statements[0] else {
        panic!("expected struct");
    };
    assert!(s.attributes[0].has_parens);
    assert!(s.attributes[0].args.is_empty());
}

#[test]
fn test_decl_without_attributes_is_rejected() {
    // The old `test fn` form is gone: `test` is an ordinary identifier and
    // only `#Test` marks a test function (spec `testing.md`).
    assert!(parse_program("test fn t() {}").is_err());
    assert!(parse_program("test async fn t() {}").is_err());
}

#[test]
fn test_decl_forms_parse_as_test_statements() {
    for src in ["#Test fn t() {}", "#Test async fn t() {}"] {
        let program = parse_program(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(
            matches!(&program.statements[0], Statement::Test(_)),
            "{src}"
        );
    }
}
