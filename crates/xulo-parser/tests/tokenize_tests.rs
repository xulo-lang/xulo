use xulo_lexer::token::{RESERVED_WORDS, Token, Token::*};
use xulo_parser::OPERATORS;
use xulo_parser::tokenize;

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
    // `number` is one of the spec's 61 keywords, not an identifier.
    let tokens = tokenize("fn main(): number { }").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Fn, Ident, LParen, RParen, Colon, Keyword, LBrace, RBrace, EOF
        ]
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
fn hash_attribute_opener_lexes() {
    // `#` is the attribute opener (spec `grammar.md` `Attribute`), a token of
    // its own so the formatter can keep `#[name]` glued.
    let toks = tokenize("#[test] fn t() {}").unwrap();
    let kinds: Vec<Token> = toks.iter().map(|t| t.kind).collect();
    assert_eq!(kinds[0], Hash);
    assert_eq!(kinds[1], LBracket);
    assert_eq!(kinds[2], Ident);
    assert_eq!(kinds[3], RBracket);
    assert!(matches!(kinds[4], Fn));
}

#[test]
fn rejects_garbage() {
    // `#` is the attribute sigil and lexes; a non-ASCII glyph does not.
    assert!(tokenize("let § = 1").is_err());
}

#[test]
fn string_escapes() {
    let tokens = tokenize(r#" "a\"b" 'c\'d' "#).unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![String, String, EOF]);
}

#[test]
fn unicode_escape_accepts_scalar_values() {
    let tokens = tokenize(r#" "\u{1F600}" "\u0041}" "#).unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![String, String, EOF]);
}

#[test]
fn unicode_escape_rejects_out_of_range_and_surrogates() {
    for bad in [
        "\\u{110000}",
        "\\u{FFFFFFFF}",
        "\\uD800",
        "\\u{DFFF}",
        "\\uDC00",
    ] {
        let src = format!("\"{bad}\"");
        let err = tokenize(&src).unwrap_err();
        assert!(
            err.kind == xulo_core::error::ErrorKind::Lex,
            "{bad}: unexpected {}",
            err.message
        );
    }
    assert!(tokenize(r#" "\u{10FFFF}" "#).is_ok());
    assert!(tokenize(r#" "\u{1F600}" "#).is_ok());
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
    assert_eq!(err.kind, xulo_core::error::ErrorKind::Lex);
    assert!(err.message.contains("unterminated block comment"));
}

#[test]
fn tokenizes_reserved_words() {
    // A representative sample of the spec's reserved list: they must lex to
    // `Reserved`, never to an identifier. `struct` is a keyword, not reserved,
    // and `spawn` is a keyword too (spec `lexical-structure.md`).
    let tokens = tokenize("class switch case yield export _").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![Reserved, Reserved, Reserved, Reserved, Reserved, Ident, EOF]
    );
    assert_eq!(tokens[0].text, "class");
}

/// The specification's closed keyword set (spec `lexical-structure.md`, 61
/// words). Keep this in step with `grammar.pest`'s `kw_word` rule.
const SPEC_KEYWORDS: &[&str] = &[
    "and", "as", "async", "await", "bool", "break", "const", "continue", "copy", "defer", "else",
    "enum", "extern", "f32", "f64", "false", "float", "fn", "for", "i8", "i16", "i32", "i64", "if",
    "impl", "in", "int", "is", "isize", "let", "lock", "match", "mod", "move", "mut", "not",
    "number", "null", "or", "panic", "pub", "return", "self", "shared", "spawn", "string",
    "struct", "super", "trait", "true", "type", "u8", "u16", "u32", "u64", "unit", "unknown",
    "use", "usize", "where", "while",
];

#[test]
fn every_spec_keyword_lexes_as_a_keyword() {
    assert_eq!(SPEC_KEYWORDS.len(), 61);
    for word in SPEC_KEYWORDS {
        let tokens = tokenize(word).unwrap();
        assert_eq!(tokens.len(), 2, "input {word:?}");
        assert!(
            tokens[0].kind != Ident && tokens[0].kind != Reserved,
            "{word:?} lexed as {:?}",
            tokens[0].kind
        );
    }
}

#[test]
fn every_spec_reserved_word_lexes_as_reserved() {
    // Spec `lexical-structure.md` §Reserved Words; every keyword is excluded.
    let reserved = "abstract actor arguments associatedtype bench case cfg channel class \
                    debugger default deinit delete derive do doc eval export extension \
                    fallthrough final function generator generic global guard init \
                    instanceof interface isolated iterator lazy library local macro meta \
                    module new open override package priv protocol receiver ref rethrows \
                    select sender static switch task this typealias unowned unsafe var \
                    virtual void weak with yield";
    let words: Vec<&str> = reserved.split_whitespace().collect();
    assert_eq!(words.len(), 61);
    assert_eq!(RESERVED_WORDS.len(), words.len());
    for word in words {
        assert!(!SPEC_KEYWORDS.contains(&word), "{word:?} is a keyword");
        assert!(RESERVED_WORDS.contains(&word), "{word:?} is not listed");
        let tokens = tokenize(word).unwrap();
        assert_eq!(tokens[0].kind, Reserved, "{word:?}");
        assert_eq!(tokens[0].text, word);
    }
    // No keyword may be reserved: they are separate closed sets.
    for word in SPEC_KEYWORDS {
        assert!(!RESERVED_WORDS.contains(word), "{word:?} is reserved");
    }
}

#[test]
fn removed_constructs_lex_as_ordinary_names() {
    // `try`/`catch`/`throw` have no syntax, and `import`/`from` are ordinary
    // identifiers again (spec `lexical-structure.md`).
    let tokens = tokenize("try catch throw import from").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Ident, Ident, Ident, Ident, Ident, EOF]);
}

#[test]
fn tokenizes_new_keywords() {
    let tokens = tokenize("const a = null").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Const, Ident, Assign, Null, EOF]);
}

#[test]
fn tokenizes_primitive_type_keywords() {
    // The primitive type names are keywords. `boolean`, `object`, and `list`
    // are not in the language, so they stay ordinary identifiers.
    let tokens = tokenize("string number bool int float unit unknown boolean object list").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Keyword, Keyword, Keyword, Keyword, Keyword, Keyword, Keyword, Ident, Ident, Ident, EOF
        ]
    );
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
    let tokens = tokenize("a.b c?.d e ?? f g..<h h...i ...j").unwrap();
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
            Ident,
            Ellipsis,
            Ident,
            Ellipsis,
            Ident,
            EOF
        ]
    );
}

