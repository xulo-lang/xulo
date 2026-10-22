//! FFI declaration parsing (spec `grammar.md` ExternBlock/ExternItem/
//! ExternFnDecl, `ffi.md`): accept shapes, lower them to the AST, and pin
//! the syntactic rejections (body/mode/defaults/attributes/async on the
//! extern forms, missing `pub`, non-literal ABI tags).

use xulo_core::ast::{Statement, Type};
use xulo_parser::{parse_program, tokenize};
fn parse_ok(src: &str) -> xulo_core::ast::Program {
    parse_program(src).unwrap_or_else(|e| panic!("expected OK for {src:?}: {}", e.message))
}

fn parse_err(src: &str) -> xulo_core::error::XuloError {
    parse_program(src).expect_err(&format!("expected error for {src:?}"))
}

fn extern_block(src: &str) -> xulo_core::ast::ExternBlockDecl {
    let p = parse_ok(src);
    match &p.statements[0] {
        Statement::ExternBlock(b) => b.clone(),
        other => panic!("expected extern block, got {other:?}"),
    }
}

// ---------------------------------------------------------------
// ExternBlock / ExternItem
// ---------------------------------------------------------------

#[test]
fn extern_block_parses_import_item() {
    let b = extern_block(r#"extern "C" { fn abs(x: i32): i32; }"#);
    assert!(!b.is_pub);
    assert_eq!(b.abi, "C");
    assert_eq!(b.items.len(), 1);
    let item = &b.items[0];
    assert_eq!(item.name, "abs");
    assert_eq!(item.local_name, "abs");
    assert_eq!(item.params.len(), 1);
    assert_eq!(item.params[0].name, "x");
    assert_eq!(
        item.params[0].type_annotation,
        Some(Type::Named("i32".into(), vec![]))
    );
    assert_eq!(item.return_type, Some(Type::Named("i32".into(), vec![])));
    assert!(item.params[0].default.is_none());
    assert!(item.params[0].mode.is_none());
}

#[test]
fn extern_item_as_alias_binds_locally() {
    let src = r#"extern "C" { fn log_message(msg: string) as puts; }"#;
    let b = extern_block(src);
    let item = &b.items[0];
    assert_eq!(item.name, "log_message");
    assert_eq!(item.local_name, "puts");
    assert_eq!(&src[item.local_name_span.clone()], "puts");
    assert_ne!(item.local_name_span, item.name_span);
    assert!(item.return_type.is_none());
}

#[test]
fn pub_extern_block_marks_every_item_public() {
    let b = extern_block(r#"pub extern "system" { fn getpid(): u32; fn abort(); }"#);
    assert!(b.is_pub);
    assert_eq!(b.abi, "system");
    assert_eq!(b.items.len(), 2);
}

#[test]
fn extern_block_accepts_trailing_param_comma_and_empty_body() {
    let b = extern_block(r#"extern "C" { fn f(a: i32, b: i64,); }"#);
    assert_eq!(b.items[0].params.len(), 2);
    let empty = extern_block(r#"extern "C" {}"#);
    assert!(empty.items.is_empty());
}

#[test]
fn extern_block_parses_inside_inline_mod() {
    let p = parse_ok(r#"mod m { extern "C" { fn x(): unit; } }"#);
    let Statement::Mod(m) = &p.statements[0] else {
        panic!("expected mod");
    };
    let body = m.body.as_ref().expect("inline mod has a body");
    assert!(matches!(body[0], Statement::ExternBlock(_)));
}

#[test]
fn extern_block_accepts_pointer_types() {
    let b = extern_block(r#"extern "C" { fn malloc(size: usize): Ptr; fn free(p: Ptr); }"#);
    assert_eq!(
        b.items[0].return_type,
        Some(Type::Named("Ptr".into(), vec![]))
    );
    assert_eq!(
        b.items[0].params[0].type_annotation,
        Some(Type::Named("usize".into(), vec![]))
    );
    assert_eq!(
        b.items[1].params[0].type_annotation,
        Some(Type::Named("Ptr".into(), vec![]))
    );
}

#[test]
fn ptr_question_is_optional_type() {
    let b = extern_block(r#"extern "C" { fn find(key: i32): Ptr?; }"#);
    assert_eq!(
        b.items[0].return_type,
        Some(Type::Optional(Box::new(Type::Named("Ptr".into(), vec![]))))
    );
}

#[test]
fn extern_items_accept_spec_examples_without_semicolons() {
    // Every example in spec/ffi.md omits the item terminator; grammar.md's
    // ExternItem includes it. Both spellings parse.
    let b = extern_block(
        "extern \"C\" {\n  fn abs(x: i32): i32\n  fn getpid(): i32\n  fn malloc(size: usize): Ptr? as cMalloc\n  fn free(p: Ptr?) as cFree\n}",
    );
    assert_eq!(b.items.len(), 4);
    assert_eq!(b.items[2].local_name, "cMalloc");
    let mixed = extern_block(r#"extern "C" { fn abs(x: i32): i32; fn getpid(): i32 }"#);
    assert_eq!(mixed.items.len(), 2);
}

// ---------------------------------------------------------------
// ExternFnDecl (export form)
// ---------------------------------------------------------------

#[test]
fn pub_extern_fn_lowers_to_export_fn_with_abi() {
    let p = parse_ok(r#"pub extern "C" fn add(a: i32, b: i32): i32 { return a + b }"#);
    let Statement::Export(export) = &p.statements[0] else {
        panic!("expected export");
    };
    let xulo_core::ast::ExportItem::Fn(f) = &export.item else {
        panic!("expected export fn");
    };
    assert_eq!(f.name, "add");
    assert_eq!(f.abi.as_deref(), Some("C"));
    assert_eq!(f.params.len(), 2);
    assert_eq!(f.return_type, Some(Type::Named("i32".into(), vec![])));
    assert!(!f.is_async);
    assert!(f.attributes.is_empty());
    assert!(!f.body.statements.is_empty());
}

#[test]
fn pub_extern_fn_without_return_type_parses() {
    let p = parse_ok(r#"pub extern "C" fn noop() { }"#);
    let Statement::Export(export) = &p.statements[0] else {
        panic!("expected export");
    };
    let xulo_core::ast::ExportItem::Fn(f) = &export.item else {
        panic!("expected export fn");
    };
    assert!(f.return_type.is_none());
    assert_eq!(f.abi.as_deref(), Some("C"));
}

// ---------------------------------------------------------------
// Syntactic rejections
// ---------------------------------------------------------------

#[test]
fn extern_fn_requires_pub() {
    parse_err(r#"extern "C" fn f() { }"#);
}

#[test]
fn extern_fn_requires_block() {
    parse_err(r#"pub extern "C" fn f();"#);
}

#[test]
fn extern_fn_rejects_async_and_attributes() {
    parse_err(r#"pub extern "C" async fn f() { }"#);
    parse_err(r#"#Foo pub extern "C" fn f() { }"#);
}

#[test]
fn extern_item_rejects_body_mode_and_defaults() {
    // An ExternItem has no Block: it ends in `;`.
    parse_err(r#"extern "C" { fn f() {} }"#);
    // ExternParam carries no mode and no default.
    parse_err(r#"extern "C" { fn f(mut x: i32); }"#);
    parse_err(r#"extern "C" { fn f(x: i32 = 1); }"#);
}

#[test]
fn abi_tag_must_be_a_string_literal() {
    parse_err(r#"extern C { fn f(); }"#);
    parse_err(r#"pub extern 42 fn f() { }"#);
}

#[test]
fn extern_not_allowed_in_statement_position() {
    parse_err("fn main() { extern \"C\" { fn f(); } }");
}

#[test]
fn new_keywords_are_not_identifiers() {
    parse_err("let usize = 1");
    parse_err("let isize = 1");
    parse_err("fn usize() { }");
    // Word boundary: `external` keeps its tail and stays an identifier.
    let p = parse_ok("let external = 1");
    assert!(matches!(p.statements[0], Statement::Let(_)));
}

#[test]
fn usize_and_isize_are_primitive_type_keywords() {
    let p = parse_ok("fn f(): usize { return 0 }");
    let Statement::Fn(f) = &p.statements[0] else {
        panic!("expected fn");
    };
    assert_eq!(f.return_type, Some(Type::Named("usize".into(), vec![])));
    // Keywords take no type-argument list.
    parse_err("fn f(): usize<i32> { return 0 }");
}

// ---------------------------------------------------------------
// Token stream (fmt / tooling path)
// ---------------------------------------------------------------

#[test]
fn tokenize_extern_declarations() {
    let toks = tokenize(r#"pub extern "C" { fn abs(x: i32): i32; }"#).unwrap();
    let kinds: Vec<_> = toks.iter().map(|t| t.kind).collect();
    use xulo_lexer::token::Token::*;
    assert_eq!(
        kinds,
        vec![
            Pub, Ident, String, LBrace, Fn, Ident, LParen, Ident, Colon, Ident, RParen, Colon,
            Ident, Semicolon, RBrace, EOF
        ]
    );
}
