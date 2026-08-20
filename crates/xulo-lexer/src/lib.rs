pub mod token;

use std::ops::Range;

use winnow::ascii::{digit1, multispace1};
use winnow::combinator::preceded;
use winnow::error::{ContextError, ErrMode, ParserError};
use winnow::prelude::*;
use winnow::token::{literal, take_while};

use xulo_core::error::{ErrorKind, XuloError};

use self::token::{LexedToken, Token};

type Input<'i> = &'i str;
type Res<O> = winnow::ModalResult<O, ContextError>;

/// Tokenize a Xulo source file into a token stream (with an explicit trailing
/// `EOF` token). A UTF-8 byte-order mark at the start is skipped (editors on
/// Windows commonly prepend one; treating it as a character yields a
/// confusing "unexpected character" error).
pub fn tokenize(source: &str) -> Result<Vec<LexedToken>, XuloError> {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    let total = source.len();
    let mut cursor: Input = source;
    let mut tokens = Vec::new();

    loop {
        let at = total - cursor.len();
        if let Err(ErrMode::Cut(_)) = ws_or_comment(&mut cursor) {
            return Err(XuloError::new(ErrorKind::Lex, "unterminated block comment").at(at..total));
        }
        if cursor.is_empty() {
            break;
        }
        let start = total - cursor.len();
        let first = cursor.chars().next();
        if first == Some('`') {
            lex_template(&mut cursor, total, &mut tokens)?;
            continue;
        }
        match lex_token(&mut cursor, total) {
            Ok(tok) => tokens.push(tok),
            Err(_) => {
                return Err(match first {
                    None => {
                        XuloError::new(ErrorKind::Lex, "unexpected end of input").at(start..total)
                    }
                    Some('"' | '\'') => string_diagnostic(source, start),
                    Some(c) if c.is_ascii_digit() => {
                        // `1e5`, `1a`, `0x1f`: the lexer consumed the leading
                        // digits before failing; report the whole run with an
                        // honest message instead of blaming the first digit.
                        let run: String = cursor
                            .chars()
                            .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '.' || *ch == '_')
                            .collect();
                        let literal = format!("{c}{run}");
                        XuloError::new(
                            ErrorKind::Lex,
                            format!("invalid number literal `{literal}`"),
                        )
                        .at(start..start + literal.len())
                    }
                    Some(c) => {
                        let len = c.len_utf8();
                        let bad = &cursor[..len.min(cursor.len())];
                        XuloError::new(ErrorKind::Lex, format!("unexpected character `{bad}`"))
                            .at(start..start + bad.len())
                    }
                });
            }
        }
    }

    tokens.push(LexedToken::new(Token::EOF, "", total..total));
    Ok(tokens)
}

/// Classify a failed string literal so the diagnostic says *why* it failed:
/// an invalid escape sequence, or an unterminated literal. The string parser
/// returns a bare backtrack, so this rescan distinguishes the common cases.
fn string_diagnostic(source: &str, start: usize) -> XuloError {
    let mut chars = source[start..].chars().peekable();
    let quote = chars.next().unwrap_or('"');
    let mut escaped = false;
    let mut pos = start + quote.len_utf8();
    for c in chars {
        if escaped {
            if !matches!(c, '"' | '\'' | '\\' | 'n' | 't' | 'r' | 'u') {
                let span = start..pos + c.len_utf8();
                return XuloError::new(ErrorKind::Lex, format!("invalid escape sequence `\\{c}`"))
                    .at(span);
            }
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == quote {
            let span = start..pos + c.len_utf8();
            return XuloError::new(ErrorKind::Lex, "invalid string literal").at(span);
        } else if c == '\n' {
            let span = start..pos;
            return XuloError::new(ErrorKind::Lex, "unterminated string literal").at(span);
        }
        pos += c.len_utf8();
    }
    XuloError::new(ErrorKind::Lex, "unterminated string literal").at(start..source.len())
}

fn lex_token(input: &mut Input<'_>, total: usize) -> Res<LexedToken> {
    let before = *input;
    let first = input.chars().next().ok_or_else(|| backtrack(input))?;
    let kind_token = match first {
        '"' | '\'' => {
            string(input)?;
            Token::String
        }
        c if c.is_ascii_digit() => {
            number(input)?;
            Token::Number
        }
        c if c.is_ascii_alphabetic() || c == '_' => {
            ident(input)?;
            let text = text_of(before, input);
            Token::from_keyword(text).unwrap_or(Token::Ident)
        }
        _ => operator(input)?,
    };
    let text = text_of(before, input);
    let span = span_at(total, before, input);
    Ok(LexedToken::new(kind_token, text.to_string(), span))
}

/// Skip whitespace, `//` line comments, and `/* ... */` block comments.
/// An unterminated block comment is reported as a `Cut` error.
fn ws_or_comment(input: &mut Input) -> Res<()> {
    loop {
        let before = *input;
        let _: Res<&str> = multispace1(input);
        let _: Res<&str> =
            preceded(literal("//"), take_while(0.., |c: char| c != '\n')).parse_next(input);
        if input.starts_with("/*") {
            block_comment(input)?;
        }
        if *input == before {
            break;
        }
    }
    Ok(())
}

/// Consume a `/* ... */` block comment; `Cut` if unterminated.
fn block_comment(input: &mut Input) -> Res<()> {
    bump(input);
    bump(input);
    loop {
        let Some(c) = bump(input) else {
            return Err(ErrMode::Cut(ContextError::default()));
        };
        if c == '*' && input.starts_with('/') {
            bump(input);
            return Ok(());
        }
    }
}

fn ident(input: &mut Input) -> Res<()> {
    let Some(first) = input.chars().next() else {
        return Err(backtrack(input));
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(backtrack(input));
    }
    bump(input);
    let _ = take_while(0.., |c: char| c.is_ascii_alphanumeric() || c == '_').parse_next(input)?;
    Ok(())
}

fn number(input: &mut Input) -> Res<()> {
    digit1(input)?;
    if input.starts_with('.') {
        let tail = &input[1..];
        if tail.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            let mut rest = tail;
            digit1(&mut rest)?;
            *input = rest;
        }
    }
    if input
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(backtrack(input));
    }
    Ok(())
}