#[test]
fn tokenizes_closed_range_after_number() {
    // `0..=9` is `0` + `..=` + `9` (the third mark must not be re-read as a
    // member access, and the number lexer must not swallow `..` as a fraction).
    let tokens = tokenize("0..=9").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Number, RangeOp, Number, EOF]);
}

#[test]
fn spread_ellipsis_after_number_stays_ellipsis() {
    // `...` is only the spread prefix now; lexically it stays one token.
    let tokens = tokenize("0...9").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Number, Ellipsis, Number, EOF]);
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
fn tokenizes_async_and_visibility_keywords() {
    // `try`/`catch`/`throw`/`import`/`from` were removed from the language, so
    // they are ordinary identifiers now (spec `lexical-structure.md`).
    let tokens = tokenize(
        "async fn f(): async { await g() } try { throw 1 } catch (e) { } \
         import { a as b } from \"./m\" pub fn main() { }",
    )
    .unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Async, Fn, Ident, LParen, RParen, Colon, Async, LBrace, Await, Ident, LParen, RParen,
            RBrace, Ident, LBrace, Ident, Number, RBrace, Ident, LParen, Ident, RParen, LBrace,
            RBrace, Ident, LBrace, Ident, As, Ident, RBrace, Ident, String, Pub, Fn, Ident, LParen,
            RParen, LBrace, RBrace, EOF
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
            Pub, Fn, Ident, LParen, Ident, Colon, Keyword, RParen, Colon, Keyword, LBrace, Return,
            Ident, RBrace, EOF
        ]
    );
}

#[test]
fn tokenizes_use_keyword() {
    let tokens = tokenize("pub use { a, b }").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![Pub, Use, LBrace, Ident, Comma, Ident, RBrace, EOF]
    );
}

#[test]
fn removed_export_lexes_as_reserved() {
    let tokens = tokenize("export fn main() { }").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds[0], Reserved);
    assert_eq!(tokens[0].text, "export");
}

#[test]
fn tokenizes_dollar_operator() {
    let tokens = tokenize("$name").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Dollar, Ident, EOF]);
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
fn integer_and_float_forms_lex() {
    for good in ["0x1f", "0X1F", "0b1010", "0o77", "3.14"] {
        let tokens = tokenize(good).unwrap();
        let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
        assert_eq!(kinds, vec![Number, EOF], "input {good:?}");
    }
}

#[test]
fn exponent_number_forms_are_diagnostics() {
    // Exponent notation does not exist (spec: expressions/literals.md), and a
    // number glued to an identifier is not a literal: located lex errors, not
    // crashes.
    for bad in ["1e3", "1e999", "1a", "0x1fg"] {
        let err = tokenize(bad).unwrap_err();
        assert_eq!(err.kind, xulo_core::error::ErrorKind::Lex, "input {bad:?}");
        assert!(err.span.is_some(), "input {bad:?} must carry a span");
    }
}

