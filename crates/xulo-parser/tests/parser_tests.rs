//! Integration tests for the pest-based parser: spec-conformant constructs,
//! lowered-AST shape checks, negative cases, and the nesting-depth limit.

use xulo_core::ast::{
    Expression, Literal, MatchBody, MatchPattern, PathHead, Statement, Type, UiElement, UseTarget,
};
use xulo_parser::{MAX_NEST_DEPTH, parse_program};

fn parse_ok(src: &str) -> xulo_core::ast::Program {
    parse_program(src).unwrap_or_else(|e| panic!("expected OK for {src:?}: {}", e.message))
}

fn parse_err(src: &str) -> xulo_core::error::XuloError {
    parse_program(src).expect_err(&format!("expected error for {src:?}"))
}

fn first_let(src: &str) -> xulo_core::ast::LetBinding {
    let p = parse_ok(src);
    match &p.statements[0] {
        Statement::Let(l) => l.clone(),
        other => panic!("expected let, got {other:?}"),
    }
}

// ---------------------------------------------------------------
// Declarations
// ---------------------------------------------------------------

#[test]
fn fn_decl_with_params_and_return() {
    let p = parse_ok("fn add(a: number, b: number): number { a + b }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn");
    };
    assert_eq!(f.name, "add");
    assert_eq!(f.params.len(), 2);
    assert_eq!(f.params[0].name, "a");
    assert_eq!(f.return_type, Some(Type::Number));
    assert!(!f.is_async);
    assert!(f.type_params.is_empty());
}

#[test]
fn pub_fn_is_exported() {
    let p = parse_ok("pub fn add(a: number): number { a }");
    assert!(matches!(p.statements[0], Statement::Export(_)));
}

#[test]
fn async_fn() {
    let p = parse_ok("async fn f(): number { await g() }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn");
    };
    assert!(f.is_async);
}

#[test]
fn generic_fn_with_inline_bound() {
    let p = parse_ok("fn id<T: Area>(x: T): T { x }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn");
    };
    assert_eq!(f.type_params, vec!["T".to_string()]);
    assert_eq!(f.bounds.len(), 1);
    assert_eq!(f.bounds[0].param, "T");
}

#[test]
fn generic_fn_with_where_clause() {
    let p = parse_ok("fn f<T>(x: T): T where T: Area { x }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn");
    };
    assert_eq!(f.bounds.len(), 1);
    assert_eq!(f.bounds[0].param, "T");
}

#[test]
fn struct_decl() {
    let p = parse_ok("struct User { name: string, age: number }");
    let Statement::Struct(s) = &p.statements[0] else {
        panic!("expected struct");
    };
    assert_eq!(s.name, "User");
    assert_eq!(s.fields.len(), 2);
}

#[test]
fn enum_decl_with_payloads() {
    let p = parse_ok("enum Msg { Submit(data: string, count: number), Ping }");
    let Statement::Enum(e) = &p.statements[0] else {
        panic!("expected enum");
    };
    assert_eq!(e.name, "Msg");
    assert_eq!(e.variants.len(), 2);
    assert_eq!(e.variants[0].name, "Submit");
    assert_eq!(e.variants[0].payload.as_ref().unwrap().len(), 2);
    assert!(e.variants[1].payload.is_none());
}

#[test]
fn trait_decl_and_impl() {
    let p = parse_ok(
        "trait Area { fn area(self, scale: number): number } \
         impl Area for Rectangle { fn area(self, scale: number): number { self.w * self.h * scale } }",
    );
    assert!(matches!(p.statements[0], Statement::Trait(_)));
    assert!(matches!(p.statements[1], Statement::Impl(_)));
}

#[test]
fn inherent_impl_with_self_expressions() {
    let p = parse_ok("impl Rectangle { fn area(self): number { self.w * self.h } }");
    let Statement::Impl(imp) = &p.statements[0] else {
        panic!("expected impl");
    };
    assert!(imp.is_inherent);
    assert_eq!(imp.type_name, "Rectangle");
    assert!(imp.type_params.is_empty());
    assert!(imp.type_bounds.is_empty());
    assert!(imp.type_args.is_empty());
}

