use xulo_core::ast::{BinaryOperator, Expression, Literal, Statement};
use xulo_lexer::tokenize;
use xulo_parser::parse_program;

fn parse(src: &str) -> xulo_core::ast::Program {
    let tokens = tokenize(src).unwrap();
    parse_program(&tokens).unwrap()
}

#[test]
fn parses_function_definition() {
    let p = parse(r#"fn add(a: number, b: number): number { return a + b }"#);
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn statement");
    };
    assert_eq!(f.name, "add");
    assert_eq!(f.params.len(), 2);
    assert_eq!(f.return_type, Some(xulo_core::ast::Type::Number));
}

#[test]
fn parses_precedence() {
    let p = parse("let x = 1 + 2 * 3");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    match &b.value {
        Some(Expression::BinaryOp(op)) => {
            assert_eq!(op.operator, BinaryOperator::Add);
            assert!(
                matches!(op.right, Expression::BinaryOp(ref m) if m.operator == BinaryOperator::Mul)
            );
        }
        _ => panic!("expected binary op"),
    }
}

#[test]
fn range_and_comparison_share_one_precedence_level() {
    // Both sides of the range operator accept a comparison operand: the
    // grammar is symmetric (`a ..< b == c` used to be a parse error while
    // `a == b ..< c` parsed — the range operator bound to whichever side had
    // already been consumed).
    let p = parse("let x = a == b ..< c");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    assert!(
        matches!(b.value, Some(Expression::Range(_))),
        "`(a == b) ..< c` must parse as a range"
    );

    let p = parse("let y = a ..< b == c");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    match &b.value {
        Some(Expression::BinaryOp(op)) if op.operator == BinaryOperator::Eq => {
            assert!(
                matches!(op.left, Expression::Range(_)),
                "`(a ..< b) == c` must parse as a comparison"
            );
        }
        other => panic!("expected `(a ..< b) == c`, got {other:?}"),
    }
}

#[test]
fn parses_if_else_if() {
    let p = parse("if a { 1 } else if b { 2 } else { 3 }");
    let Statement::Expr(es) = &p.statements[0] else {
        panic!("expected if");
    };
    let Expression::If(outer) = &es.expr else {
        panic!("expected if");
    };
    assert!(outer.else_branch.is_some());
    let else_block = outer.else_branch.as_ref().unwrap();
    assert!(
        matches!(else_block.statements.as_slice(), [Statement::Expr(es)] if matches!(es.expr, Expression::If(_)))
    );
}

#[test]
fn parses_list_and_call() {
    let p = parse(r#"let xs = [1, 2, 3] print("hi")"#);
    assert!(matches!(&p.statements[0], Statement::Let(b)
        if matches!(&b.value, Some(Expression::Literal { value: Literal::List(v), .. }) if v.len() == 3)));
    assert!(
        matches!(&p.statements[1], Statement::Expr(es) if matches!(&es.expr, Expression::Call(c) if c.callee == "print"))
    );
}

#[test]
fn trailing_commas_are_accepted() {
    // `f(a, b,)`, `[1, 2,]`, `{ a: 1, }`, and enum payloads `A(number,)` all
    // accept a trailing comma, consistent with `match` arms and enum variants.
    let p = parse(r#"let xs = [1, 2,] print(f(1, 2,)) let o = { a: 1, }"#);
    assert!(matches!(&p.statements[0], Statement::Let(b)
        if matches!(&b.value, Some(Expression::Literal { value: Literal::List(v), .. }) if v.len() == 2)));
    assert!(
        matches!(&p.statements[1], Statement::Expr(es) if matches!(&es.expr, Expression::Call(c) if c.arguments.len() == 1 && matches!(&c.arguments[0].value, Expression::Call(inner) if inner.arguments.len() == 2)))
    );
    assert!(matches!(&p.statements[2], Statement::Let(b)
        if matches!(&b.value, Some(Expression::Literal { value: Literal::Object(f), .. }) if f.len() == 1)));

    let p = parse("enum E { A(number, string,) B }");
    let Statement::Enum(e) = &p.statements[0] else {
        panic!("expected enum");
    };
    let Some(payload) = &e.variants[0].payload else {
        panic!("expected payload");
    };
    assert_eq!(payload.len(), 2);
}

#[test]
fn optional_semicolons() {
    assert_eq!(parse("let x = 1").statements.len(), 1);
    assert_eq!(parse("let x = 1;").statements.len(), 1);
}

#[test]
fn reports_syntax_errors() {
    let tokens = tokenize("fn main() {").unwrap();
    assert!(parse_program(&tokens).is_err());
}

#[test]
fn parses_const_binding() {
    let p = parse("const PI = 3.14");
    let Statement::Let(b) = &p.statements[0] else {
        panic!("expected let");
    };
    assert!(b.is_const);
    assert_eq!(b.name, "PI");
}

#[test]
fn parses_null_literal() {
    let p = parse("let x = null");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(
        &b.value,
        Some(Expression::Literal {
            value: Literal::Null,
            ..
        })
    ));
}

#[test]
fn parses_assignment() {
    let p = parse("count = count + 1");
    let Statement::Assign(a) = &p.statements[0] else {
        panic!("expected assign");
    };
    assert_eq!(a.target, xulo_core::ast::AssignTarget::Name("count".into()));
}

#[test]
fn parses_member_and_index_assignment() {
    let p = parse("user.name = \"b\" xs[0] = 10");
    let Statement::Assign(a) = &p.statements[0] else {
        panic!("expected assign");
    };
    assert!(matches!(&a.target, xulo_core::ast::AssignTarget::Member(_, prop) if prop == "name"));
    let Statement::Assign(b) = &p.statements[1] else {
        panic!("expected assign");
    };
    assert!(matches!(b.target, xulo_core::ast::AssignTarget::Index(..)));
}

#[test]
fn parses_type_alias() {
    let p = parse("type User = { name: string }\n type Pair<T> = list<T>");
    let Statement::TypeAlias(a) = &p.statements[0] else {
        panic!("expected type alias");
    };
    assert_eq!(a.name, "User");
    assert!(matches!(a.type_, xulo_core::ast::Type::ObjectType(_)));
    let Statement::TypeAlias(g) = &p.statements[1] else {
        panic!("expected generic alias");
    };
    assert_eq!(g.type_params, vec!["T".to_string()]);
    assert!(matches!(g.type_, xulo_core::ast::Type::List(_)));
}

#[test]
fn parses_enum_with_payload() {
    let p = parse("enum Result<T> { Success(T) Error(string) }");
    let Statement::Enum(e) = &p.statements[0] else {
        panic!("expected enum");
    };
    assert_eq!(e.name, "Result");
    assert_eq!(e.variants.len(), 2);
    assert!(e.variants[0].payload.is_some());
    assert!(e.variants[1].payload.is_some());
}

#[test]
fn parses_enum_reference_and_construction() {
    let p = parse("let t = Theme::Dark\nlet r = Result::Success(42)");
    let Statement::Let(a) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(&a.value, Some(Expression::EnumRef(r))
        if r.enum_name == "Theme" && r.variant == "Dark"));
    let Statement::Let(b) = &p.statements[1] else {
        panic!();
    };
    assert!(matches!(&b.value, Some(Expression::Call(c))
        if c.callee == "Result::Success"));
}