#[test]
fn unterminated_string_is_a_located_lex_error() {
    for bad in ["\"oops", "'nope", "\u{1f600}\""] {
        let err = tokenize(bad).unwrap_err();
        assert_eq!(err.kind, xulo_core::error::ErrorKind::Lex);
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
fn string_and_number_diagnostics_name_the_real_cause() {
    // An invalid escape is reported as such, not as "unterminated".
    let err = tokenize(r#""a\q""#).unwrap_err();
    assert!(
        err.message.contains("invalid escape sequence `\\q`"),
        "got: {}",
        err.message
    );
    // A number followed by junk reports the whole literal, with the span
    // covering it (`1e5` used to blame the leading `1`).
    let err = tokenize("1e5").unwrap_err();
    assert!(
        err.message.contains("invalid number literal `1e5`"),
        "got: {}",
        err.message
    );
    let src = "let x = 1e5";
    let err = tokenize(src).unwrap_err();
    let literal = &src[err.span.clone().expect("span")];
    assert_eq!(literal, "1e5", "span must cover the whole literal");
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

#[test]
fn skips_utf8_bom() {
    let toks = tokenize("\u{feff}let x = 1").unwrap();
    assert_eq!(toks[0].kind, Token::Let);
    assert_eq!(toks[1].kind, Token::Ident);
}

#[test]
fn plain_backtick_template_is_a_single_chunk() {
    let tokens = tokenize("`abc`").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![TChunk, EOF]);
    assert_eq!(tokens[0].text, "abc");
}

#[test]
fn empty_template_is_a_single_empty_chunk() {
    let tokens = tokenize("``").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![TChunk, EOF]);
    assert_eq!(tokens[0].text, "");
}

#[test]
fn template_interpolation_token_run() {
    let tokens = tokenize("`a${x}b`").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![TChunk, InterpOpen, Ident, InterpClose, TChunk, EOF]
    );
    assert_eq!(tokens[0].text, "a");
    assert_eq!(tokens[2].text, "x");
    assert_eq!(tokens[4].text, "b");
}

#[test]
fn template_interpolation_spans_are_absolute() {
    // ` a ${ x } b `
    let tokens = tokenize("`a${x}b`").unwrap();
    assert_eq!(tokens[0].span, 1..2); // chunk "a"
    assert_eq!(tokens[1].span, 2..4); // `${`
    assert_eq!(tokens[2].span, 4..5); // `x`
    assert_eq!(tokens[3].span, 5..6); // `}`
    assert_eq!(tokens[4].span, 6..7); // chunk "b"
    assert_eq!(tokens[5].span, 8..8); // EOF
}

#[test]
fn template_desugars_each_section() {
    let tokens = tokenize("`a${x}b${2+3}c`").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TChunk,
            InterpOpen,
            Ident,
            InterpClose,
            TChunk,
            InterpOpen,
            Number,
            Plus,
            Number,
            InterpClose,
            TChunk,
            EOF
        ]
    );
}

#[test]
fn template_counts_braces_inside_expressions() {
    let tokens = tokenize("`a${ {b:1}.b }c`").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TChunk,
            InterpOpen,
            LBrace,
            Ident,
            Colon,
            Number,
            RBrace,
            Dot,
            Ident,
            InterpClose,
            TChunk,
            EOF
        ]
    );
}

#[test]
fn template_nested_quotes_are_opaque() {
    let tokens = tokenize("`x${ \"}\" }y`").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![TChunk, InterpOpen, String, InterpClose, TChunk, EOF]
    );
}

#[test]
fn template_nested_backticks_lex_their_own_interpolation() {
    let tokens = tokenize("`x${ `y${z}` }w`").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            TChunk,      // "x"
            InterpOpen,  // outer ${ ... } opens
            TChunk,      // nested "y"
            InterpOpen,  // nested ${ opens
            Ident,       // z
            InterpClose, // nested } closes
            TChunk,      // nested trailing ""
            InterpClose, // outer } closes
            TChunk,      // "w"
            EOF
        ]
    );
}

#[test]
fn template_escapes() {
    let tokens = tokenize("`\\`` + `\\$\\\\\\n\\t`").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![TChunk, Plus, TChunk, EOF]);
    assert_eq!(tokens[0].text, r"\`");
    assert_eq!(tokens[2].text, r"\$\\\n\t");
}

#[test]
fn template_multiline_is_allowed() {
    let tokens = tokenize("`line1\nline2`").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![TChunk, EOF]);
}

