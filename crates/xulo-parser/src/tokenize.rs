//! The token layer: source text to a token stream.
//!
//! The rules live in the `token` / `trivia` entry rules of `grammar.pest`
//! (section 10), alongside the syntax rules the parser uses, so lexing and
//! parsing can never disagree about what a token is. This module is the driver
//! around them: skip trivia, match one token at the cursor, classify it, and
//! turn a rule that does not match into a located [`XuloError`] instead of a
//! panic.
//!
//! Template literals stay a hand-written scanner below: each `${…}` section is
//! re-lexed with [`tokenize`], so braces, nested strings and backticks nest the
//! same way anywhere in the file.

use pest::Parser;

use xulo_core::error::{ErrorKind, XuloError};
use xulo_lexer::token::{LexedToken, Token};

use super::{Rule, XuloParser};

type Input<'i> = &'i str;

/// Every operator spelling the grammar accepts, paired with its token.
///
/// `token_operator` in `grammar.pest` decides *what matches*, this table
/// decides *what it means*. The `operator_table_matches_the_grammar` test
/// reads the grammar file and fails if the two ever drift apart.
pub const OPERATORS: &[(&str, Token)] = &[
    ("..<", Token::RangeOp),
    ("...", Token::Ellipsis),
    ("**", Token::Power),
    ("==", Token::Eq),
    ("!=", Token::Neq),
    ("<=", Token::Lte),
    (">=", Token::Gte),
    ("=>", Token::Arrow),
    (":=", Token::ColonAssign),
    ("::", Token::DoubleColon),
    ("?.", Token::QuestionDot),
    ("??", Token::Nullish),
    ("=", Token::Assign),
    ("+", Token::Plus),
    ("-", Token::Minus),
    ("*", Token::Star),
    ("/", Token::Slash),
    ("%", Token::Modulo),
    ("^", Token::Xor),
    ("~", Token::Tilde),
    ("<", Token::Lt),
    (">", Token::Gt),
    ("|", Token::Pipe),
    ("&", Token::Amp),
    ("?", Token::Question),
    ("!", Token::Bang),
    (".", Token::Dot),
    (":", Token::Colon),
    ("@", Token::At),
    ("#", Token::Hash),
    ("$", Token::Dollar),
    (";", Token::Semicolon),
    (",", Token::Comma),
    ("(", Token::LParen),
    (")", Token::RParen),
    ("{", Token::LBrace),
    ("}", Token::RBrace),
    ("[", Token::LBracket),
    ("]", Token::RBracket),
];

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
        // Remember where trivia started: an unterminated block comment is
        // reported from there to the end of the file, not just from the `/*`.
        let at = total - cursor.len();
        if let Some(consumed) = matches_rule(cursor, Rule::trivia) {
            cursor = &cursor[consumed..];
        }
        // `trivia` stops at a `/*` that never closes; that is a truncated
        // comment rather than a `/` followed by a `*`.
        if cursor.starts_with("/*") {
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
        match lex_one(source, start) {
            Some((kind, len)) => {
                let text = &source[start..start + len];
                tokens.push(LexedToken::new(kind, text, start..start + len));
                cursor = &source[start + len..];
            }
            None => return Err(diagnose(source, start, total, first)),
        }
    }

    tokens.push(LexedToken::new(Token::EOF, "", total..total));
    Ok(tokens)
}

/// Explain a position that no token rule accepted.
///
/// Every user-facing lexer message lives here: the grammar decides what
/// matches, this decides how a miss is reported.
fn diagnose(source: &str, start: usize, total: usize, first: Option<char>) -> XuloError {
    match first {
        None => XuloError::new(ErrorKind::Lex, "unexpected end of input").at(start..total),
        Some('"' | '\'') => string_diagnostic(source, start),
        Some(c) if c.is_ascii_digit() => {
            // `1e5`, `1a`: the digits matched before the trailing junk was
            // rejected, so blame the whole run rather than the first digit.
            let literal: String = source[start..]
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '.' || *ch == '_')
                .collect();
            XuloError::new(
                ErrorKind::Lex,
                format!("invalid number literal `{literal}`"),
            )
            .at(start..start + literal.len())
        }
        Some(c) => {
            let bad = &source[start..start + c.len_utf8()];
            XuloError::new(ErrorKind::Lex, format!("unexpected character `{bad}`"))
                .at(start..start + bad.len())
        }
    }
}

/// Match one token at `source[start..]`.
///
/// `None` means "nothing usable here": a string escape the Unicode range
/// rejects, or no token at all. The caller turns that into the located
/// diagnostic.
fn lex_one(source: &str, start: usize) -> Option<(Token, usize)> {
    let mut pairs = XuloParser::parse(Rule::token, &source[start..]).ok()?;
    let mut inner = pairs.next()?.into_inner();
    let matched = inner.next()?;
    let text = matched.as_str();
    let kind = match matched.as_rule() {
        Rule::string_literal => {
            if !unicode_escapes_ok(text) {
                return None;
            }
            Token::String
        }
        Rule::number_literal => Token::Number,
        Rule::token_ident => Token::from_keyword(text).unwrap_or(Token::Ident),
        Rule::token_operator => operator_token(text)?,
        other => unreachable!("token layer matched unexpected rule {other:?}"),
    };
    Some((kind, text.len()))
}