#[test]
fn parses_complex_types() {
    let p = parse("let a: list<number> = []\nlet b: string? = null\nlet c: number | string = 1");
    assert!(matches!(&p.statements[0], Statement::Let(b)
        if matches!(b.type_annotation, Some(xulo_core::ast::Type::List(_)))));
    assert!(matches!(&p.statements[1], Statement::Let(b)
        if matches!(b.type_annotation, Some(xulo_core::ast::Type::Optional(_)))));
    assert!(matches!(&p.statements[2], Statement::Let(b)
        if matches!(b.type_annotation, Some(xulo_core::ast::Type::Union(_)))));
}

#[test]
fn parses_string_literal_type() {
    let p = parse(r#"let x: "active" = "active""#);
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    assert!(
        matches!(b.type_annotation, Some(xulo_core::ast::Type::Literal(ref s)) if s == "active")
    );
}

#[test]
fn parses_fn_type() {
    let p = parse("let h: fn(a: number, b: number): number = null");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    let Some(xulo_core::ast::Type::FnSig { params, ret }) = &b.type_annotation else {
        panic!("expected fn sig");
    };
    assert_eq!(params.len(), 2);
    assert!(ret.is_some());
}

#[test]
fn generic_fn_params() {
    let p = parse("fn first<T>(list: list<T>): T { list[0] }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn");
    };
    assert_eq!(f.type_params, vec!["T".to_string()]);
}

#[test]
fn parses_while_stmt() {
    let p = parse("while x < 10 { x = x + 1 }");
    assert!(matches!(&p.statements[0], Statement::While(w)
        if matches!(w.condition, Expression::BinaryOp(ref b) if b.operator == BinaryOperator::Lt)));
}

#[test]
fn parses_match_expr() {
    let p = parse("match v { 0 => \"zero\" _ => \"other\" }");
    let Statement::Expr(es) = &p.statements[0] else {
        panic!("expected match");
    };
    let Expression::Match(m) = &es.expr else {
        panic!("expected match");
    };
    assert_eq!(m.arms.len(), 2);
    assert!(matches!(
        m.arms[0].pattern,
        xulo_core::ast::MatchPattern::Literal(Literal::Number(0.0))
    ));
    assert!(matches!(
        m.arms[1].pattern,
        xulo_core::ast::MatchPattern::Wildcard
    ));
}

#[test]
fn parses_match_enum_payload() {
    let p = parse("match r { Result::Success(v) => v Result::Error(msg) => 0 }");
    let Statement::Expr(es) = &p.statements[0] else {
        panic!("expected match");
    };
    let Expression::Match(m) = &es.expr else {
        panic!("expected match");
    };
    assert!(
        matches!(&m.arms[0].pattern, xulo_core::ast::MatchPattern::EnumPayload {
        enum_name, variant, bindings, ..
    } if enum_name == "Result" && variant == "Success" && bindings == &["v".to_string()])
    );
}