#[test]
fn generic_impl_keeps_params_bounds_and_receiver_args() {
    // A generic `impl` keeps its type parameter list, its inline bounds, and
    // the receiver's written arguments — the pattern applicability decides
    // against (spec `types/traits.md`, `types/generics.md`).
    let p = parse_ok(
        "trait Area { fn area(self): number } \
         impl<T: Area> Palette<T> { fn render(self): number { 12 } }",
    );
    let Statement::Impl(imp) = &p.statements[1] else {
        panic!("expected impl");
    };
    assert!(imp.is_inherent);
    assert_eq!(imp.type_params, vec!["T".to_string()]);
    assert_eq!(imp.type_bounds.len(), 1);
    assert_eq!(imp.type_bounds[0].param, "T");
    assert_eq!(imp.type_bounds[0].traits, vec!["Area".to_string()]);
    assert!(
        matches!(&imp.type_args[..], [Type::Named(n, args)] if n == "T" && args.is_empty()),
        "receiver args: {:?}",
        imp.type_args
    );
}

#[test]
fn generic_impl_for_trait_keeps_repeated_receiver_pattern() {
    let p = parse_ok("impl<T> Show for Pair<T, T> { fn show(self): number { 1 } }");
    let Statement::Impl(imp) = &p.statements[0] else {
        panic!("expected impl");
    };
    assert!(!imp.is_inherent);
    assert_eq!(imp.trait_name, "Show");
    assert_eq!(imp.type_params, vec!["T".to_string()]);
    assert_eq!(imp.type_args.len(), 2);
    assert!(
        imp.type_args
            .iter()
            .all(|a| matches!(a, Type::Named(n, _) if n == "T")),
        "receiver args: {:?}",
        imp.type_args
    );
}

#[test]
fn type_alias() {
    let p = parse_ok("type Id = number");
    let Statement::TypeAlias(t) = &p.statements[0] else {
        panic!("expected type alias");
    };
    assert_eq!(t.name, "Id");
}

#[test]
fn tuple_type_alias() {
    let p = parse_ok("type Pair = (string, number)");
    let Statement::TypeAlias(t) = &p.statements[0] else {
        panic!("expected type alias");
    };
    assert_eq!(t.type_, Type::Tuple(vec![Type::String, Type::Number]));
}

// ---------------------------------------------------------------
// Use declarations (header forms)
// ---------------------------------------------------------------

#[test]
fn use_path_with_alias() {
    let p = parse_ok("use form::validate as v;");
    let Statement::Use(u) = &p.statements[0] else {
        panic!("expected use");
    };
    let UseTarget::Path {
        head,
        segments,
        alias,
    } = &u.target
    else {
        panic!("expected a path target, got {:?}", u.target);
    };
    assert!(matches!(head, PathHead::Name(n) if n == "form"));
    assert_eq!(segments, &["validate".to_string()]);
    assert_eq!(alias.as_deref(), Some("v"));
    assert!(!u.is_pub);
    assert!(!u.type_only);
}

#[test]
fn type_use() {
    let p = parse_ok("use type ui::User;");
    let Statement::Use(u) = &p.statements[0] else {
        panic!("expected use");
    };
    assert!(u.type_only);
    let UseTarget::Path { head, segments, .. } = &u.target else {
        panic!("expected a path target");
    };
    assert!(matches!(head, PathHead::Name(n) if n == "ui"));
    assert_eq!(segments, &["User".to_string()]);
}

