//! Call-site type-argument lists (`first<Int>(…)`) — the `CallTypeArgs`
//! production, its PEG disambiguation against `<` as the relational operator,
//! and the nested-bracket spacing diagnostic (spec `grammar.md`,
//! `expressions/calls.md`).

use xulo_core::ast::{Call, Expression, Statement, Type};
use xulo_parser::parse_program;

fn parse_ok(src: &str) -> xulo_core::ast::Program {
    parse_program(src).unwrap_or_else(|e| panic!("expected OK for {src:?}: {}", e.message))
}

fn parse_err(src: &str) -> xulo_core::error::XuloError {
    parse_program(src).expect_err(&format!("expected error for {src:?}"))
}

/// The `Call` of the first expression statement in `fn main() { … }`.
fn main_call(src: &str) -> Call {
    let p = parse_ok(src);
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn, got {:?}", p.statements[0]);
    };
    let Statement::Expr(es) = &f.body.statements[0] else {
        panic!("expected expr statement, got {:?}", f.body.statements[0]);
    };
    match &es.expr {
        Expression::Call(c) => c.clone(),
        other => panic!("expected call, got {other:?}"),
    }
}

#[test]
fn written_type_args_parse_onto_the_call() {
    let call = main_call("fn main() { first<number>([1]) }");
    assert_eq!(call.callee, "first");
    assert_eq!(call.type_args, Some(vec![Type::Number]));
    assert_eq!(call.arguments.len(), 1);
}

#[test]
fn nested_list_type_args_need_the_space_between_closers() {
    let call = main_call("fn main() { first<List<number> >([1]) }");
    assert_eq!(
        call.type_args,
        Some(vec![Type::List(Box::new(Type::Number))])
    );
}

#[test]
fn empty_list_is_a_zero_length_type_argument_list() {
    // `f<>(…)` must reach the arity diagnostic, never a comparison
    // (spec `expressions/calls.md`).
    let call = main_call("fn main() { f<>(1) }");
    assert_eq!(call.type_args, Some(Vec::<Type>::new()));
}

#[test]
fn spaces_around_the_list_are_insignificant() {
    let call = main_call("fn main() { first< number >([1]) }");
    assert_eq!(call.type_args, Some(vec![Type::Number]));
}

#[test]
fn trailing_content_after_close_falls_back_to_comparison() {
    // Condition 2 of the disambiguation: the token after `>` must be `(`.
    // Without it the `<` is relational — and two relational operators are
    // non-associative, so the comparison reading fails to parse.
    let err = parse_err("fn main() { let x = a < number > [1] }");
    assert!(matches!(err.kind, xulo_core::error::ErrorKind::Parse));
}

#[test]
fn two_comparisons_spelled_with_parens_stay_comparisons() {
    // spec `expressions/calls.md`: an expression that means two comparisons
    // is written `f((a < b), c > (d))`.
    let p = parse_ok("fn main() { f((a < b), c > (d)) }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn");
    };
    let Statement::Expr(es) = &f.body.statements[0] else {
        panic!("expected expr statement");
    };
    match &es.expr {
        Expression::Call(c) => {
            assert_eq!(c.callee, "f");
            assert!(c.type_args.is_none(), "no written list: {:?}", c.type_args);
        }
        other => panic!("expected call, got {other:?}"),
    }
}

#[test]
fn logical_and_separates_comparison_operands() {
    parse_ok("fn main() { if a < b and b > (c - 1) { print(\"x\") } }");
}

#[test]
fn adjacent_closers_fail_with_a_note_to_insert_the_space() {
    // spec `grammar.md`: `first<Map<String, Int>>(m)` is not well-formed and
    // fails with a note; the well-formed spelling is `…> >(m)`.
    let err = parse_err("fn main() { first<List<List<number>>>(xs) }");
    let note = err.note.as_deref().unwrap_or_else(|| panic!("no note"));
    assert!(note.contains("`> >`"), "unexpected note: {note}");
}

#[test]
fn type_args_attach_to_method_enum_and_component_callees() {
    let call = main_call("fn main() { obj.m<number>(1) }");
    assert_eq!(call.method.as_deref(), Some("m"));
    assert_eq!(call.type_args, Some(vec![Type::Number]));

    let call = main_call("fn main() { Color::Red<number>() }");
    assert_eq!(call.callee, "Color::Red");
    assert_eq!(call.type_args, Some(vec![Type::Number]));

    let p = parse_ok("fn main() { Text<number>(\"Hi\") { } }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn");
    };
    let Statement::Component(c) = &f.body.statements[0] else {
        panic!(
            "expected component statement, got {:?}",
            f.body.statements[0]
        );
    };
    assert_eq!(c.name, "Text");
    assert_eq!(c.type_args, Some(vec![Type::Number]));
}

#[test]
fn subscript_and_result_callees_take_no_list_but_still_parse() {
    // The production itself is postfix-wide; the checker rejects these with
    // `E0807` (a callee reached through another postfix declares no type
    // parameters), which is a better diagnostic than the non-associative
    // comparison chain the alternative reading would produce.
    for src in [
        "fn main() { xs[0]<number>(1) }",
        "fn main() { getFn()<number>(1) }",
    ] {
        let p = parse_ok(src);
        let Statement::Fn(f) = &p.statements[0] else {
            panic!("expected fn");
        };
        let Statement::Expr(es) = &f.body.statements[0] else {
            panic!("expected expr statement");
        };
        let Expression::CallValue(cv) = &es.expr else {
            panic!("expected call through a value for {src}: {:?}", es.expr);
        };
        assert_eq!(cv.type_args, Some(vec![Type::Number]), "{src}");
    }
}