#[test]
fn parses_ternary() {
    let p = parse("let x = a > 1 ? \"big\" : \"small\"");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(b.value, Some(Expression::Ternary(_))));
}

#[test]
fn parses_logical_operators() {
    let p = parse("let x = a and b or !c");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(b.value, Some(Expression::BinaryOp(ref bo))
        if bo.operator == BinaryOperator::Or));
}

#[test]
fn parses_member_access_and_index() {
    let p = parse("let n = user.name\nlet v = list[0]");
    let Statement::Let(a) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(a.value, Some(Expression::Member(ref m)) if m.property == "name"));
    let Statement::Let(b) = &p.statements[1] else {
        panic!();
    };
    assert!(matches!(b.value, Some(Expression::Index(_))));
}

#[test]
fn parses_method_call() {
    let p = parse("store.actions.setLoading(true)");
    let Statement::Expr(es) = &p.statements[0] else {
        panic!("expected call");
    };
    let Expression::Call(c) = &es.expr else {
        panic!("expected call");
    };
    assert!(c.object.is_some());
    assert_eq!(c.method.as_deref(), Some("setLoading"));
}

#[test]
fn parses_nullish_and_optional_member() {
    let p = parse("let x = user?.name ?? \"anon\"");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(b.value, Some(Expression::Nullish(_))));
}

#[test]
fn parses_object_spread() {
    let p = parse("let o = { ...base, y: 2 }");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    let Some(Expression::Literal {
        value: Literal::Object(fields),
        ..
    }) = &b.value
    else {
        panic!("expected object literal");
    };
    assert!(matches!(
        fields[0],
        xulo_core::ast::ObjectField::Spread { .. }
    ));
}

#[test]
fn parses_default_params() {
    let p = parse("fn greet(name: string = \"x\") { }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    assert!(f.params[0].default.is_some());
}

#[test]
fn parses_named_args() {
    let p = parse("greet(label: \"Submit\", variant: \"outline\")");
    let Statement::Expr(es) = &p.statements[0] else {
        panic!("expected call");
    };
    let Expression::Call(c) = &es.expr else {
        panic!("expected call");
    };
    assert_eq!(c.arguments[0].name.as_deref(), Some("label"));
}

#[test]
fn parses_range_for_loop() {
    let p = parse("for i in 0..<10 { print(i) }");
    let Statement::For(f) = &p.statements[0] else {
        panic!("expected for");
    };
    assert!(matches!(f.iterable, Expression::Range(_)));
}

#[test]
fn parses_unary_not() {
    let p = parse("let x = !flag");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(b.value, Some(Expression::Unary(ref u))
        if u.operator == xulo_core::ast::UnaryOperator::Not));
}

#[test]
fn parses_async_fn_and_await() {
    let p = parse("fn load(): async number { let v = await fetch() return v }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn");
    };
    assert!(f.is_async);
    assert_eq!(
        f.return_type,
        Some(xulo_core::ast::Type::Async(Box::new(
            xulo_core::ast::Type::Number
        )))
    );
    let Statement::Let(b) = &f.body.statements[0] else {
        panic!("expected let");
    };
    assert!(matches!(b.value, Some(Expression::Await { .. })));
}

#[test]
fn parses_try_catch_throw() {
    use xulo_core::ast::{Statement, TryStmt};
    let p = parse("try { throw \"err\" } catch (e) { print(e) }");
    let Statement::Try(t) = &p.statements[0] else {
        panic!("expected try");
    };
    let TryStmt {
        catch_var,
        try_block,
        ..
    } = t;
    assert_eq!(catch_var, "e");
    assert!(matches!(
        try_block.statements.as_slice(),
        [Statement::Throw(_)]
    ));
}

