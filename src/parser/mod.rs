pub mod expression;
pub mod statement;
pub mod types;

use std::ops::Range;

use winnow::error::{AddContext, ErrMode, ModalError, ParserError};
use winnow::stream::Stream;

use crate::ast::Program;
use crate::error::{ErrorKind, XuloError};
use crate::lexer::token::{LexedToken, Token};

pub type In<'i> = &'i [LexedToken];
pub type Pr<O> = winnow::ModalResult<O, PErr>;

/// A recoverable parse error carrying a source span and message.
#[derive(Debug, Clone)]
pub struct PErr {
    pub span: Range<usize>,
    pub message: String,
}

/// `ErrMode::Backtrack` from a `PErr` for the current position.
pub fn backtrack_p(input: &In<'_>) -> winnow::error::ErrMode<PErr> {
    winnow::error::ErrMode::Backtrack(PErr::unexpected(input))
}

impl PErr {
    pub fn unexpected(input: &[LexedToken]) -> Self {
        match input.first() {
            Some(t) => PErr {
                span: t.span.clone(),
                message: format!("unexpected {}", t.kind.describe()),
            },
            None => PErr {
                span: 0..0,
                message: "unexpected end of input".into(),
            },
        }
    }
}

impl ModalError for PErr {
    fn cut(self) -> Self {
        self
    }
    fn backtrack(self) -> Self {
        self
    }
}

impl ParserError<In<'_>> for PErr {
    type Inner = Self;

    fn from_input(input: &In<'_>) -> Self {
        let slice = *input;
        PErr::unexpected(slice)
    }

    /// Pick the error that made the most progress into the input.
    fn or(self, other: Self) -> Self {
        if other.span.start > self.span.start {
            other
        } else {
            self
        }
    }

    fn into_inner(self) -> Result<Self, Self> {
        Ok(self)
    }
}

impl AddContext<In<'_>, &'static str> for PErr {
    fn add_context(
        mut self,
        input: &In<'_>,
        _token_start: &<In<'_> as Stream>::Checkpoint,
        context: &'static str,
    ) -> Self {
        if let Some(t) = input.first() {
            self.span = t.span.clone();
            self.message = format!("expected {context}, found {}", t.kind.describe());
        }
        self
    }
}

/// Parse a token stream into a `Program`. Expects the trailing `EOF` token.
pub fn parse_program(tokens: &[LexedToken]) -> Result<Program, XuloError> {
    let mut input = tokens;
    match program(&mut input) {
        Ok(p) => Ok(p),
        Err(ErrMode::Backtrack(e) | ErrMode::Cut(e)) => Err(XuloError {
            kind: ErrorKind::Parse,
            message: e.message,
            span: Some(e.span),
            file: None,
        }),
        Err(ErrMode::Incomplete(_)) => {
            Err(XuloError::new(ErrorKind::Parse, "unexpected end of input"))
        }
    }
}

/// True when the current position is the `EOF` token (or the stream ended).
pub fn at_eof(input: &In<'_>) -> bool {
    match input.first() {
        None => true,
        Some(t) => t.kind == Token::EOF,
    }
}

/// True if the next token has the given kind (does not consume).
pub fn peek_is(input: &In<'_>, kind: Token) -> bool {
    matches!(input.first(), Some(t) if t.kind == kind)
}

/// Consumes the next token iff it has the given kind. Returns `true` if consumed.
pub fn opt_tk(input: &mut In<'_>, kind: Token) -> bool {
    if peek_is(input, kind) {
        *input = &input[1..];
        true
    } else {
        false
    }
}

/// Byte span covered by the tokens consumed between `original` (input at the
/// start of a node) and `input` (input after that node). Falls back to `start`
/// when nothing was consumed.
pub fn consumed_span(original: In<'_>, input: In<'_>, start: usize) -> Range<usize> {
    let consumed = original.len().saturating_sub(input.len());
    let s = original.first().map(|t| t.span.start).unwrap_or(start);
    let e = original[..consumed].last().map(|t| t.span.end).unwrap_or(s);
    s..e
}

/// Matches a single token of the given kind, discarding its payload.
pub fn tk(input: &mut In<'_>, kind: Token) -> Pr<()> {
    match input.first() {
        Some(t) if t.kind == kind => {
            *input = &input[1..];
            Ok(())
        }
        _ => Err(ErrMode::Backtrack(PErr::unexpected(input))),
    }
}

/// Matches a single token of the given kind, returning an owned copy.
pub fn verified_tk(input: &mut In<'_>, kind: Token) -> Pr<LexedToken> {
    match input.first() {
        Some(t) if t.kind == kind => {
            let tok = t.clone();
            *input = &input[1..];
            Ok(tok)
        }
        _ => Err(ErrMode::Backtrack(PErr::unexpected(input))),
    }
}

