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
    assert!(tokenize("let # = 1").is_err());
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
    assert_eq!(kinds, vec![Ident, Ident, Ident, Ident, Ident, EOF]);
}

#[test]
fn tokenizes_new_symbols() {
    let tokens = tokenize("a | b & c ? d ! e :: f").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Ident,
            Pipe,
            Ident,
            Amp,
            Ident,
            Question,
            Ident,
            Bang,
            Ident,
            DoubleColon,
            Ident,
            EOF
        ]
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
            Ident,
            Dot,
            Ident,
            Ident,
            QuestionDot,
            Ident,
            Ident,
            Nullish,
            Ident,
            Ident,
            RangeOp,
            Ident,
            Ellipsis,
            Ident,
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
            RBrace, Try, LBrace, Throw, Number, RBrace, Catch, LParen, Ident, RParen, LBrace,
            RBrace, Import, LBrace, Ident, As, Ident, RBrace, From, String, Import, Star, As,
            Ident, From, String, Import, Type, LBrace, Ident, RBrace, From, String, Export,
            Default, Fn, Ident, LParen, RParen, LBrace, RBrace, EOF
        ]
    );
}

#[test]
fn tokenizes_pub_keyword() {
    let tokens = tokenize("pub fn add(a: number): number { return a }").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Pub, Fn, Ident, LParen, Ident, Colon, Ident, RParen, Colon, Ident, LBrace, Return,
            Ident, RBrace, EOF
        ]
    );
}

#[test]
fn tokenizes_at_and_dollar() {
    let tokens = tokenize("@State let x = 0 $name").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![At, Ident, Let, Ident, Assign, Number, Dollar, Ident, EOF]
    );
}

#[test]
fn number_literal_forms_preserve_text() {
    let tokens = tokenize("3.14 42 0.5").unwrap();
    let texts: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == Number)
        .map(|t| t.text.to_string())
        .collect();
    assert_eq!(
        texts,
        vec!["3.14".to_string(), "42".to_string(), "0.5".to_string()]
    );
    // spans are raw byte offsets covering the full literal
    let spans: Vec<_> = tokens
        .iter()
        .filter(|t| t.kind == Number)
        .map(|t| t.span.clone())
        .collect();
    assert_eq!(spans, vec![0..4, 5..7, 8..11]);
}

#[test]
fn unsupported_number_forms_are_diagnostics() {
    // No hex/binary/exponent literals: these are located lex errors, not crashes.
    for bad in ["0x1f", "0b101", "1e3", "1e999"] {
        let err = tokenize(bad).unwrap_err();
        assert_eq!(err.kind, xulo::error::ErrorKind::Lex, "input {bad:?}");
        assert!(err.span.is_some(), "input {bad:?} must carry a span");
    }
}

#[test]
fn unterminated_string_is_a_located_lex_error() {
    for bad in ["\"oops", "'nope", "\"a\\q\"", "'a\\z'", "\u{1f600}\""] {
        let err = tokenize(bad).unwrap_err();
        assert_eq!(err.kind, xulo::error::ErrorKind::Lex);
        assert!(
            err.message.contains("unterminated string literal")
                || err.message.contains("unexpected character"),
            "input {bad:?}: {}",
            err.message
        );
        assert!(err.span.is_some(), "input {bad:?} must carry a span");
    }
}

#[test]
fn quotes_can_contain_the_opposite_quote() {
    let tokens = tokenize(r#""it's" 'say "hi"'"#).unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![String, String, EOF]);
}

#[test]
fn multibyte_bytes_precede_tokens_in_spans() {
    // Multibyte chars in string literals are counted as raw byte offsets.
    let tokens = tokenize(r#""世😀" + "x""#).unwrap();
    // "世😀" = quote + 3 + 4 + quote = 9 bytes; '+' at 10; final string 12..15
    assert_eq!(tokens[0].span, 0..9);
    assert_eq!(tokens[1].span, 10..11);
    assert_eq!(tokens[2].span, 12..15);
}

#[test]
fn glued_operators_lex_with_correct_spans() {
    // "a+b a==b a??b a?.b a..<b 1-2"
    let tokens = tokenize("a+b a==b a??b a?.b a..<b 1-2").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Ident,
            Plus,
            Ident,
            Ident,
            Eq,
            Ident,
            Ident,
            Nullish,
            Ident,
            Ident,
            QuestionDot,
            Ident,
            Ident,
            RangeOp,
            Ident,
            Number,
            Minus,
            Number,
            EOF
        ]
    );
    assert_eq!(tokens[1].span, 1..2); // '+' in "a+b"
    assert_eq!(tokens[4].span, 5..7); // '==' in "a==b"
    assert_eq!(tokens[7].span, 10..12); // '??' in "a??b"
    assert_eq!(tokens[10].span, 15..17); // '?.' in "a?.b"
    assert_eq!(tokens[13].span, 20..23); // '..<' in "a..<b"
}

#[test]
fn lone_operators_and_trailing_eof_are_diagnostics_not_panics() {
    for bad in ["$", "@", "\\", "？", "...", "..", "\u{0}"] {
        let _ = tokenize(bad); // must not panic; may error
    }
    assert!(tokenize("\\").is_err());
}