#[test]
fn parses_import_and_export_forms() {
    use xulo_core::ast::{ExportItem, ExportStmt, ImportSpec, ImportStmt, Statement};

    let p = parse(r#"import { a as b } from "./m" import * as ns from "./n" import t from "./d""#);
    assert!(matches!(p.statements[0],
        Statement::Import(ImportStmt { spec: ImportSpec::Named(_), ref source, type_only: false }) if source == "./m"));
    assert!(matches!(
        p.statements[1],
        Statement::Import(ImportStmt {
            spec: ImportSpec::Namespace(_),
            ..
        })
    ));
    assert!(matches!(
        p.statements[2],
        Statement::Import(ImportStmt {
            spec: ImportSpec::Default(_),
            ..
        })
    ));

    let p = parse(
        r#"import type { User } from "./u" export { a, b } export const X = 1 export type Y = string"#,
    );
    assert!(matches!(
        p.statements[0],
        Statement::Import(ImportStmt {
            type_only: true,
            ..
        })
    ));
    assert!(matches!(
        p.statements[1],
        Statement::Export(ExportStmt {
            item: ExportItem::Names(_)
        })
    ));
    assert!(matches!(
        p.statements[2],
        Statement::Export(ExportStmt {
            item: ExportItem::Let(_)
        })
    ));
    assert!(matches!(
        p.statements[3],
        Statement::Export(ExportStmt {
            item: ExportItem::Type(_)
        })
    ));
}

#[test]
fn parses_pub_declarations_as_exports() {
    use xulo_core::ast::{ExportItem, ExportStmt};
    let p = parse(
        "pub fn add(a: number): number { return a } pub const PI = 3.14 \
         pub let x = 1 pub type User = { name: string } pub enum Status { Active }",
    );
    assert_eq!(p.statements.len(), 5);
    assert!(matches!(
        &p.statements[0],
        Statement::Export(ExportStmt {
            item: ExportItem::Fn(_)
        })
    ));
    assert!(matches!(
        &p.statements[1],
        Statement::Export(ExportStmt {
            item: ExportItem::Let(_)
        })
    ));
    assert!(matches!(
        &p.statements[2],
        Statement::Export(ExportStmt {
            item: ExportItem::Let(_)
        })
    ));
    assert!(matches!(
        &p.statements[3],
        Statement::Export(ExportStmt {
            item: ExportItem::Type(_)
        })
    ));
    assert!(matches!(
        &p.statements[4],
        Statement::Export(ExportStmt {
            item: ExportItem::Enum(_)
        })
    ));
}

#[test]
fn rejects_pub_combined_with_export() {
    let tokens = tokenize("pub export fn foo() {}").unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(
        err.message.contains("cannot be combined"),
        "unexpected message: {}",
        err.message
    );
    assert!(err.span.is_some(), "error must carry a span");
}

#[test]
fn rejects_reserved_word_as_binding_name() {
    let tokens = tokenize("let struct = 1").unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(
        err.message.contains("reserved keyword `struct`"),
        "got: {}",
        err.message
    );
    assert!(err.span.is_some(), "error must carry a span");
}

#[test]
fn rejects_reserved_word_as_function_name() {
    let tokens = tokenize("fn new() {}").unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(
        err.message.contains("reserved keyword `new`"),
        "unexpected message: {}",
        err.message
    );
    assert!(err.span.is_some(), "error must carry a span");
}

#[test]
fn rejects_reserved_word_as_statement() {
    let tokens = tokenize("yield").unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(
        err.message.contains("reserved keyword `yield`"),
        "unexpected message: {}",
        err.message
    );
    assert!(err.span.is_some(), "error must carry a span");
}

#[test]
fn rejects_reserved_word_in_type_position() {
    let tokens = tokenize("let x: struct = 1").unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(
        err.message.contains("reserved keyword `struct`"),
        "unexpected message: {}",
        err.message
    );
    assert!(err.span.is_some(), "error must carry a span");
}

#[test]
fn parses_export_default_fn() {
    use xulo_core::ast::ExportItem;
    let p = parse("export default fn main() { print(\"hi\") }");
    let Statement::Export(export) = &p.statements[0] else {
        panic!("expected export");
    };
    assert!(matches!(export.item, ExportItem::Default(_)));
}

#[test]
fn parses_anonymous_function_expression() {
    use xulo_core::ast::{Expression, FnExpr};
    let p = parse("let f = fn(a: number, b: number): number { a + b }");
    let Statement::Let(b) = &p.statements[0] else {
        panic!("expected let");
    };
    match &b.value {
        Some(Expression::FnExpr(f)) => {
            let FnExpr {
                params,
                return_type,
                body,
                is_async,
                ..
            } = f.as_ref();
            assert_eq!(params.len(), 2);
            assert_eq!(return_type, &Some(xulo_core::ast::Type::Number));
            assert!(!body.statements.is_empty());
            assert!(!is_async);
        }
        _ => panic!("expected fn expression"),
    }
}

#[test]
fn parses_async_anonymous_function() {
    let p = parse("let f = fn(): async number { let v = await g() return v }");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(b.value, Some(Expression::FnExpr(ref f))
        if f.is_async && f.return_type == Some(xulo_core::ast::Type::Async(Box::new(xulo_core::ast::Type::Number)))));
}

#[test]
fn parses_list_spread() {
    let p = parse("let xs = [1, ...rest, 3]");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    let Some(Expression::Literal {
        value: Literal::List(items),
        ..
    }) = &b.value
    else {
        panic!("expected list literal");
    };
    assert_eq!(items.len(), 3);
    assert!(matches!(items[1], Expression::Spread { .. }));
}

#[test]
fn parses_list_spread_leading() {
    let p = parse("let xs = [...head]");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    assert!(
        matches!(b.value, Some(Expression::Literal { value: Literal::List(ref items), .. })
        if matches!(items[0], Expression::Spread { .. }))
    );
}