/// How many bytes `rule` consumes at the very start of `input`, or `None`
/// when it does not match there at all.
fn matches_rule(input: &str, rule: Rule) -> Option<usize> {
    XuloParser::parse(rule, input)
        .ok()?
        .next()
        .map(|pair| pair.as_str().len())
}

/// Look a matched operator spelling up in [`OPERATORS`].
fn operator_token(text: &str) -> Option<Token> {
    OPERATORS
        .iter()
        .find(|(spelling, _)| *spelling == text)
        .map(|(_, token)| *token)
}

/// Check every `\u` escape of a lexed string literal.
///
/// The grammar only matches an escape's *shape*; naming a Unicode scalar
/// value (`<= U+10FFFF`, never a surrogate) is a numeric comparison no PEG
/// can express, so it happens here. A rejection falls back to
/// [`string_diagnostic`], which is also where the hand-written scanner used
/// to send these inputs.
fn unicode_escapes_ok(text: &str) -> bool {
    let mut rest = text;
    while let Some(pos) = rest.find('\\') {
        let after = &rest[pos + 1..];
        let mut chars = after.chars();
        match chars.next() {
            // `chars` has just consumed the `u`, so `as_str()` is the payload.
            Some('u') => match scalar_payload(chars.as_str()) {
                Some(consumed) => rest = &after[1 + consumed..],
                None => return false,
            },
            // `\"`, `\\`, `\n`, … - the shape is the grammar's business.
            Some(c) => rest = &after[c.len_utf8()..],
            None => return true,
        }
    }
    true
}

/// Check the payload of a `\u` escape (`{…}` or exactly four hex digits) and
/// return how many bytes it consumed. `None` when it is malformed or does not
/// name a Unicode scalar value.
fn scalar_payload(payload: &str) -> Option<usize> {
    if let Some(rest) = payload.strip_prefix('{') {
        let digits_end = rest
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(rest.len());
        let digits = &rest[..digits_end];
        if digits.is_empty() || digits.len() > 6 || !rest[digits_end..].starts_with('}') {
            return None;
        }
        return is_scalar(digits).then_some(digits_end + 2);
    }
    let digits = payload.get(..4)?;
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    is_scalar(digits).then_some(4)
}

/// `digits` is a hexadecimal scalar value: within range, no surrogates.
fn is_scalar(digits: &str) -> bool {
    match u32::from_str_radix(digits, 16) {
        Ok(value) => value <= 0x10_FFFF && !(0xD800..=0xDFFF).contains(&value),
        Err(_) => false,
    }
}

/// Classify a failed string literal so the diagnostic says *why* it failed:
/// an invalid escape sequence, or an unterminated literal. The string rule
/// only backtracks, so this rescan distinguishes the common cases.
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

/// Consume a `\uXXXX` or `\u{...}` escape (after the `\u` has been consumed).
///
/// Used by the template scanner, which walks raw text rather than going
/// through the grammar.
fn consume_unicode_escape(input: &mut Input) -> Result<(), ()> {
    if input.starts_with('{') {
        bump(input);
        let mut digits = 0;
        let mut value: u32 = 0;
        loop {
            let Some(h) = input.chars().next() else {
                return Err(());
            };
            if h == '}' {
                if digits == 0 {
                    return Err(());
                }
                // A `\u{...}` escape must name a valid Unicode scalar value
                // (<= U+10FFFF); surrogates are likewise invalid in isolation.
                if value > 0x10_FFFF || (0xD800..=0xDFFF).contains(&value) {
                    return Err(());
                }
                bump(input);
                return Ok(());
            }
            if !h.is_ascii_hexdigit() || digits >= 6 {
                return Err(());
            }
            value = value * 16 + h.to_digit(16).unwrap_or(0);
            bump(input);
            digits += 1;
        }
    }
    let mut value: u32 = 0;
    for _ in 0..4 {
        let Some(h) = input.chars().next() else {
            return Err(());
        };
        if !h.is_ascii_hexdigit() {
            return Err(());
        }
        value = value * 16 + h.to_digit(16).unwrap_or(0);
        bump(input);
    }
    // `\uD800`-`\uDFFF` are surrogate code points, not valid scalar values.
    if (0xD800..=0xDFFF).contains(&value) {
        return Err(());
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

fn bump(input: &mut Input) -> Option<char> {
    let c = input.chars().next()?;
    *input = &input[c.len_utf8()..];
    Some(c)
}