fn string(input: &mut Input) -> Res<()> {
    let Some(q) = input.chars().next() else {
        return Err(backtrack(input));
    };
    if q != '"' && q != '\'' {
        return Err(backtrack(input));
    }
    bump(input);
    loop {
        let Some(c) = input.chars().next() else {
            return Err(backtrack(input));
        };
        if c == q {
            bump(input);
            return Ok(());
        }
        if c == '\n' {
            return Err(backtrack(input));
        }
        bump(input);
        if c == '\\' {
            let Some(e) = input.chars().next() else {
                return Err(backtrack(input));
            };
            if !matches!(e, '"' | '\'' | '\\' | 'n' | 't' | 'r' | 'u') {
                return Err(backtrack(input));
            }
            bump(input);
            if e == 'u' {
                consume_unicode_escape(input)?;
            }
        }
    }
}

/// Consume a `\uXXXX` or `\u{...}` escape (after the `\u` has been consumed).
fn consume_unicode_escape(input: &mut Input) -> Res<()> {
    if input.starts_with('{') {
        bump(input);
        let mut digits = 0;
        let mut value: u32 = 0;
        loop {
            let Some(h) = input.chars().next() else {
                return Err(backtrack(input));
            };
            if h == '}' {
                if digits == 0 {
                    return Err(backtrack(input));
                }
                // A `\u{...}` escape must name a valid Unicode scalar value
                // (≤ U+10FFFF); surrogates are likewise invalid in isolation.
                if value > 0x10_FFFF || (0xD800..=0xDFFF).contains(&value) {
                    return Err(backtrack(input));
                }
                bump(input);
                return Ok(());
            }
            if !h.is_ascii_hexdigit() || digits >= 6 {
                return Err(backtrack(input));
            }
            value = value * 16 + h.to_digit(16).unwrap_or(0);
            bump(input);
            digits += 1;
        }
    }
    let mut value: u32 = 0;
    for _ in 0..4 {
        let Some(h) = input.chars().next() else {
            return Err(backtrack(input));
        };
        if !h.is_ascii_hexdigit() {
            return Err(backtrack(input));
        }
        value = value * 16 + h.to_digit(16).unwrap_or(0);
        bump(input);
    }
    // `\uD800`-`\uDFFF` are surrogate code points, not valid scalar values.
    if (0xD800..=0xDFFF).contains(&value) {
        return Err(backtrack(input));
    }
    Ok(())
}