#[test]
fn parses_match_expr_with_commas() {
    let p = parse("match v { 0 => \"zero\", 1 => \"one\", _ => \"other\" }");
    let Statement::Expr(es) = &p.statements[0] else {
        panic!("expected match");
    };
    let Expression::Match(m) = &es.expr else {
        panic!("expected match");
    };
    assert_eq!(m.arms.len(), 3);
    assert!(matches!(
        m.arms[0].pattern,
        xulo_core::ast::MatchPattern::Literal(Literal::Number(0.0))
    ));
    assert!(matches!(
        m.arms[2].pattern,
        xulo_core::ast::MatchPattern::Wildcard
    ));
}

#[test]
fn parses_match_enum_payload_with_commas() {
    let p = parse("match r { Result::Success(v) => v, Result::Error(msg) => 0, _ => -1 }");
    let Statement::Expr(es) = &p.statements[0] else {
        panic!("expected match");
    };
    let Expression::Match(m) = &es.expr else {
        panic!("expected match");
    };
    assert_eq!(m.arms.len(), 3);
    assert!(
        matches!(&m.arms[1].pattern, xulo_core::ast::MatchPattern::EnumPayload {
        variant, bindings, ..
    } if variant == "Error" && bindings == &["msg".to_string()])
    );
}

#[test]
fn parses_named_enum_payload() {
    let p = parse("enum Action { Click, Submit(data: object), Cancel }");
    let Statement::Enum(e) = &p.statements[0] else {
        panic!("expected enum");
    };
    assert_eq!(e.variants[1].name, "Submit");
    assert_eq!(
        e.variants[1]
            .payload
            .as_ref()
            .and_then(|p| p.first())
            .and_then(|p| p.name.as_deref()),
        Some("data")
    );
    assert_eq!(e.variants[0].payload, None);
}

#[test]
fn parses_call_on_indexed_function_value() {
    let p = parse("let r = xs[0](10, 4)");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(b.value, Some(Expression::CallValue(_))));
}

#[test]
fn parses_bare_fn_expression_statement() {
    let p =
        parse("fn makeAdder(n: number): fn(number): number { fn(v: number): number { v + n } }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn");
    };
    assert!(
        matches!(f.body.statements.last(), Some(Statement::Expr(es)) if matches!(es.expr, Expression::FnExpr(_)))
    );
}

#[test]
fn parses_state_declaration() {
    use xulo_core::ast::StateStmt;
    let p = parse("@State let count: number = 0");
    let Statement::State(StateStmt { binding }) = &p.statements[0] else {
        panic!("expected state");
    };
    assert_eq!(binding.name, "count");
    assert!(matches!(
        binding.type_annotation,
        Some(xulo_core::ast::Type::Number)
    ));
}

#[test]
fn parses_store_destructure() {
    use xulo_core::ast::{BindingPattern, StoreStmt};
    let p = parse("@Store const { user, theme: t } = useAppStore()");
    let Statement::Store(StoreStmt { pattern, .. }) = &p.statements[0] else {
        panic!("expected store");
    };
    match pattern {
        BindingPattern::Destructure(fields) => {
            assert_eq!(fields[0].0, "user");
            assert_eq!(fields[1].1.as_deref(), Some("t"));
        }
        _ => panic!("expected destructure"),
    }
}

#[test]
fn parses_effect_with_deps() {
    use xulo_core::ast::EffectStmt;
    let p = parse("@Effect fn() { fetchUser(id) }, [id]");
    let Statement::Effect(EffectStmt { deps, .. }) = &p.statements[0] else {
        panic!("expected effect");
    };
    assert!(deps.is_some());
    assert_eq!(deps.as_ref().unwrap().len(), 1);
}

#[test]
fn parses_environment_declaration() {
    use xulo_core::ast::EnvStmt;
    let p = parse("@Environment let router: Router");
    let Statement::Environment(EnvStmt { name, .. }) = &p.statements[0] else {
        panic!("expected environment");
    };
    assert_eq!(name, "router");
}

#[test]
fn parses_component_block() {
    use xulo_core::ast::{ComponentStmt, UiElement};
    let p = parse("VStack(spacing: 16) { Text(\"Hello\") }");
    let Statement::Component(ComponentStmt {
        name,
        args,
        children,
    }) = &p.statements[0]
    else {
        panic!("expected component");
    };
    assert_eq!(name, "VStack");
    assert_eq!(args.len(), 1);
    assert!(matches!(&children[0], UiElement::Component(c) if c.name == "Text"));
}

#[test]
fn parses_component_without_args_or_children() {
    let p = parse("Screen { }");
    let Statement::Component(c) = &p.statements[0] else {
        panic!("expected component");
    };
    assert_eq!(c.name, "Screen");
    assert!(c.children.is_empty());
}

#[test]
fn parses_ui_if_and_for() {
    use xulo_core::ast::UiElement;
    let p = parse("VStack { if ok { Text(\"a\") } else { Text(\"b\") } for x in xs { Text(x) } }");
    let Statement::Component(c) = &p.statements[0] else {
        panic!("expected component");
    };
    assert!(matches!(&c.children[0], UiElement::If { .. }));
    assert!(matches!(&c.children[1], UiElement::For { .. }));
}

