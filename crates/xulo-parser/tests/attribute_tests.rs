use xulo_core::ast::Statement;
use xulo_parser::parse_program;

/// Attributes parse onto every declaration the grammar allows them on (spec
/// `grammar.md`): `Attribute = "#" "[" AttributeName [ "(" [ AttributeArgs ]
/// ")" ] "]"`.
#[test]
fn attributes_parse_on_every_attributed_declaration() {
    let program = parse_program(
        r#"
#[test]
fn t() {}
#[derive(Debug)]
struct S { a: string }
#[derive(Debug)]
enum E { A B }
#[Known]
trait Named { fn name(self): string }
#[Known]
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
    // `#[test] fn t()` is a `TestDecl`: the marker is the declaration's own
    // form, not an attribute entry (spec `grammar.md` `TestDecl`).
    assert_eq!(attrs_of(0), Vec::<(String, Vec<String>, bool)>::new());
    assert_eq!(
        attrs_of(1),
        vec![("derive".to_string(), vec!["Debug".to_string()], true)]
    );
    assert_eq!(
        attrs_of(2),
        vec![("derive".to_string(), vec!["Debug".to_string()], true)]
    );
    assert_eq!(attrs_of(3), vec![("Known".to_string(), vec![], false)]);
    assert_eq!(attrs_of(4), vec![("Known".to_string(), vec![], false)]);
}

#[test]
fn derive_argument_lists_carry_every_name() {
    let program = parse_program("#[derive(Debug, Display)]\nstruct S { a: string }")
        .unwrap_or_else(|e| panic!("{}", e.message));
    let Statement::Struct(s) = &program.statements[0] else {
        panic!("expected struct");
    };
    let args: Vec<&str> = s.attributes[0]
        .args
        .iter()
        .map(|(n, _)| n.as_str())
        .collect();
    assert_eq!(args, vec!["Debug", "Display"]);
    assert!(s.attributes[0].has_parens);
    // The span covers `#` through the closing `]`.
    assert_eq!(s.attributes[0].span, 0..25);
}

#[test]
fn empty_attribute_parens_are_recorded_as_parens() {
    let program = parse_program("#[derive()]\nstruct S { a: string }")
        .unwrap_or_else(|e| panic!("{}", e.message));
    let Statement::Struct(s) = &program.statements[0] else {
        panic!("expected struct");
    };
    assert!(s.attributes[0].has_parens);
    assert!(s.attributes[0].args.is_empty());
}

#[test]
fn derive_and_macro_are_reserved_but_valid_attribute_names() {
    // `derive` and `macro` stay reserved words: they are not identifiers in
    // any ordinary position, and are accepted only between `#[` and `]`.
    assert!(parse_program("fn derive() {}").is_err());
    assert!(parse_program("fn main() { let macro = 1 }").is_err());
    for src in ["#[derive]\nstruct S { a: string }", "#[macro]\nfn m() {}"] {
        let program = parse_program(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        let attrs = match &program.statements[0] {
            Statement::Struct(s) => &s.attributes,
            Statement::Fn(f) => &f.attributes,
            other => panic!("unexpected statement {other:?}"),
        };
        assert_eq!(attrs[0].name, src[2..src.find(']').unwrap()].to_string());
    }
}

#[test]
fn old_hash_attribute_spelling_is_rejected() {
    // The old `#Name` spelling is a syntax error, never reinterpreted.
    for src in [
        "#Test\nfn t() {}",
        "#Derive(Debug)\nstruct S { a: string }",
        "#Inline\nfn f() {}",
        "#\nfn f() {}",
    ] {
        assert!(parse_program(src).is_err(), "{src}");
    }
}

#[test]
fn capitalised_bracket_names_parse_as_attribute_names() {
    // `#[Test]` and `#[Derive]` parse but are not registered names; the
    // checker reports `E0108`. The parser keeps no registry knowledge.
    let program = parse_program("#[Test]\nfn t() {}").unwrap_or_else(|e| panic!("{e}"));
    let Statement::Fn(f) = &program.statements[0] else {
        panic!("expected fn");
    };
    assert_eq!(f.attributes[0].name, "Test");
}

#[test]
fn test_decl_without_attributes_is_rejected() {
    // The old `test fn` form is gone: `test` is an ordinary identifier and
    // only `#[test]` marks a test function (spec `testing.md`).
    assert!(parse_program("test fn t() {}").is_err());
    assert!(parse_program("test async fn t() {}").is_err());
}

#[test]
fn test_decl_forms_parse_as_test_statements() {
    for src in ["#[test] fn t() {}", "#[test] async fn t() {}"] {
        let program = parse_program(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        assert!(
            matches!(&program.statements[0], Statement::Test(_)),
            "{src}"
        );
    }
}
