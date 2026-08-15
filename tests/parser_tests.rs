use xulo::ast::{BinaryOperator, Expression, Literal, Statement};
use xulo::lexer::tokenize;
use xulo::parser::parse_program;

fn parse(src: &str) -> xulo::ast::Program {
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
    assert_eq!(f.return_type, Some(xulo::ast::Type::Number));
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
    assert_eq!(a.target, xulo::ast::AssignTarget::Name("count".into()));
}

#[test]
fn parses_member_and_index_assignment() {
    let p = parse("user.name = \"b\" xs[0] = 10");
    let Statement::Assign(a) = &p.statements[0] else {
        panic!("expected assign");
    };
    assert!(matches!(&a.target, xulo::ast::AssignTarget::Member(_, prop) if prop == "name"));
    let Statement::Assign(b) = &p.statements[1] else {
        panic!("expected assign");
    };
    assert!(matches!(b.target, xulo::ast::AssignTarget::Index(..)));
}

#[test]
fn parses_type_alias() {
    let p = parse("type User = { name: string }\n type Pair<T> = list<T>");
    let Statement::TypeAlias(a) = &p.statements[0] else {
        panic!("expected type alias");
    };
    assert_eq!(a.name, "User");
    assert!(matches!(a.type_, xulo::ast::Type::ObjectType(_)));
    let Statement::TypeAlias(g) = &p.statements[1] else {
        panic!("expected generic alias");
    };
    assert_eq!(g.type_params, vec!["T".to_string()]);
    assert!(matches!(g.type_, xulo::ast::Type::List(_)));
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
        if matches!(b.type_annotation, Some(xulo::ast::Type::List(_)))));
    assert!(matches!(&p.statements[1], Statement::Let(b)
        if matches!(b.type_annotation, Some(xulo::ast::Type::Optional(_)))));
    assert!(matches!(&p.statements[2], Statement::Let(b)
        if matches!(b.type_annotation, Some(xulo::ast::Type::Union(_)))));
}

#[test]
fn parses_string_literal_type() {
    let p = parse(r#"let x: "active" = "active""#);
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(b.type_annotation, Some(xulo::ast::Type::Literal(ref s)) if s == "active"));
}

#[test]
fn parses_fn_type() {
    let p = parse("let h: fn(a: number, b: number): number = null");
    let Statement::Let(b) = &p.statements[0] else {
        panic!();
    };
    let Some(xulo::ast::Type::FnSig { params, ret }) = &b.type_annotation else {
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
        xulo::ast::MatchPattern::Literal(Literal::Number(0.0))
    ));
    assert!(matches!(
        m.arms[1].pattern,
        xulo::ast::MatchPattern::Wildcard
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
        matches!(&m.arms[0].pattern, xulo::ast::MatchPattern::EnumPayload {
        enum_name, variant, binding
    } if enum_name == "Result" && variant == "Success" && binding == "v")
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
    assert!(matches!(fields[0], xulo::ast::ObjectField::Spread { .. }));
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
        if u.operator == xulo::ast::UnaryOperator::Not));
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
        Some(xulo::ast::Type::Async(Box::new(xulo::ast::Type::Number)))
    );
    let Statement::Let(b) = &f.body.statements[0] else {
        panic!("expected let");
    };
    assert!(matches!(b.value, Some(Expression::Await { .. })));
}

#[test]
fn parses_try_catch_throw() {
    use xulo::ast::{Statement, TryStmt};
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
    use xulo::ast::{ExportItem, ExportStmt, ImportSpec, ImportStmt, Statement};

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
fn parses_export_default_fn() {
    use xulo::ast::ExportItem;
    let p = parse("export default fn main() { print(\"hi\") }");
    let Statement::Export(export) = &p.statements[0] else {
        panic!("expected export");
    };
    assert!(matches!(export.item, ExportItem::Default(_)));
}

#[test]
fn parses_anonymous_function_expression() {
    use xulo::ast::{Expression, FnExpr};
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
            assert_eq!(return_type, &Some(xulo::ast::Type::Number));
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
        if f.is_async && f.return_type == Some(xulo::ast::Type::Async(Box::new(xulo::ast::Type::Number)))));
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
        xulo::ast::MatchPattern::Literal(Literal::Number(0.0))
    ));
    assert!(matches!(
        m.arms[2].pattern,
        xulo::ast::MatchPattern::Wildcard
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
        matches!(&m.arms[1].pattern, xulo::ast::MatchPattern::EnumPayload {
        variant, binding, ..
    } if variant == "Error" && binding == "msg")
    );
}

#[test]
fn parses_named_enum_payload() {
    let p = parse("enum Action { Click, Submit(data: object), Cancel }");
    let Statement::Enum(e) = &p.statements[0] else {
        panic!("expected enum");
    };
    assert_eq!(e.variants[1].name, "Submit");
    assert_eq!(e.variants[1].payload_name.as_deref(), Some("data"));
    assert_eq!(e.variants[0].payload_name, None);
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
    use xulo::ast::StateStmt;
    let p = parse("@State let count: number = 0");
    let Statement::State(StateStmt { binding }) = &p.statements[0] else {
        panic!("expected state");
    };
    assert_eq!(binding.name, "count");
    assert!(matches!(
        binding.type_annotation,
        Some(xulo::ast::Type::Number)
    ));
}

#[test]
fn parses_store_destructure() {
    use xulo::ast::{BindingPattern, StoreStmt};
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
    use xulo::ast::EffectStmt;
    let p = parse("@Effect fn() { fetchUser(id) }, [id]");
    let Statement::Effect(EffectStmt { deps, .. }) = &p.statements[0] else {
        panic!("expected effect");
    };
    assert!(deps.is_some());
    assert_eq!(deps.as_ref().unwrap().len(), 1);
}

#[test]
fn parses_environment_declaration() {
    use xulo::ast::EnvStmt;
    let p = parse("@Environment let router: Router");
    let Statement::Environment(EnvStmt { name, .. }) = &p.statements[0] else {
        panic!("expected environment");
    };
    assert_eq!(name, "router");
}

#[test]
fn parses_component_block() {
    use xulo::ast::{ComponentStmt, UiElement};
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
    use xulo::ast::UiElement;
    let p = parse("VStack { if ok { Text(\"a\") } else { Text(\"b\") } for x in xs { Text(x) } }");
    let Statement::Component(c) = &p.statements[0] else {
        panic!("expected component");
    };
    assert!(matches!(&c.children[0], UiElement::If { .. }));
    assert!(matches!(&c.children[1], UiElement::For { .. }));
}

#[test]
fn parses_dollar_binding_argument() {
    use xulo::ast::Expression;
    let p = parse("Input(value: $name)");
    let Statement::Component(c) = &p.statements[0] else {
        panic!("expected component");
    };
    assert!(matches!(&c.args[0].value, Expression::Binding { name: n, .. } if n == "name"));
}