#[test]
fn parses_unary_minus_and_double_not() {
    let p = parse("let a = -5 let b = !!flag let c = -x");
    let Statement::Let(a) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(a.value, Some(Expression::Unary(ref u))
        if u.operator == xulo_core::ast::UnaryOperator::Neg));
    let Statement::Let(b) = &p.statements[1] else {
        panic!();
    };
    let Some(Expression::Unary(outer)) = &b.value else {
        panic!("expected unary");
    };
    assert!(matches!(&outer.operand, Expression::Unary(inner)
        if inner.operator == xulo_core::ast::UnaryOperator::Not));
    let Statement::Let(c) = &p.statements[2] else {
        panic!();
    };
    assert!(matches!(c.value, Some(Expression::Unary(ref u))
        if u.operator == xulo_core::ast::UnaryOperator::Neg));
}

#[test]
fn parses_nested_object_and_list_literals() {
    let p = parse("let o = { a: { b: [1, 2] }, c: \"x\" }");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    let Some(Expression::Literal {
        value: Literal::Object(fields),
        ..
    }) = &b.value
    else {
        panic!("expected object literal");
    };
    assert_eq!(fields.len(), 2);
    assert!(
        matches!(&fields[0], xulo_core::ast::ObjectField::Field { name, value } if name == "a"
        && matches!(value, Expression::Literal { value: Literal::Object(_), .. }))
    );
    assert!(
        matches!(&fields[1], xulo_core::ast::ObjectField::Field { name, value } if name == "c"
        && matches!(value, Expression::Literal { value: Literal::String(s), .. } if s == "x"))
    );
}

#[test]
fn parses_method_chain_through_index_and_call() {
    let p = parse("store.actions.pick(0).items[0].label");
    let Statement::Expr(es) = &p.statements[0] else {
        panic!();
    };
    let Expression::Member(m) = &es.expr else {
        panic!("expected member");
    };
    assert_eq!(m.property, "label");
    assert!(matches!(&m.object, Expression::Index(_)));
}

#[test]
fn parses_nested_ternary_is_right_associative() {
    let p = parse("let x = a ? b ? c : d : e");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    let Some(Expression::Ternary(outer)) = &b.value else {
        panic!("expected ternary");
    };
    assert!(matches!(&outer.then_value, Expression::Ternary(inner)
        if matches!(&inner.else_value, Expression::Identifier { name, .. } if name == "d")));
}

#[test]
fn parses_ternary_inside_call_and_index() {
    let p = parse("f(cond ? 1 : 2)[0]");
    let Statement::Expr(es) = &p.statements[0] else {
        panic!();
    };
    let Expression::Index(idx) = &es.expr else {
        panic!("expected index");
    };
    let Expression::Call(c) = &*idx.object else {
        panic!("expected call");
    };
    assert!(matches!(&c.arguments[0].value, Expression::Ternary(_)));
    assert!(
        matches!(&*idx.index, Expression::Literal { value: Literal::Number(n), .. } if *n == 0.0)
    );
}

#[test]
fn parses_match_scalar_and_enum_patterns() {
    let p =
        parse("match v { 1 => \"a\" \"s\" => \"b\" true => \"c\" E::V(x) => \"d\" _ => \"e\" }");
    let Statement::Expr(es) = &p.statements[0] else {
        panic!("expected match");
    };
    let Expression::Match(m) = &es.expr else {
        panic!("expected match expr");
    };
    assert_eq!(m.arms.len(), 5);
    assert!(matches!(
        &m.arms[0].pattern,
        xulo_core::ast::MatchPattern::Literal(Literal::Number(_))
    ));
    assert!(matches!(
        &m.arms[1].pattern,
        xulo_core::ast::MatchPattern::Literal(Literal::String(_))
    ));
    assert!(matches!(
        &m.arms[2].pattern,
        xulo_core::ast::MatchPattern::Literal(Literal::Boolean(_))
    ));
    if let xulo_core::ast::MatchPattern::EnumPayload { bindings, .. } = &m.arms[3].pattern {
        assert_eq!(bindings, &vec!["x".to_string()]);
    } else {
        panic!("expected enum payload pattern");
    }
    assert!(matches!(
        &m.arms[4].pattern,
        xulo_core::ast::MatchPattern::Wildcard
    ));
}

#[test]
fn parses_optional_and_paren_union_types() {
    let p = parse(
        "let a: list<number>? = null let b: (number | string) = 1 let c: { name: string }? = null",
    );
    let Statement::Let(a) = &p.statements[0] else {
        panic!()
    };
    assert!(matches!(
        a.type_annotation,
        Some(xulo_core::ast::Type::Optional(_))
    ));
    let Statement::Let(b) = &p.statements[1] else {
        panic!()
    };
    assert!(matches!(
        b.type_annotation,
        Some(xulo_core::ast::Type::Union(_))
    ));
    let Statement::Let(c) = &p.statements[2] else {
        panic!()
    };
    assert!(matches!(
        c.type_annotation,
        Some(xulo_core::ast::Type::Optional(_))
    ));
}

