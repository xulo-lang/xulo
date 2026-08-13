use xulo::lexer::token::{Token, Token::*};
use xulo::lexer::tokenize;

#[test]
fn tokenizes_literals() {
    let tokens = tokenize(r#"let x = "hi" + 42;"#).unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![Let, Ident, Assign, String, Plus, Number, Semicolon, EOF]
    );
}

#[test]
fn tokenizes_keywords_and_types() {
    let tokens = tokenize("fn main(): number { }").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![Fn, Ident, LParen, RParen, Colon, Ident, LBrace, RBrace, EOF]
    );
}

#[test]
fn skips_comments() {
    let tokens = tokenize("// hi\nlet x = 1 // trailing").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Let, Ident, Assign, Number, EOF]);
}

#[test]
fn tracks_byte_spans() {
    let tokens = tokenize("let x = 12").unwrap();
    assert_eq!(tokens[0].span, 0..3); // let
    assert_eq!(tokens[3].span, 8..10); // 12
    assert_eq!(tokens[4].span, 10..10); // EOF
}

#[test]
fn rejects_garbage() {
    assert!(tokenize("let @ = 1").is_err());
}

#[test]
fn string_escapes() {
    let tokens = tokenize(r#" "a\"b" 'c\'d' "#).unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![String, String, EOF]);
}

#[test]
fn block_comments_are_skipped() {
    let tokens = tokenize("let x = 1 /* ignored */ let y = 2").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![Let, Ident, Assign, Number, Let, Ident, Assign, Number, EOF]
    );
}

#[test]
fn unterminated_block_comment_is_an_error() {
    let err = tokenize("let x = 1 /* oops").unwrap_err();
    assert_eq!(err.kind, xulo::error::ErrorKind::Lex);
    assert!(err.message.contains("unterminated block comment"));
}

#[test]
fn tokenizes_new_keywords() {
    let tokens = tokenize("const a = null").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Const, Ident, Assign, Null, EOF]);
}

#[test]
fn tokenizes_type_keywords_as_idents() {
    let tokens = tokenize("string number boolean object list").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![Ident, Ident, Ident, Ident, Ident, EOF]
    );
}

#[test]
fn tokenizes_new_symbols() {
    let tokens = tokenize("a | b & c ? d ! e :: f").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![Ident, Pipe, Ident, Amp, Ident, Question, Ident, Bang, Ident, DoubleColon, Ident, EOF]
    );
}

#[test]
fn tokenizes_phase2_keywords() {
    let tokens = tokenize("while match and or").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![While, Match, And, Or, EOF]);
}

#[test]
fn tokenizes_phase2_symbols() {
    let tokens = tokenize("a.b c?.d e ?? f g..<h ...i").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Ident, Dot, Ident,
            Ident, QuestionDot, Ident,
            Ident, Nullish, Ident,
            Ident, RangeOp, Ident,
            Ellipsis, Ident,
            EOF
        ]
    );
}

#[test]
fn range_in_for_is_lexed() {
    let tokens = tokenize("for i in 0..<10 { }").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![For, Ident, In, Number, RangeOp, Number, LBrace, RBrace, EOF]
    );
}

#[test]
fn tokenizes_phase3_keywords() {
    let tokens = tokenize(
        "async fn f(): async { await g() } try { throw 1 } catch (e) { } \
         import { a as b } from \"./m\" import * as ns from \"./n\" \
         import type { T } from \"./t\" export default fn main() { }",
    )
    .unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Async, Fn, Ident, LParen, RParen, Colon, Async, LBrace, Await, Ident, LParen, RParen,
            RBrace,
            Try, LBrace, Throw, Number, RBrace, Catch, LParen, Ident, RParen, LBrace, RBrace,
            Import, LBrace, Ident, As, Ident, RBrace, From, String,
            Import, Star, As, Ident, From, String,
            Import, Type, LBrace, Ident, RBrace, From, String,
            Export, Default, Fn, Ident, LParen, RParen, LBrace, RBrace,
            EOF
        ]
    );
}