/// Reads an ordinary identifier (`Ident`) as a `String`.
pub fn ident_name(input: &mut In<'_>) -> Pr<String> {
    verified_tk(input, Token::Ident).map(|t| t.text)
}

/// Reads either an `Ident` or the `print` keyword as a name.
pub fn name(input: &mut In<'_>) -> Pr<String> {
    let t = input
        .first()
        .cloned()
        .ok_or_else(|| ErrMode::Backtrack(PErr::unexpected(input)))?;
    match t.kind {
        Token::Ident | Token::Print => {
            *input = &input[1..];
            Ok(t.text)
        }
        _ => Err(ErrMode::Backtrack(PErr::unexpected(input))),
    }
}

/// Program: zero or more statements, then end of stream.
fn program(input: &mut In<'_>) -> Pr<Program> {
    let mut statements = Vec::new();
    while !at_eof(input) {
        statements.push(terminated_statement(input)?);
    }
    Ok(Program { statements })
}

/// Statement followed by an optional semicolon.
pub fn terminated_statement(input: &mut In<'_>) -> Pr<crate::ast::Statement> {
    let original = *input;
    let s = statement::statement(input)?;
    let has_semicolon = opt_tk(input, Token::Semicolon);
    match s {
        crate::ast::Statement::Expr(es) => Ok(crate::ast::Statement::Expr(crate::ast::ExprStmt {
            expr: es.expr,
            has_semicolon: es.has_semicolon || has_semicolon,
            span: if has_semicolon {
                consumed_span(original, input, es.span.start)
            } else {
                es.span
            },
        })),
        other => Ok(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinaryOperator, Expression, Literal, Statement};
    use crate::lexer::tokenize;

    fn parse(src: &str) -> Program {
        let toks = tokenize(src).unwrap();
        parse_program(&toks).unwrap()
    }

    fn parse_err(src: &str) -> XuloError {
        let toks = tokenize(src).unwrap();
        parse_program(&toks).unwrap_err()
    }

    #[test]
    fn parse_hello() {
        let p = parse(r#"fn main() { print("Hello, world!") }"#);
        assert_eq!(p.statements.len(), 1);
        let Statement::Fn(f) = &p.statements[0] else {
            panic!("expected fn");
        };
        assert_eq!(f.name, "main");
        assert_eq!(f.params.len(), 0);
        assert_eq!(f.body.statements.len(), 1);
    }

    #[test]
    fn parse_let_and_arith() {
        let p = parse("let x = 1 + 2 * 3");
        let Statement::Let(b) = &p.statements[0] else {
            panic!("expected let");
        };
        match &b.value {
            Some(Expression::BinaryOp(op)) => {
                assert_eq!(op.operator, BinaryOperator::Add);
                assert!(matches!(
                    op.left.clone(),
                    Expression::Literal {
                        value: Literal::Number(1.0),
                        ..
                    }
                ));
                match &op.right {
                    Expression::BinaryOp(mul) => {
                        assert_eq!(mul.operator, BinaryOperator::Mul);
                    }
                    _ => panic!("expected mul"),
                }
            }
            _ => panic!("expected binary op"),
        }
    }

    #[test]
    fn parse_if_else() {
        let p = parse("if 1 > 2 { print(\"a\") } else { print(\"b\") }");
        let Statement::Expr(es) = &p.statements[0] else {
            panic!("expected if expr");
        };
        let Expression::If(cond) = &es.expr else {
            panic!("expected if expr");
        };
        assert!(matches!(cond.condition.clone(), Expression::BinaryOp(_)));
        assert!(cond.else_branch.is_some());
    }

    #[test]
    fn parse_list_and_call() {
        let p = parse(r#"let xs = [1, 2, 3] print("hi")"#);
        let Statement::Let(b) = &p.statements[0] else {
            panic!("expected let");
        };
        assert!(matches!(
            &b.value,
            Some(Expression::Literal {
                value: Literal::List(v),
                ..
            }) if v.len() == 3
        ));
        assert!(
            matches!(&p.statements[1], Statement::Expr(es) if matches!(es.expr, Expression::Call(_)))
        );
    }

    #[test]
    fn parse_precedence() {
        // (10 - 2) * 3 vs 10 - (2 * 3): left tree for equal precedence
        let p = parse("10 - 2 - 3");
        let Statement::Expr(es) = &p.statements[0] else {
            panic!();
        };
        let Expression::BinaryOp(op) = &es.expr else {
            panic!();
        };
        assert_eq!(op.operator, BinaryOperator::Sub);
        assert!(
            matches!(op.left.clone(), Expression::BinaryOp(inner) if inner.operator == BinaryOperator::Sub)
        );
    }

    #[test]
    fn reports_syntax_error() {
        let e = parse_err("let x = ");
        assert_eq!(e.kind, ErrorKind::Parse);
    }
}