#[test]
fn group_and_glob_use() {
    let p = parse_ok("use ui::{HStack, Text as T};\nuse lsp-server::*;");
    let Statement::Use(u) = &p.statements[0] else {
        panic!("expected use");
    };
    let UseTarget::Group { head, entries } = &u.target else {
        panic!("expected a group target, got {:?}", u.target);
    };
    assert!(matches!(head, PathHead::Name(n) if n == "ui"));
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].segments, ["HStack".to_string()]);
    assert_eq!(entries[0].alias, None);
    assert_eq!(entries[1].segments, ["Text".to_string()]);
    assert_eq!(entries[1].alias.as_deref(), Some("T"));
    let Statement::Use(u) = &p.statements[1] else {
        panic!("expected use");
    };
    assert!(
        matches!(&u.target, UseTarget::Glob { head, .. } if matches!(head, PathHead::Package(n) if n == "lsp-server"))
    );
}

#[test]
fn pub_use_local_form() {
    let p = parse_ok("pub use { add, PI };");
    let Statement::Use(u) = &p.statements[0] else {
        panic!("expected use");
    };
    assert!(u.is_pub);
    let UseTarget::Local(names) = &u.target else {
        panic!("expected a local target, got {:?}", u.target);
    };
    assert_eq!(names, &["add".to_string(), "PI".to_string()]);
}

#[test]
fn mod_declarations() {
    let p = parse_ok("mod forms;\npub mod widgets { fn x() {} }");
    let Statement::Mod(m) = &p.statements[0] else {
        panic!("expected mod");
    };
    assert_eq!(m.name, "forms");
    assert!(!m.is_pub);
    assert!(m.body.is_none());
    let Statement::Mod(m) = &p.statements[1] else {
        panic!("expected mod");
    };
    assert_eq!(m.name, "widgets");
    assert!(m.is_pub);
    let body = m.body.as_ref().expect("inline body");
    assert_eq!(body.len(), 1);
    assert!(matches!(body[0], Statement::Fn(_)));
}

// ---------------------------------------------------------------
// Let / const / destructuring
// ---------------------------------------------------------------

#[test]
fn let_variants() {
    let p =
        parse_ok("let x = 1 let mut y = 2 let z: string = \"a\" let w = shared [1] const K = 5");
    assert_eq!(p.statements.len(), 5);
    let x = match &p.statements[0] {
        Statement::Let(l) => l,
        _ => panic!(),
    };
    assert!(!x.is_mutable);
    let y = match &p.statements[1] {
        Statement::Let(l) => l,
        _ => panic!(),
    };
    assert!(y.is_mutable);
    let z = match &p.statements[2] {
        Statement::Let(l) => l,
        _ => panic!(),
    };
    assert_eq!(z.type_annotation, Some(Type::String));
    let w = match &p.statements[3] {
        Statement::Let(l) => l,
        _ => panic!(),
    };
    assert!(w.is_shared);
    let k = match &p.statements[4] {
        Statement::Let(l) => l,
        _ => panic!(),
    };
    assert!(k.is_const);
}

#[test]
fn walrus_sugar_is_mutable() {
    let l = first_let("let x := 1");
    assert!(l.is_mutable);
    assert!(matches!(l.value, Some(Expression::Literal { .. })));
}

#[test]
fn object_destructure() {
    let l = first_let("let { name, age } = user");
    let fields = l
        .object_destructuring
        .as_ref()
        .expect("expected object destructuring");
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].0, "name");
    assert!(fields[0].1.is_none());
}

#[test]
fn tuple_destructure() {
    let l = first_let("let (a, b) = pair");
    let names = l.tuple_names.as_ref().expect("expected tuple names");
    assert_eq!(names, &vec!["a".to_string(), "b".to_string()]);
}

// ---------------------------------------------------------------
// Statements
// ---------------------------------------------------------------