/// Lex a backtick template literal (JS-style `` `text ${expr} text` ``).
///
/// Emits a fresh token stream: one `TChunk` per literal text run (interior raw
/// slice, unescaped by the parser) and `InterpOpen`/`InterpClose` sentinels
/// bracketing the re-lexed tokens of every `${expr}` section. Double- and
/// single-quoted strings never interpolate; backtick text may span lines.
fn lex_template(
    input: &mut Input<'_>,
    total: usize,
    tokens: &mut Vec<LexedToken>,
) -> Result<(), XuloError> {
    let source = *input;
    let base = total - source.len();
    let mut pos = base + 1;
    let mut chunk_start = base + 1;
    let mut expr_start = 0usize;
    let mut depth = 0usize;
    let mut in_expr = false;

    let rest = |pos: usize| &source[pos - base..];

    loop {
        let cur = rest(pos);
        let Some(c) = cur.chars().next() else {
            return Err(
                XuloError::new(ErrorKind::Lex, "unterminated template literal").at(base..total),
            );
        };
        if in_expr {
            match c {
                '{' => {
                    depth += 1;
                    pos += c.len_utf8();
                }
                '}' if depth == 0 => {
                    let inner = &source[expr_start - base..pos - base];
                    let lexed = tokenize(inner).map_err(|mut e| {
                        if let Some(span) = &mut e.span {
                            span.start += expr_start;
                            span.end += expr_start;
                        }
                        e
                    })?;
                    for t in lexed {
                        if t.kind == Token::EOF {
                            continue;
                        }
                        tokens.push(LexedToken::new(
                            t.kind,
                            t.text,
                            t.span.start + expr_start..t.span.end + expr_start,
                        ));
                    }
                    tokens.push(LexedToken::new(Token::InterpClose, "}", pos..pos + 1));
                    in_expr = false;
                    chunk_start = pos + 1;
                    pos += c.len_utf8();
                }
                '}' => {
                    depth -= 1;
                    pos += c.len_utf8();
                }
                // Nested strings / template literals inside the expression are
                // opaque to the outer scan (braces inside them are their own).
                '"' | '\'' | '`' => {
                    pos += 1;
                    loop {
                        let cur2 = rest(pos);
                        let Some(n) = cur2.chars().next() else {
                            return Err(XuloError::new(
                                ErrorKind::Lex,
                                "unterminated template literal",
                            )
                            .at(base..total));
                        };
                        if n == '\\' {
                            let Some(e) = cur2[1..].chars().next() else {
                                return Err(XuloError::new(
                                    ErrorKind::Lex,
                                    "unterminated template literal",
                                )
                                .at(base..total));
                            };
                            pos += 1 + e.len_utf8();
                            continue;
                        }
                        pos += n.len_utf8();
                        if n == c {
                            break;
                        }
                    }
                }
                _ => {
                    pos += c.len_utf8();
                }
            }
        } else {
            match c {
                '`' => {
                    let text = &source[chunk_start - base..pos - base];
                    tokens.push(LexedToken::new(Token::TChunk, text, chunk_start..pos));
                    *input = rest(pos + 1);
                    return Ok(());
                }
                '\\' => {
                    let Some(e) = cur[1..].chars().next() else {
                        return Err(XuloError::new(
                            ErrorKind::Lex,
                            "unterminated template literal",
                        )
                        .at(base..total));
                    };
                    match e {
                        '`' | '\\' | '$' | 'n' | 't' | 'r' | '\'' | '"' => {
                            pos += 1 + e.len_utf8();
                        }
                        'u' => {
                            let mut inner: Input = &cur[2..];
                            if consume_unicode_escape(&mut inner).is_err() {
                                return Err(XuloError::new(
                                    ErrorKind::Lex,
                                    "invalid escape sequence",
                                )
                                .at(pos..pos + 2));
                            }
                            pos += 2 + (cur.len() - 2 - inner.len());
                        }
                        _ => {
                            let bad = rest(pos);
                            return Err(XuloError::new(ErrorKind::Lex, "invalid escape sequence")
                                .at(pos..pos + bad.chars().next().map_or(1, |c| c.len_utf8())));
                        }
                    }
                }
                '$' if cur[1..].starts_with('{') => {
                    let text = &source[chunk_start - base..pos - base];
                    tokens.push(LexedToken::new(Token::TChunk, text, chunk_start..pos));
                    tokens.push(LexedToken::new(Token::InterpOpen, "${", pos..pos + 2));
                    in_expr = true;
                    depth = 0;
                    expr_start = pos + 2;
                    pos += 2;
                }
                _ => {
                    pos += c.len_utf8();
                }
            }
        }
    }
}

