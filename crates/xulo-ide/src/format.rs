use xulo_core::error::XuloError;
use xulo_lexer::{self, token::Token};

/// Format a Xulo source file: 2-space indentation, normalized spacing around
/// tokens, one statement per line, and blocks expanded across lines.
///
/// Original line breaks are preserved as a hint (statements / match arms that
/// were written on separate lines stay on separate lines).
///
/// Known limitation: comments are discarded (the lexer does not surface them).
pub fn format(source: &str) -> Result<String, XuloError> {
    let tokens = xulo_lexer::tokenize(source)?;
    let toks = &tokens[..tokens.len().saturating_sub(1)];
    let mut out = String::new();
    let mut depth: usize = 0;
    let mut at_line_start = true;
    let mut line_has_content = false;
    let mut prev: Option<Token> = None;
    let mut prev_unary = false;
    let mut inline_braces: Vec<bool> = Vec::new();

    for i in 0..toks.len() {
        let tok = &toks[i];
        let cur = tok.kind;
        let next = toks.get(i + 1).map(|t| t.kind);
        let (orig_newline, orig_blank) = if i == 0 {
            (false, false)
        } else {
            let gap = &source[toks[i - 1].span.end..tok.span.start];
            (gap.contains('\n'), gap.matches('\n').count() >= 2)
        };

        // A `{` starts an inline group (import spec / `pub use` / destructure)
        // when it directly follows `import`/`const`/`let`/`use`, e.g.
        // `import { a } from ...` or `pub use { a, b }`.
        let is_inline_open = cur == Token::LBrace
            && matches!(
                prev,
                Some(Token::Import) | Some(Token::Const) | Some(Token::Let) | Some(Token::Use)
            );

        // A `}` closes a block: drop the indent level before laying out its line.
        let closes_block = cur == Token::RBrace && prev != Some(Token::LBrace);
        let closes_inline = cur == Token::RBrace && inline_braces.last() == Some(&true);
        if closes_block {
            depth = depth.saturating_sub(1);
        }

        let mut blank_before = false;

        let mut break_before = (cur == Token::RBrace
            && prev != Some(Token::LBrace)
            && line_has_content
            && !closes_inline)
            || prev == Some(Token::Semicolon)
            || (cur != Token::RBrace
                && prev == Some(Token::LBrace)
                && inline_braces.last() != Some(&true))
            || (prev == Some(Token::RBrace)
                && !matches!(
                    cur,
                    Token::Else
                        | Token::Catch
                        | Token::Semicolon
                        | Token::RParen
                        | Token::Comma
                        | Token::Dot
                        | Token::QuestionDot
                        | Token::From
                        | Token::Assign
                ));

        if !break_before
            && cur != Token::LBrace
            && orig_newline
            && !at_line_start
            && inline_braces.last() != Some(&true)
        {
            break_before = true;
        }
        if orig_blank && !at_line_start {
            blank_before = true;
        }

        if break_before || blank_before {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            if blank_before {
                out.push('\n');
            }
            out.push_str(&"  ".repeat(depth));
            at_line_start = true;
            prev_unary = false;
        }

        let in_inline = inline_braces.last() == Some(&true) && cur != Token::RBrace;
        let space = if at_line_start {
            false
        } else {
            needs_space(prev, cur, next, prev_unary, in_inline)
        };
        if space {
            out.push(' ');
        }

        out.push_str(&tok.text);
        at_line_start = false;
        line_has_content = true;
        prev_unary = (cur == Token::Minus || cur == Token::Plus) && is_unary_context(prev);

        if cur == Token::LBrace {
            // An empty `{ }` pair is a unit: it must not change the indent
            // depth (the `{` would otherwise leak +1 because the matching `}`
            // is skipped by `closes_block`'s empty-pair check).
            if next != Some(Token::RBrace) {
                depth += 1;
            }
            inline_braces.push(is_inline_open);
        } else if cur == Token::RBrace && !inline_braces.is_empty() {
            inline_braces.pop();
        }

        prev = Some(cur);
    }

    while out.ends_with('\n') {
        out.pop();
    }
    out.push('\n');
    Ok(out)
}