#[test]
fn parses_enum_payload_wildcard_and_named_construction() {
    let p = parse("E::V(_, _) E::W(data: 1, label: \"x\")");
    let Statement::Expr(es) = &p.statements[0] else {
        panic!("expected enum call");
    };
    let Expression::Call(c) = &es.expr else {
        panic!("expected call")
    };
    assert!(c.is_enum());
    assert_eq!(c.enum_parts(), Some(("E", "V")));
    assert_eq!(c.arguments.len(), 2);
    assert!(matches!(&c.arguments[0].value, Expression::Identifier { name, .. } if name == "_"));

    let Statement::Expr(es) = &p.statements[1] else {
        panic!()
    };
    let Expression::Call(c) = &es.expr else {
        panic!("expected call")
    };
    assert_eq!(c.arguments[0].name.as_deref(), Some("data"));
    assert_eq!(c.arguments[1].name.as_deref(), Some("label"));
}

#[test]
fn parses_await_on_indexed_value() {
    let p = parse("let x = await xs[0]");
    let Statement::Let(b) = &p.statements[0] else {
        panic!()
    };
    let Some(Expression::Await { expr, .. }) = &b.value else {
        panic!("expected await")
    };
    assert!(matches!(**expr, Expression::Index(_)));
}

#[test]
fn parses_component_nested_ui_with_args() {
    let p = parse(
        "VStack { HStack(spacing: 4) { Text(\"a\") } Button(onClick: fn() {}) { Text(\"b\") } }",
    );
    let Statement::Component(comp) = &p.statements[0] else {
        panic!("expected component");
    };
    assert_eq!(comp.name, "VStack");
    assert_eq!(comp.children.len(), 2);
    assert!(
        matches!(&comp.children[0], xulo_core::ast::UiElement::Component(inner)
        if inner.name == "HStack")
    );
}

#[test]
fn parses_expr_child_in_component_block() {
    use xulo_core::ast::{Expression, UiElement};
    let p = parse("VStack { children }");
    let Statement::Component(comp) = &p.statements[0] else {
        panic!("expected component");
    };
    assert_eq!(comp.children.len(), 1);
    assert!(
        matches!(&comp.children[0], UiElement::Expr(Expression::Identifier { name, .. })
        if name == "children")
    );
}

#[test]
fn parses_mixed_component_and_expr_children() {
    use xulo_core::ast::UiElement;
    let p = parse("Card(title: \"x\") { Text(\"a\") children }");
    let Statement::Component(comp) = &p.statements[0] else {
        panic!("expected component");
    };
    assert_eq!(comp.children.len(), 2);
    assert!(matches!(&comp.children[0], UiElement::Component(c) if c.name == "Text"));
    assert!(matches!(&comp.children[1], UiElement::Expr(_)));
}

#[test]
fn parses_member_and_index_expr_children() {
    use xulo_core::ast::{Expression, UiElement};
    let p = parse("VStack { user.name items[0] }");
    let Statement::Component(comp) = &p.statements[0] else {
        panic!("expected component");
    };
    assert!(matches!(
        &comp.children[0],
        UiElement::Expr(Expression::Member(_))
    ));
    assert!(matches!(
        &comp.children[1],
        UiElement::Expr(Expression::Index(_))
    ));
}

#[test]
fn syntax_error_carries_span() {
    let tokens = tokenize("fn main() { let x = }").unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(err.span.is_some(), "syntax error must carry a span");
}

