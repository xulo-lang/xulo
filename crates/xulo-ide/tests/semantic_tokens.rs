//! Integration tests for `xulo-ide::semantic_tokens`: declaration sites and
//! name-uses carry the right token types, and the returned stream is sorted
//! and non-overlapping (so the LSP delta encoding stays valid).

use xulo_ide::semantic_tokens::TokenType;
use xulo_ide::{analyze_source, semantic_tokens::SemanticToken};

/// Byte offset of the `n`-th (0-based) occurrence of `needle` in `src`.
fn nth(src: &str, needle: &str, n: usize) -> usize {
    src.match_indices(needle).nth(n).expect("needle present").0
}

fn tokens(src: &str) -> Vec<SemanticToken> {
    analyze_source(src).semantic_tokens()
}

const SAMPLE: &str = "\
type Rectangle = object

enum Color {
    Red,
    Green
}

trait Area {
    fn area(self): number
}

impl Area for Rectangle {
    fn area(self): number {
        return 0
    }
}

fn panel(): View {
    @State let count: number = 0
    print(count)
}

fn main() {
    const LIMIT = 10
    print(LIMIT)
}
";

#[test]
fn type_aliases_enums_traits_and_impls_declare() {
    let tokens = tokens(SAMPLE);
    let start = nth(SAMPLE, "type Rectangle", 0) + 5;
    assert!(tokens.contains(&token(
        start,
        start + "Rectangle".len(),
        TokenType::Type,
        true
    )));
    let start = nth(SAMPLE, "enum Color", 0) + 5;
    assert!(tokens.contains(&token(start, start + 5, TokenType::Enum, true)));
    for name in ["Red", "Green"] {
        let start = nth(SAMPLE, name, 0);
        assert!(tokens.contains(&token(
            start,
            start + name.len(),
            TokenType::EnumMember,
            true
        )));
    }
    let start = nth(SAMPLE, "trait Area", 0) + 6;
    assert!(tokens.contains(&token(start, start + 4, TokenType::Interface, true)));
    let start = nth(SAMPLE, "fn area", 0) + 3;
    assert!(tokens.contains(&token(start, start + 4, TokenType::Method, true)));
    let start = nth(SAMPLE, "fn area", 1) + 3;
    assert!(tokens.contains(&token(start, start + 4, TokenType::Method, true)));
}

#[test]
fn functions_declare() {
    let tokens = tokens(SAMPLE);
    let start = nth(SAMPLE, "fn panel", 0) + 3;
    assert!(tokens.contains(&token(start, start + 5, TokenType::Function, true)));
    let start = nth(SAMPLE, "fn main", 0) + 3;
    assert!(tokens.contains(&token(start, start + 4, TokenType::Function, true)));
}

#[test]
fn state_is_property_const_is_constant() {
    let tokens = tokens(SAMPLE);
    let start = nth(SAMPLE, "@State let count", 0) + 11;
    assert!(tokens.contains(&token(start, start + 5, TokenType::Property, true)));
    let use_start = nth(SAMPLE, "count", 1);
    assert!(tokens.contains(&token(use_start, use_start + 5, TokenType::Property, false)));
    let start = nth(SAMPLE, "const LIMIT", 0) + 6;
    assert!(
        tokens.contains(&token(start, start + 5, TokenType::Constant, true)),
        "missing constant decl at {start}: {tokens:#?}"
    );
    let use_start = nth(SAMPLE, "LIMIT", 1);
    assert!(tokens.contains(&token(use_start, use_start + 5, TokenType::Variable, false)));
}

#[test]
fn let_binding_declares_and_uses_resolve() {
    let src = "\
fn main() {
    let alpha = 1
    print(alpha)
}
";
    let tokens = tokens(src);
    let start = nth(src, "let alpha", 0) + 4;
    assert!(tokens.contains(&token(start, start + 5, TokenType::Variable, true)));
    let use_start = nth(src, "alpha", 1);
    assert!(tokens.contains(&token(use_start, use_start + 5, TokenType::Variable, false)));
}

#[test]
fn tokens_are_sorted_and_non_overlapping() {
    let tokens = tokens(SAMPLE);
    assert!(!tokens.is_empty());
    let mut cursor = 0;
    for token in &tokens {
        assert!(
            token.span.start >= cursor,
            "overlapping or unsorted span: {token:?}"
        );
        cursor = token.span.end;
    }
}

fn token(start: usize, end: usize, token_type: TokenType, declaration: bool) -> SemanticToken {
    SemanticToken {
        span: start..end,
        token_type,
        declaration,
    }
}