/// Manually match punctuation and operator symbols.
fn operator(input: &mut Input) -> Res<Token> {
    let mut it = input.chars();
    let (c1, c2, c3) = (it.next(), it.next(), it.next());
    let c1 = c1.ok_or_else(|| backtrack(input))?;
    let (tok, len) = match (c1, c2, c3) {
        ('.', Some('.'), Some('<')) => (Token::RangeOp, 3),
        ('.', Some('.'), Some('.')) => (Token::Ellipsis, 3),
        ('=', Some('='), _) => (Token::Eq, 2),
        ('!', Some('='), _) => (Token::Neq, 2),
        ('<', Some('='), _) => (Token::Lte, 2),
        ('>', Some('='), _) => (Token::Gte, 2),
        ('=', Some('>'), _) => (Token::Arrow, 2),
        (':', Some(':'), _) => (Token::DoubleColon, 2),
        ('?', Some('.'), _) => (Token::QuestionDot, 2),
        ('?', Some('?'), _) => (Token::Nullish, 2),
        ('=', _, _) => (Token::Assign, 1),
        ('+', _, _) => (Token::Plus, 1),
        ('-', _, _) => (Token::Minus, 1),
        ('*', _, _) => (Token::Star, 1),
        ('/', _, _) => (Token::Slash, 1),
        ('<', _, _) => (Token::Lt, 1),
        ('>', _, _) => (Token::Gt, 1),
        ('|', _, _) => (Token::Pipe, 1),
        ('&', _, _) => (Token::Amp, 1),
        ('?', _, _) => (Token::Question, 1),
        ('!', _, _) => (Token::Bang, 1),
        ('.', _, _) => (Token::Dot, 1),
        (':', _, _) => (Token::Colon, 1),
        ('@', _, _) => (Token::At, 1),
        ('$', _, _) => (Token::Dollar, 1),
        (';', _, _) => (Token::Semicolon, 1),
        (',', _, _) => (Token::Comma, 1),
        ('(', _, _) => (Token::LParen, 1),
        (')', _, _) => (Token::RParen, 1),
        ('{', _, _) => (Token::LBrace, 1),
        ('}', _, _) => (Token::RBrace, 1),
        ('[', _, _) => (Token::LBracket, 1),
        (']', _, _) => (Token::RBracket, 1),
        _ => return Err(backtrack(input)),
    };
    *input = &input[len..];
    Ok(tok)
}

fn bump(input: &mut Input) -> Option<char> {
    let c = input.chars().next()?;
    *input = &input[c.len_utf8()..];
    Some(c)
}

fn text_of<'i>(before: &'i str, after: &str) -> &'i str {
    &before[..before.len() - after.len()]
}

fn span_at(total: usize, before: &str, after: &str) -> Range<usize> {
    let start = total - before.len();
    let end = total - after.len();
    start..end
}

fn backtrack(input: &Input) -> ErrMode<ContextError> {
    ErrMode::Backtrack(ContextError::from_input(input))
}

#[cfg(test)]
mod tests {
    use super::token::Token;
    use super::*;

    fn kinds(src: &str) -> Vec<Token> {
        tokenize(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn literals() {
        use Token::*;
        assert_eq!(
            kinds(r#"let x = "hi" + 42;"#),
            vec![Let, Ident, Assign, String, Plus, Number, Semicolon, EOF]
        );
    }

    #[test]
    fn keywords_and_types() {
        use Token::*;
        assert_eq!(
            kinds("fn main(): number { }"),
            vec![Fn, Ident, LParen, RParen, Colon, Ident, LBrace, RBrace, EOF]
        );
    }

    #[test]
    fn comments_skipped() {
        use Token::*;
        assert_eq!(
            kinds("// header\nlet x = 1 // trailing"),
            vec![Let, Ident, Assign, Number, EOF]
        );
    }

    #[test]
    fn operators_two_char_first() {
        use Token::*;
        assert_eq!(
            kinds("a == b <= c => d"),
            vec![Ident, Eq, Ident, Lte, Ident, Arrow, Ident, EOF]
        );
    }

    #[test]
    fn string_escapes() {
        use Token::*;
        assert_eq!(kinds(r#" "a\"b" 'c\'d' "#), vec![String, String, EOF]);
    }

    #[test]
    fn spans_are_byte_offsets() {
        let toks = tokenize("let x = 12").unwrap();
        assert_eq!(toks[0].span, 0..3); // 'let'
        assert_eq!(toks[3].span, 8..10); // '12'
        assert_eq!(toks[4].span, 10..10); // EOF
    }

    #[test]
    fn errors_on_garbage() {
        assert!(tokenize("let # = 1").is_err());
    }
}
