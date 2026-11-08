//! Macro syntax (spec `grammar.md`: `MacroCall`, `QuoteCall`, widened
//! `AttributeArg`, and the eight attribute hosts).

use xulo_core::ast::{Expression, QuotePiece, Statement};
use xulo_parser::parse_program;

fn expr(src: &str) -> Expression {
    let program = parse_program(&format!("fn f() {{ {src} }}"))
        .unwrap_or_else(|e| panic!("{src}: {}", e.message));
    let Statement::Fn(f) = &program.statements[0] else {
        panic!("expected fn");
    };
    let Statement::Expr(e) = &f.body.statements[0] else {
        panic!("expected expression statement");
    };
    e.expr.clone()
}

#[test]
fn function_macro_calls_parse() {
    let Expression::MacroCall(call) = expr("sql!(\"SELECT 1\")") else {
        panic!("expected macro call");
    };
    assert_eq!(call.path, ["sql"]);
    assert_eq!(call.args.len(), 1);

    let Expression::MacroCall(call) = expr("m::json!(value)") else {
        panic!("expected macro call");
    };
    assert_eq!(call.path, ["m", "json"]);
    assert_eq!(call.args.len(), 1);

    let Expression::MacroCall(call) = expr("build!()") else {
        panic!("expected macro call");
    };
    assert!(call.args.is_empty());

    // Whitespace does not change the choice, and `!=` never begins a macro.
    assert!(matches!(expr("a !(b)"), Expression::MacroCall(_)));
    assert!(matches!(expr("a != b"), Expression::BinaryOp(_)));
}

#[test]
fn quote_call_keeps_tokens_and_holes() {
    let Expression::Quote(q) = expr("ast::quote! { impl X for ${name} { } }") else {
        panic!("expected quote");
    };
    assert!(
        q.pieces
            .iter()
            .any(|p| matches!(p, QuotePiece::Hole { .. }))
    );
    let text: String = q
        .pieces
        .iter()
        .filter_map(|p| match p {
            QuotePiece::Tokens { text, .. } => Some(text.as_str()),
            QuotePiece::Hole { .. } => None,
        })
        .collect();
    assert!(text.contains("impl X for"));
    assert!(matches!(
        q.pieces.iter().find(|p| matches!(p, QuotePiece::Hole { .. })),
        Some(QuotePiece::Hole { expr: Expression::Identifier { name, .. }, .. }) if name == "name"
    ));
}

#[test]
fn quote_nested_braces_and_strings_are_balanced() {
    let Expression::Quote(q) = expr("ast::quote! { fn f() { print(\"}\") } }") else {
        panic!("expected quote");
    };
    let text: String = q
        .pieces
        .iter()
        .filter_map(|p| match p {
            QuotePiece::Tokens { text, .. } => Some(text.as_str()),
            QuotePiece::Hole { .. } => None,
        })
        .collect();
    assert!(text.contains("fn f()"));
    assert!(text.contains("\"}\""));
}

#[test]
fn attribute_values_and_named_arguments_parse() {
    let program = parse_program(
        "#[route(\"/v1/users\", 2)]\nfn get() {}\n#[macro_derive(Serialize, max_steps = 5)]\nfn m() {}",
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    let Statement::Fn(get) = &program.statements[0] else {
        panic!("expected fn");
    };
    let route = &get.attributes[0];
    assert_eq!(route.args.len(), 2);
    assert_eq!(route.args[0].name, None);
    assert!(matches!(
        route.args[0].value,
        xulo_core::ast::AttributeValue::String(ref s) if s == "/v1/users"
    ));
    let Statement::Fn(m) = &program.statements[1] else {
        panic!("expected fn");
    };
    let mac = &m.attributes[0];
    assert_eq!(mac.name, "macro_derive");
    assert_eq!(mac.args[0].name, None);
    assert!(matches!(
        mac.args[0].value,
        xulo_core::ast::AttributeValue::Identifier(ref name) if name == "Serialize"
    ));
    assert_eq!(mac.args[1].name.as_deref(), Some("max_steps"));
}

#[test]
fn attributes_parse_on_type_let_and_const() {
    let program = parse_program(
        "#[note]\ntype Id = int\n#[note]\nlet x = 1\n#[note]\nconst Y = 2\nfn main() {}",
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    let Statement::TypeAlias(t) = &program.statements[0] else {
        panic!("expected type alias");
    };
    assert_eq!(t.attributes[0].name, "note");
    let Statement::Let(l) = &program.statements[1] else {
        panic!("expected let");
    };
    assert_eq!(l.attributes[0].name, "note");
    let Statement::Let(c) = &program.statements[2] else {
        panic!("expected const");
    };
    assert!(c.is_const);
    assert_eq!(c.attributes[0].name, "note");
}

#[test]
fn definition_attributes_take_quota_keys() {
    let program = parse_program(
        "#[macro(max_steps = 1)]\nfn f() {}\n#[macro_attr(max_memory = 2)]\nfn g() {}\n#[macro_derive(Serialize, max_depth = 3)]\nfn h() {}",
    )
    .unwrap_or_else(|e| panic!("{}", e.message));
    let attrs: Vec<&xulo_core::ast::Attribute> = program
        .statements
        .iter()
        .map(|s| {
            let Statement::Fn(f) = s else {
                panic!("expected fn");
            };
            &f.attributes[0]
        })
        .collect();
    assert_eq!(attrs[0].name, "macro");
    assert_eq!(attrs[0].args[0].name.as_deref(), Some("max_steps"));
    assert_eq!(attrs[1].name, "macro_attr");
    assert_eq!(attrs[1].args[0].name.as_deref(), Some("max_memory"));
    assert_eq!(attrs[2].name, "macro_derive");
    assert_eq!(attrs[2].args[0].name, None);
    assert_eq!(attrs[2].args[1].name.as_deref(), Some("max_depth"));
}