#[test]
fn assignment_forms() {
    let p = parse_ok("fn f() { x = 1 user.name = \"a\" xs[0] = 2 }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    assert_eq!(f.body.statements.len(), 3);
    assert!(
        f.body
            .statements
            .iter()
            .all(|s| matches!(s, Statement::Assign(_)))
    );
}

#[test]
fn chained_assignment_rejected() {
    let e = parse_err("fn f() { a = b = c }");
    assert!(e.message.contains("chained assignment"), "{}", e.message);
}

#[test]
fn defer_expr_and_block() {
    let p = parse_ok("fn f() { defer print(\"bye\") defer { print(\"b2\") } }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    assert_eq!(f.body.statements.len(), 2);
    assert!(
        f.body
            .statements
            .iter()
            .all(|s| matches!(s, Statement::Defer(_)))
    );
}

#[test]
fn if_else_if_chain() {
    let p = parse_ok("fn f() { if a { x() } else if b { y() } else { z() } }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    assert_eq!(f.body.statements.len(), 1);
}

#[test]
fn while_and_for() {
    let p = parse_ok("fn f() { while x < 10 { x = x + 1 } for i in 0..<10 { print(i) } }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(f.body.statements[0], Statement::While(_)));
    assert!(matches!(f.body.statements[1], Statement::For(_)));
}

#[test]
fn return_bare_and_with_value() {
    let p = parse_ok("fn f(): number { return } fn g(): number { return 1 }");
    assert_eq!(p.statements.len(), 2);
}

#[test]
fn semicolon_is_optional() {
    let p = parse_ok("fn f() { print(1); print(2) }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    match &f.body.statements[0] {
        Statement::Expr(es) => assert!(es.has_semicolon),
        other => panic!("expected expr stmt, got {other:?}"),
    }
    match &f.body.statements[1] {
        Statement::Expr(es) => assert!(!es.has_semicolon),
        other => panic!("expected expr stmt, got {other:?}"),
    }
}

// ---------------------------------------------------------------
// Expressions
// ---------------------------------------------------------------

#[test]
fn precedence_power_binds_tighter_than_mul() {
    let l = first_let("let x = 1 + 2 * 3 ** 2");
    let Some(Expression::BinaryOp(b)) = &l.value else {
        panic!("expected binary op, got {:?}", l.value);
    };
    assert!(matches!(b.operator, xulo_core::ast::BinaryOperator::Add));
    // right side must be 2 * 3 ** 2 → Mul with Pow inside
    let Expression::BinaryOp(mul) = &b.right else {
        panic!("expected mul on the right");
    };
    assert!(matches!(mul.operator, xulo_core::ast::BinaryOperator::Mul));
    let Expression::BinaryOp(pow) = &mul.right else {
        panic!("expected pow");
    };
    assert!(matches!(pow.operator, xulo_core::ast::BinaryOperator::Pow));
}

#[test]
fn power_is_right_assoc() {
    let l = first_let("let x = 2 ** 3 ** 2");
    let Some(Expression::BinaryOp(b)) = &l.value else {
        panic!();
    };
    let Expression::BinaryOp(inner) = &b.right else {
        panic!("right-assoc: right side must be another pow");
    };
    assert!(matches!(
        inner.operator,
        xulo_core::ast::BinaryOperator::Pow
    ));
}

#[test]
fn relational_is_non_assoc() {
    parse_err("fn f() { let x = a < b < c }");
}

#[test]
fn nullish_and_logical() {
    let p = parse_ok("let x = a ?? b let y = a and b or c");
    assert_eq!(p.statements.len(), 2);
}

#[test]
fn ternary_vs_postfix_propagation() {
    let p = parse_ok("let x = c ? a : b let y = f()?");
    assert_eq!(p.statements.len(), 2);
    let y = match &p.statements[1] {
        Statement::Let(l) => l,
        _ => panic!(),
    };
    assert!(matches!(y.value, Some(Expression::Propagate { .. })));
}

#[test]
fn is_and_is_not() {
    let p = parse_ok("fn f(x: number): bool { return x is number and x is not string }");
    assert_eq!(p.statements.len(), 1);
}

#[test]
fn unary_operators() {
    // spec: ! - ~ await move copy (there is no `not` operator)
    parse_ok("let a = !true let b = -1 let c = ~5 let d = move x let e = copy y");
}

#[test]
fn member_index_and_optional_chaining() {
    let p = parse_ok("let a = user.name let b = xs[0] let c = user?.name");
    let c = match &p.statements[2] {
        Statement::Let(l) => l,
        _ => panic!(),
    };
    let Some(Expression::Member(m)) = &c.value else {
        panic!("expected member, got {:?}", c.value);
    };
    assert!(m.optional);
}

#[test]
fn numbers_hex_bin_oct() {
    let _p = parse_ok("let a = 0x1f let b = 0b1010 let c = 0o17 let d = 3.14");
    fn num(src: &str) -> f64 {
        let Statement::Let(l) = &parse_ok(src).statements[0] else {
            panic!();
        };
        match &l.value {
            Some(Expression::Literal {
                value: Literal::Number(n),
                ..
            }) => *n,
            other => panic!("expected number, got {other:?}"),
        }
    }
    assert_eq!(num("let a = 0x1f"), 31.0);
    assert_eq!(num("let b = 0b1010"), 10.0);
    assert_eq!(num("let c = 0o17"), 15.0);
    assert_eq!(num("let d = 2.5"), 2.5);
}

#[test]
fn string_escapes() {
    let l = first_let(r#"let s = "a\nb\tA""#);
    let Some(Expression::Literal {
        value: Literal::String(s),
        ..
    }) = &l.value
    else {
        panic!("expected string");
    };
    assert_eq!(s, "a\nb\tA");
}

#[test]
fn quotes_of_the_other_kind_are_literal_text() {
    // The two quote forms are interchangeable (spec: expressions/literals.md),
    // so neither may be read as a delimiter inside the other.
    let a = first_let(r#"let s = "it's""#);
    let Some(Expression::Literal {
        value: Literal::String(s),
        ..
    }) = &a.value
    else {
        panic!("expected string");
    };
    assert_eq!(s, "it's");

    let b = first_let(r#"let t = 'say "hi"'"#);
    let Some(Expression::Literal {
        value: Literal::String(t),
        ..
    }) = &b.value
    else {
        panic!("expected string");
    };
    assert_eq!(t, r#"say "hi""#);
}

#[test]
fn template_desugars_to_concat() {
    let l = first_let("let s = `a${b}c`");
    // old lowering: ("a" + str(b)) + "c"
    fn adds(e: &Expression) -> usize {
        match e {
            Expression::BinaryOp(b) => adds(&b.left) + adds(&b.right) + 1,
            _ => 0,
        }
    }
    assert_eq!(adds(&l.value.expect("value")), 2, "two concatenations");
}

#[test]
fn closures() {
    let p = parse_ok(
        "let f = fn(a: number): number { a } let g = (a, b) => a + b let h = x => { print(x) } let i = async () => 1",
    );
    assert_eq!(p.statements.len(), 4);
}

#[test]
fn list_with_spread_and_trailing_comma() {
    let l = first_let("let xs = [1, 2, ...rest, ]");
    let Some(Expression::Literal {
        value: Literal::List(items),
        ..
    }) = &l.value
    else {
        panic!("expected list");
    };
    assert_eq!(items.len(), 3);
    assert!(matches!(items[2], Expression::Spread { .. }));
}

#[test]
fn brace_map_with_spread() {
    let p = parse_ok("let m = { a: 1, ...base, b: 2 }");
    assert_eq!(p.statements.len(), 1);
}

#[test]
fn typed_map() {
    let p = parse_ok("let m = Map<string, number>{ \"a\": 1 }");
    let Statement::Let(l) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(l.value, Some(Expression::TypedMap(_))));
}

#[test]
fn tuple_vs_group() {
    let t = first_let("let t = (1, 2, 3)");
    assert!(matches!(t.value, Some(Expression::Tuple { .. })));
    let g = first_let("let g = (1)");
    assert!(matches!(
        g.value,
        Some(Expression::Literal {
            value: Literal::Number(_),
            ..
        })
    ));
}

#[test]
fn single_element_tuple_rejected() {
    parse_err("let t = (1,)");
}

#[test]
fn variant_paths() {
    let p = parse_ok("let c = Color::Red let r = Result2::Ok(1)");
    let c = match &p.statements[0] {
        Statement::Let(l) => l,
        _ => panic!(),
    };
    assert!(matches!(c.value, Some(Expression::EnumRef(_))));
    let r = match &p.statements[1] {
        Statement::Let(l) => l,
        _ => panic!(),
    };
    assert!(matches!(r.value, Some(Expression::Call(_))));
}

#[test]
fn spawn_and_lock() {
    let p = parse_ok(
        "fn f() { let t = spawn async { work() } let u = spawn .thread async { work() } lock counter { counter = counter + 1 } }",
    );
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(f.body.statements[2], Statement::Expr(_)));
}

#[test]
fn panic_expression() {
    parse_ok("fn f() { panic(\"boom\") }");
}

#[test]
fn self_is_an_ordinary_expression_name() {
    parse_ok("impl Box { fn get(self): number { self.value + 1 } }");
}

#[test]
fn nested_generics_require_space_before_gt() {
    parse_ok("fn f(x: List<List<number> >): List<List<number> > { x }");
    parse_err("fn f(x: List<List<number>>): number { 1 }");
}

#[test]
fn optional_and_fn_and_literal_types() {
    parse_ok("fn f(x: number?, y: fn(a: number): bool, z: \"tag\"): unit { }");
}

// ---------------------------------------------------------------
// Match
// ---------------------------------------------------------------

#[test]
fn match_arms_or_guard_wildcard() {
    let p = parse_ok(
        "fn f(x: number): number { match x { 0 | 1 => 2, 2...5 => 3, n if n > 5 => 4, _ => 5 } }",
    );
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    let Statement::Expr(es) = &f.body.statements[0] else {
        panic!();
    };
    let Expression::Match(m) = &es.expr else {
        panic!("expected match, got {:?}", es.expr);
    };
    assert_eq!(m.arms.len(), 4);
    assert!(m.arms[2].guard.is_some());
    match &m.arms[3].pattern {
        MatchPattern::Wildcard => {}
        other => panic!("expected wildcard, got {other:?}"),
    }
}

#[test]
fn match_arm_body_block() {
    parse_ok("fn f(x: number) { match x { 0 => { print(1) } _ => print(2) } }");
}

#[test]
fn match_enum_payload_patterns() {
    parse_ok(
        "enum Msg { Send(to: string), Ping } fn f(m: Msg): number { match m { Msg::Send(to) => 1, Msg::Ping => 2 } }",
    );
}

#[test]
fn match_type_pattern_with_binder() {
    let p = parse_ok("fn f(x: number): number { match x { string s => 1, int _ => 2, _ => 3 } }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    let Statement::Expr(es) = &f.body.statements[0] else {
        panic!();
    };
    let Expression::Match(m) = &es.expr else {
        panic!();
    };
    assert_eq!(m.arms.len(), 3);
    match &m.arms[0].pattern {
        MatchPattern::TypePattern {
            type_name, binder, ..
        } => {
            assert_eq!(type_name, "string");
            assert_eq!(
                *binder,
                xulo_core::ast::PatternBinder::Binding("s".to_string())
            );
        }
        other => panic!("expected type pattern, got {other:?}"),
    }
}

#[test]
fn match_body_shapes() {
    let p = parse_ok("fn f(x: number) { match x { 0 => print(0), _ => { print(1) } } }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    let Statement::Expr(es) = &f.body.statements[0] else {
        panic!();
    };
    let Expression::Match(m) = &es.expr else {
        panic!();
    };
    assert!(matches!(m.arms[0].value, MatchBody::Expr(_)));
    assert!(matches!(m.arms[1].value, MatchBody::Block(_)));
}

// ---------------------------------------------------------------
// Components and UI
// ---------------------------------------------------------------

#[test]
fn component_fn_with_state_and_children() {
    let src = r#"fn Counter(): View {
  @State let count: number = 0
  VStack(spacing: 8) {
    Text(str(count))
    Button("Inc", onClick: fn() { count = count + 1 })
  }
}"#;
    let p = parse_ok(src);
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    assert_eq!(f.body.statements.len(), 2);
    assert!(matches!(f.body.statements[0], Statement::State(_)));
    match &f.body.statements[1] {
        Statement::Component(c) => {
            assert_eq!(c.name, "VStack");
            assert_eq!(c.args.len(), 1);
            assert_eq!(c.children.len(), 2);
        }
        other => panic!("expected component, got {other:?}"),
    }
}

#[test]
fn ui_block_children_for_and_if() {
    let src = "fn V(): View { VStack { for x in xs { Text(x) } if ok { Text(\"y\") } } }";
    let p = parse_ok(src);
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    let Statement::Component(c) = &f.body.statements[0] else {
        panic!("expected component body, got {:?}", f.body.statements[0]);
    };
    assert_eq!(c.children.len(), 2);
    assert!(matches!(c.children[0], UiElement::For { .. }));
    assert!(matches!(c.children[1], UiElement::If { .. }));
}

#[test]
fn bare_component_call_without_args() {
    let p = parse_ok("fn V(): View { Card { Text(\"hi\") } }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    let Statement::Component(c) = &f.body.statements[0] else {
        panic!();
    };
    assert_eq!(c.name, "Card");
    assert_eq!(c.children.len(), 1);
}

#[test]
fn component_decl_variants() {
    let src = "fn V(): View { @State let n: number = 0 @Store let { count } = store @Environment let theme: string @Effect fn() { print(n) }, [n] Text(str(n)) }";
    let p = parse_ok(src);
    let Statement::Fn(f) = &p.statements[0] else {
        panic!();
    };
    assert!(matches!(f.body.statements[0], Statement::State(_)));
    assert!(matches!(f.body.statements[1], Statement::Store(_)));
    assert!(matches!(f.body.statements[2], Statement::Environment(_)));
    assert!(matches!(f.body.statements[3], Statement::Effect(_)));
    assert!(matches!(f.body.statements[4], Statement::Component(_)));
}

#[test]
fn if_condition_block_is_not_swallowed_by_component_call() {
    // regression: a bare identifier + block in a condition must be `if`'s block
    parse_ok("fn f() { if x { g() } else { h() } }");
}

// ---------------------------------------------------------------
// Negative cases
// ---------------------------------------------------------------

#[test]
fn legacy_syntax_rejected() {
    parse_err("try { f() } catch (e) { g() }");
    parse_err("export fn f() { }");
    parse_err("fn f(): async { }");
    parse_err("class");
}

#[test]
fn reserved_words_are_not_identifiers() {
    parse_err("let class = 1");
    parse_err("let protocol = 1");
}

#[test]
fn untyped_parameters_rejected() {
    parse_err("fn f(x) { }");
}

#[test]
fn parameter_mode_follows_the_colon() {
    parse_ok("fn f(x: mut number, y: shared string) { }");
    parse_err("fn f(mut x: number) { }");
}

#[test]
fn expression_statement_must_not_start_with_brace() {
    parse_err("fn f() { { a: 1 } }");
}

#[test]
fn missing_initializer_rejected() {
    parse_err("let x =");
}

// ---------------------------------------------------------------
// Depth limit
// ---------------------------------------------------------------

#[test]
fn depth_just_under_limit_accepted() {
    // test threads have a small default stack; the CLI parses on the main
    // thread (8 MiB), so give this check a comparable stack.
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let depth = MAX_NEST_DEPTH - 28; // leave room for `let x = ` + `1`
            let src = format!("let x = {}1{}", "(".repeat(depth), ")".repeat(depth));
            assert!(parse_program(&src).is_ok());
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn depth_way_over_limit_rejected_without_panic() {
    let src = format!("let x = {}1{}", "(".repeat(10_000), ")".repeat(10_000));
    let e = parse_err(&src);
    assert_eq!(e.message, "nesting is too deep");
}

#[test]
fn deep_adversarial_input_does_not_panic() {
    // no panic on inputs that previously crashed naive recursive parsers
    for src in [
        "(".repeat(500),
        format!("{}1{}", "(".repeat(300), ")".repeat(300)),
        format!("{}{}", "[".repeat(400), "]".repeat(400)),
        format!("{}{}", "{".repeat(400), "}".repeat(400)),
    ] {
        let _ = parse_program(&src); // must not panic
    }
}

// ---------------------------------------------------------------
// Regressions: whitespace and token-boundary fixes
// ---------------------------------------------------------------

#[test]
fn template_keeps_spaces_around_interpolation() {
    // The space after `}` and after the opening backtick belongs to the
    // template text, not to inter-token whitespace skipping.
    let l = first_let("let s = `${i} times 5`");
    let Some(Expression::BinaryOp(b)) = &l.value else {
        panic!("expected concat, got {:?}", l.value);
    };
    let Expression::Literal {
        value: Literal::String(right),
        ..
    } = &b.right
    else {
        panic!("expected trailing string, got {:?}", b.right);
    };
    assert_eq!(right, " times 5");

    let l = first_let("let s = ` leading`");
    let Some(Expression::Literal {
        value: Literal::String(text),
        ..
    }) = &l.value
    else {
        panic!("expected plain string, got {:?}", l.value);
    };
    assert_eq!(text, " leading");
}

#[test]
fn template_escaped_backtick_and_dollar_do_not_close() {
    let l = first_let(r#"let s = `a\${not interp}\``"#);
    let Some(Expression::Literal {
        value: Literal::String(text),
        ..
    }) = &l.value
    else {
        panic!("expected plain string, got {:?}", l.value);
    };
    assert_eq!(text, "a${not interp}`");
}

#[test]
fn variant_path_as_call_argument() {
    // `argument`'s optional `name:` prefix must not swallow the first colon of
    // a `::` variant path.
    parse_ok("fn main() { print(Foo::Bar) }");
    parse_ok("fn main() { g(Foo::Bar, 1) }");
}

#[test]
fn optional_type_postfix() {
    let p = parse_ok("let x: number? = null let ys: List<number>? = null");
    let Statement::Let(l) = &p.statements[0] else {
        panic!("expected let");
    };
    assert_eq!(
        l.type_annotation,
        Some(Type::Optional(Box::new(Type::Number)))
    );
}

/// Regression: the statement rule must parse its expression exactly once.
/// When an expression-statement alternative and an assignment alternative
/// each re-parsed the whole expression, a deep `if`/`else` chain backtracked
/// exponentially (40 levels took minutes); this input now fails in
/// milliseconds either way.
#[test]
fn deep_else_chain_does_not_backtrack_exponentially() {
    let src = format!(
        "fn main(): View {{ Screen {{ {} Text(\"x\") {} }} }}",
        "if true { ".repeat(40),
        " } else { ".repeat(39) + " }"
    );
    // Either outcome is fine; what matters is that this returns at all.
    let _ = parse_program(&src);
}

/// spec `modules/import-export.md`: a path that stops at its head binds the
/// namespace only under `as` — `use lib;` without one is a syntax error.
#[test]
fn head_only_use_requires_as_alias() {
    let err = parse_err("use lib;\nfn main() {}");
    assert!(
        err.message.contains("expected `as`"),
        "message: {}",
        err.message
    );
}
