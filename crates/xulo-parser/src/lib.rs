//! pest.rs-based parser for Xulo.
//!
//! The grammar lives in `xulo.pest` and mirrors `spec/grammar.md`
//! production-by-production; this module wires it up, enforces the nesting
//! depth limit, converts pest diagnostics into [`XuloError`]s, and hands the
//! token tree to the AST builder in [`build`].

mod build;
mod depth;
mod expr;
mod types;

use std::ops::Range;

use pest::error::{Error as PestError, ErrorVariant, LineColLocation};
use pest::Parser;
use pest_derive::Parser;

use xulo_core::ast::Program;
use xulo_core::error::{ErrorKind, XuloError};

#[derive(Parser)]
#[grammar = "xulo.pest"]
struct XuloParser;

/// Maximum bracket nesting accepted by the pre-parse depth scan. Input deeper
/// than this is rejected with "nesting is too deep" instead of risking a
/// stack overflow in the recursive-descent parser.
pub const MAX_NEST_DEPTH: usize = 128;

/// Parse a source file into a [`Program`] (spec: `Program`).
///
/// The grammar expects imports first, then declarations; statements are only
/// legal inside blocks. Errors carry a byte span into `source`.
pub fn parse_program(source: &str) -> Result<Program, XuloError> {
    if let Some(offset) = depth::first_too_deep(source, MAX_NEST_DEPTH) {
        return Err(XuloError {
            kind: ErrorKind::Parse,
            message: "nesting is too deep".into(),
            span: Some(offset..offset),
            file: None,
        });
    }
    let pairs = XuloParser::parse(Rule::program, source).map_err(|e| pest_error(e, source))?;
    let root = pairs
        .into_iter()
        .next()
        .ok_or_else(|| XuloError::new(ErrorKind::Parse, "empty parse"))?;
    build::program(root)
}

/// Convert a pest parse error into an [`XuloError`] with a byte span.
///
/// pest reports `(line, col)`; this walks the source once to translate that
/// back to a byte offset so downstream tooling can slice the offending text.
fn pest_error(e: PestError<Rule>, source: &str) -> XuloError {
    let (line, col) = match &e.line_col {
        LineColLocation::Pos((l, c)) => (*l, *c),
        LineColLocation::Span((l, c), _) => (*l, *c),
    };
    let offset = line_col_to_offset(source, line, col);
    let message = match &e.variant {
        ErrorVariant::ParsingError {
            positives,
            negatives,
        } => match (describe_rules(positives), describe_rules(negatives)) {
            (Some(expected), Some(unexpected)) => {
                format!("expected {expected}, found {unexpected}")
            }
            (Some(expected), None) => format!("expected {expected}"),
            (None, Some(unexpected)) => format!("unexpected {unexpected}"),
            (None, None) => "syntax error".into(),
        },
        ErrorVariant::CustomError { message } => message.clone(),
    };
    XuloError {
        kind: ErrorKind::Parse,
        message,
        span: Some(offset..offset),
        file: None,
    }
}

/// Render a list of expected/unexpected rules as a comma-separated sentence.
fn describe_rules(rules: &[Rule]) -> Option<String> {
    if rules.is_empty() {
        return None;
    }
    let names: Vec<String> = rules.iter().map(|r| describe_rule(*r)).collect();
    Some(names.join(", "))
}

/// Human-readable spelling of a grammar rule for diagnostics.
fn describe_rule(rule: Rule) -> String {
    let name = format!("{rule:?}");
    // Keyword tokens are named `kw_and` etc.; show the source spelling.
    match name.strip_prefix("kw_") {
        Some(word) => format!("`{word}`"),
        None => match name.strip_prefix("map_") {
            // The fixed `Map` spelling of a typed-map literal.
            Some("kw") => "`Map`".into(),
            _ => name,
        },
    }
}

/// Byte offset of the 1-based `(line, col)` position (columns count chars).
fn line_col_to_offset(source: &str, line: usize, col: usize) -> usize {
    // Find the start byte of the requested line.
    let mut line_start = 0usize;
    let mut current_line = 1usize;
    if line > 1 {
        for (idx, ch) in source.char_indices() {
            if ch == '\n' {
                current_line += 1;
                if current_line == line {
                    line_start = idx + ch.len_utf8();
                    break;
                }
            }
        }
    }
    // Advance col-1 characters (columns are 1-based and count characters).
    let mut offset = line_start;
    for ch in source[line_start..].chars().take(col.saturating_sub(1)) {
        offset += ch.len_utf8();
    }
    offset.min(source.len())
}

/// Byte span helper shared by the builder modules.
pub(crate) fn span_of(pair: &pest::iterators::Pair<Rule>) -> Range<usize> {
    let s = pair.as_span();
    s.start()..s.end()
}

/// Recoverable builder error carrying a source span.
pub(crate) fn err(message: impl Into<String>, span: Range<usize>) -> XuloError {
    XuloError {
        kind: ErrorKind::Parse,
        message: message.into(),
        span: Some(span),
        file: None,
    }
}