#[test]
fn deep_unary_chain_is_nesting_error_not_overflow() {
    // A deep prefix-operator chain must produce a controlled nesting error
    // instead of exhausting the stack (regression: `unary` without a guard).
    let src = format!("fn main() {{ let x = {}1 }}", "!".repeat(200));
    let tokens = tokenize(&src).unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(
        err.message.contains("nesting is too deep"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn deep_unary_chain_within_limits_parses() {
    // A reasonable chain (well under the nest budget) still parses.
    let src = format!("fn main() {{ let x = {}1 }}", "!".repeat(50));
    assert!(parse(&src).statements.len() == 1);
}

#[test]
fn deeply_nested_type_is_nesting_error_not_overflow() {
    // `type_expr` recursion must be guarded as well (regression).
    let deep: String = "User<".repeat(200);
    let src = format!(
        "fn main() {{ let x: {}number > {} = 1 }}",
        deep,
        ">".repeat(200)
    );
    let tokens = tokenize(&src).unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(
        err.message.contains("nesting is too deep"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn deep_else_if_chain_is_nesting_error_not_overflow() {
    // `if ... else if ...` chains recurse via a nested `if_expr`; the
    // `ElseIf` arm must be guarded (regression).
    let mut src = String::from("fn main() { let v = 0 if false { 1 }");
    for _ in 0..200 {
        src.push_str(" else if false { 1 }");
    }
    src.push_str(" print(v) }");
    let tokens = tokenize(&src).unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(
        err.message.contains("nesting is too deep"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn parses_dollar_binding_argument() {
    use xulo_core::ast::Expression;
    let p = parse("Input(value: $name)");
    let Statement::Component(c) = &p.statements[0] else {
        panic!("expected component");
    };
    assert!(matches!(&c.args[0].value, Expression::Binding { name: n, .. } if n == "name"));
}

#[test]
fn parses_trait_declaration() {
    use xulo_core::ast::Type;
    let p = parse(
        r#"
        trait Shape {
            fn area(self): number
            fn scale(self, by: number): Shape;
        }
        "#,
    );
    let Statement::Trait(t) = &p.statements[0] else {
        panic!("expected trait statement");
    };
    assert_eq!(t.name, "Shape");
    assert_eq!(t.methods.len(), 2);
    let area = &t.methods[0];
    assert_eq!(area.name, "area");
    assert!(area.has_self);
    assert_eq!(area.params.len(), 0);
    assert_eq!(area.return_type, Some(Type::Number));
    let scale = &t.methods[1];
    assert_eq!(scale.name, "scale");
    assert!(scale.has_self);
    assert_eq!(scale.params.len(), 1);
    assert_eq!(scale.params[0].name, "by");
    assert_eq!(scale.params[0].type_annotation, Some(Type::Number));
    assert_eq!(scale.return_type, Some(Type::Named("Shape".into())));
}

#[test]
fn parses_trait_method_without_self() {
    let p = parse("trait Describable { fn describe(): string }");
    let Statement::Trait(t) = &p.statements[0] else {
        panic!("expected trait statement");
    };
    assert!(!t.methods[0].has_self);
    assert_eq!(t.methods[0].params.len(), 0);
}

#[test]
fn parses_impl_block() {
    use xulo_core::ast::Type;
    let p = parse(
        r#"
        impl Shape for Circle {
            fn area(self): number { return 1 }
            fn scale(self, by: number): Shape { return 1 }
        }
        "#,
    );
    let Statement::Impl(imp) = &p.statements[0] else {
        panic!("expected impl statement");
    };
    assert_eq!(imp.trait_name, "Shape");
    assert_eq!(imp.type_name, "Circle");
    assert_eq!(imp.methods.len(), 2);
    let area = &imp.methods[0];
    assert_eq!(area.name, "area");
    assert_eq!(area.params[0].name, "self");
    assert_eq!(area.params.len(), 1);
    assert_eq!(area.return_type, Some(Type::Number));
    let scale = &imp.methods[1];
    assert_eq!(scale.params[0].name, "self");
    assert_eq!(scale.params[1].name, "by");
}

#[test]
fn parses_generic_bounds_inline_and_where() {
    let p = parse("fn paint<T: Shape>(t: T): T where T: Drawable { return t }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn statement");
    };
    assert_eq!(f.type_params, vec!["T"]);
    // Inline bound `<T: Shape>` plus trailing `where T: Drawable`.
    assert_eq!(f.bounds.len(), 2);
    assert_eq!(f.bounds[0].param, "T");
    assert_eq!(f.bounds[0].traits, vec!["Shape"]);
    assert_eq!(f.bounds[1].param, "T");
    assert_eq!(f.bounds[1].traits, vec!["Drawable"]);
}

#[test]
fn parses_multi_trait_bound() {
    let p = parse("fn f<U: One & Two>(u: U) { print(u) }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn statement");
    };
    assert_eq!(f.bounds.len(), 1);
    assert_eq!(f.bounds[0].traits, vec!["One", "Two"]);
}

#[test]
fn parses_exported_trait() {
    let p = parse("export trait Area { fn area(self): number }");
    let Statement::Export(e) = &p.statements[0] else {
        panic!("expected export statement");
    };
    assert!(matches!(e.item, xulo_core::ast::ExportItem::Trait(_)));
}

#[test]
fn rejects_self_in_plain_function() {
    // `self` is a reserved word; only `impl` method params may use it.
    let tokens = tokenize("fn f(self) { print(1) }").unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(
        err.message.contains("`self`"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn rejects_self_not_first_in_impl_method() {
    // `self` binds the receiver positionally, so it is only legal as the
    // first parameter; mid/trailing positions would mis-bind arguments.
    let tokens =
        tokenize("impl Shape for Circle { fn area(x, self): number { return 1 } }").unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(
        err.message.contains("`self` must be the first parameter"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn rejects_self_not_first_in_trait_method() {
    let tokens = tokenize("trait Shape { fn area(x, self): number }").unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(
        err.message.contains("`self` must be the first parameter"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn trait_is_a_keyword_not_identifier() {
    let tokens = tokenize("let trait = 1").unwrap();
    let err = parse_program(&tokens).unwrap_err();
    assert!(
        err.message.contains("`trait`"),
        "unexpected message: {}",
        err.message
    );
}