/// Whether a space is required between the previous and the current token.
fn needs_space(
    prev: Option<Token>,
    cur: Token,
    next: Option<Token>,
    prev_unary: bool,
    in_inline: bool,
) -> bool {
    if prev_unary {
        return false;
    }
    if cur == Token::RBrace && prev == Some(Token::LBrace) {
        // `{ }` — a space between the pair.
        return true;
    }
    if in_inline {
        // Inside an import spec / destructure `{ a, b }`: space both sides.
        if prev == Some(Token::LBrace) || cur == Token::RBrace {
            return true;
        }
    }
    match prev {
        None => return false,
        Some(t) if no_space_after(t) => return false,
        _ => {}
    }
    if no_space_before(cur) {
        return false;
    }
    if cur == Token::LParen
        && matches!(
            prev,
            Some(Token::Ident) | Some(Token::Print) | Some(Token::Println) | Some(Token::Fn)
        )
    {
        return false;
    }
    if cur == Token::LBracket && prev == Some(Token::Ident) {
        return false;
    }
    if cur == Token::LBrace && prev == Some(Token::LParen) {
        return false;
    }
    if cur == Token::LBracket && prev == Some(Token::LParen) {
        return false;
    }
    if cur == Token::Question {
        // `a ? b : c` ternary vs `Type?` optional: a leading space is only
        // wanted when the next token starts an expression.
        return is_expr_start(next)
            && matches!(
                prev,
                Some(Token::Ident)
                    | Some(Token::Number)
                    | Some(Token::String)
                    | Some(Token::TChunk)
                    | Some(Token::Boolean)
                    | Some(Token::Null)
                    | Some(Token::RParen)
                    | Some(Token::RBracket)
            );
    }
    if (cur == Token::Minus || cur == Token::Plus) && no_space_before_unary(prev) {
        return false;
    }
    true
}

/// Contexts where a `-`/`+` token is unary and the operator binds tightly to
/// its operand, so no space precedes it (`(-5)`, `x * -1`). Compared with
/// `is_unary_context`, `=`, `return`, `,`, and `:` are excluded: there the
/// operator still reads as a binary/expression start and keeps its space
/// (`x = -5`, `return -5`, `f(a, -5)`, `{ x: -1 }`).
fn no_space_before_unary(prev: Option<Token>) -> bool {
    match prev {
        None => true,
        Some(t) => matches!(
            t,
            Token::Plus
                | Token::Minus
                | Token::Star
                | Token::Slash
                | Token::Eq
                | Token::Neq
                | Token::Lt
                | Token::Gt
                | Token::Lte
                | Token::Gte
                | Token::Arrow
                | Token::Pipe
                | Token::Amp
                | Token::Nullish
                | Token::And
                | Token::Or
                | Token::Bang
                | Token::LParen
                | Token::LBracket
                | Token::LBrace
                | Token::Question
                | Token::RangeOp
                | Token::Ellipsis
        ),
    }
}

fn no_space_after(t: Token) -> bool {
    matches!(
        t,
        Token::At
            | Token::Dollar
            | Token::LParen
            | Token::LBracket
            | Token::Dot
            | Token::QuestionDot
            | Token::DoubleColon
            | Token::Ellipsis
            | Token::Bang
            | Token::RangeOp
    )
}

fn no_space_before(t: Token) -> bool {
    matches!(
        t,
        Token::RParen
            | Token::RBracket
            | Token::Comma
            | Token::Semicolon
            | Token::Dot
            | Token::QuestionDot
            | Token::Colon
            | Token::DoubleColon
            | Token::RangeOp
            | Token::Ellipsis
    )
}

fn is_unary_context(prev: Option<Token>) -> bool {
    match prev {
        None => true,
        Some(t) => matches!(
            t,
            Token::Assign
                | Token::Plus
                | Token::Minus
                | Token::Star
                | Token::Slash
                | Token::Eq
                | Token::Neq
                | Token::Lt
                | Token::Gt
                | Token::Lte
                | Token::Gte
                | Token::Arrow
                | Token::Pipe
                | Token::Amp
                | Token::Nullish
                | Token::And
                | Token::Or
                | Token::Bang
                | Token::Comma
                | Token::LParen
                | Token::LBracket
                | Token::LBrace
                | Token::Colon
                | Token::Question
                | Token::Return
                | Token::RangeOp
                | Token::Ellipsis
        ),
    }
}

fn is_expr_start(t: Option<Token>) -> bool {
    matches!(
        t,
        Some(Token::Ident)
            | Some(Token::Number)
            | Some(Token::String)
            | Some(Token::TChunk)
            | Some(Token::Boolean)
            | Some(Token::Null)
            | Some(Token::LParen)
            | Some(Token::Bang)
            | Some(Token::At)
            | Some(Token::Minus)
            | Some(Token::Plus)
    )
}