#[test]
fn template_dollar_brace_escape_stays_literal() {
    let tokens = tokenize(r#"`\${x}`"#).unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![TChunk, EOF]);
    assert_eq!(tokens[0].text, r"\${x}");
}

#[test]
fn unterminated_template_is_an_error() {
    let err = tokenize("`abc${x}").unwrap_err();
    assert_eq!(err.kind, xulo_core::error::ErrorKind::Lex);
    assert!(err.message.contains("unterminated template"));
    assert!(tokenize("`abc").is_err());
}

#[test]
fn invalid_template_escape_is_an_error() {
    let err = tokenize(r#"`a\qb`"#).unwrap_err();
    assert_eq!(err.kind, xulo_core::error::ErrorKind::Lex);
    assert!(err.message.contains("invalid escape"));
    let err = tokenize(r#"`a\u{}`"#).unwrap_err();
    assert_eq!(err.kind, xulo_core::error::ErrorKind::Lex);
}

#[test]
fn quoted_strings_do_not_interpolate() {
    for src in [r#""a${x}b""#, r#"'a${x}b'"#] {
        let tokens = tokenize(src).unwrap();
        let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
        assert_eq!(kinds, vec![String, EOF], "input {src:?}");
    }
}

#[test]
fn print_and_println_are_ordinary_names() {
    // They are prelude intrinsics, not keywords (spec `lexical-structure.md`).
    let tokens = tokenize("print(x) println(y)").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![
            Ident, LParen, Ident, RParen, Ident, LParen, Ident, RParen, EOF
        ]
    );
    assert_eq!(tokens[0].text, "print");
    assert_eq!(tokens[4].text, "println");
}

#[test]
fn break_and_continue_are_keywords() {
    let tokens = tokenize("break continue").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Break, Continue, EOF]);
}

#[test]
fn modulo_operator() {
    let tokens = tokenize("a % b").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Ident, Modulo, Ident, EOF]);
}

#[test]
fn power_operator() {
    let tokens = tokenize("a ** b").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Ident, Power, Ident, EOF]);
}

#[test]
fn power_does_not_consume_single_star() {
    let tokens = tokenize("a * b").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Ident, Star, Ident, EOF]);
}

#[test]
fn break_continue_in_loop_context() {
    let tokens = tokenize("for i in list { break continue }").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(
        kinds,
        vec![For, Ident, In, Ident, LBrace, Break, Continue, RBrace, EOF]
    );
}

#[test]
fn bitwise_not_token() {
    let tokens = tokenize("~x").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Tilde, Ident, EOF]);
}

#[test]
fn bitwise_xor_token() {
    let tokens = tokenize("a ^ b").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Ident, Xor, Ident, EOF]);
}

#[test]
fn shift_left_two_tokens() {
    let tokens = tokenize("a << b").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Ident, Lt, Lt, Ident, EOF]);
}

#[test]
fn shift_right_two_tokens() {
    let tokens = tokenize("a >> b").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Ident, Gt, Gt, Ident, EOF]);
}

#[test]
fn bitwise_and_token() {
    let tokens = tokenize("a & b").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Ident, Amp, Ident, EOF]);
}

#[test]
fn bitwise_or_token() {
    let tokens = tokenize("a | b").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Ident, Pipe, Ident, EOF]);
}

#[test]
fn operators_two_char_first() {
    let tokens = tokenize("a == b <= c => d").unwrap();
    let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds, vec![Ident, Eq, Ident, Lte, Ident, Arrow, Ident, EOF]);
}

/// `token_operator` in `grammar.pest` decides which spellings are operators;
/// `OPERATORS` decides
/// what each one means. This fails if the two ever drift apart.
#[test]
fn operator_table_matches_the_grammar() {
    let grammar = include_str!("../grammar.pest");
    let body = grammar
        .split("token_operator = @{")
        .nth(1)
        .expect("grammar has a token_operator rule")
        .split("\n}")
        .next()
        .expect("token_operator rule is terminated");
    let mut spellings: Vec<&str> = body.split('"').skip(1).step_by(2).collect();
    spellings.sort_unstable();

    let mut table: Vec<&str> = OPERATORS.iter().map(|(spelling, _)| *spelling).collect();
    table.sort_unstable();

    assert_eq!(spellings, table);
}

#[test]
fn every_operator_spelling_lexes_to_its_token() {
    for &(spelling, expected) in OPERATORS {
        let tokens = tokenize(spelling).unwrap();
        let kinds: Vec<Token> = tokens.iter().map(|t| t.kind).collect();
        assert_eq!(kinds, vec![expected, EOF], "spelling {spelling:?}");
    }
}
