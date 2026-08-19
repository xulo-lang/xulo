//! Integration tests for `xulo_analyzer::semantic`: LSP `semanticTokens` data
//! encoding — 5-tuple structure, valid legend indices, and position deltas.

use xulo_analyzer::semantic::{TOKEN_MODIFIERS, TOKEN_TYPES, encode, token_type_index};
use xulo_ide::semantic_tokens::TokenType;
use xulo_ide::{analyze_source, semantic_tokens::SemanticToken};

#[test]
fn legend_covers_every_token_type() {
    use TokenType::*;
    for (token_type, expected_index) in [
        (Variable, 0),
        (Parameter, 1),
        (Function, 2),
        (Method, 3),
        (Property, 4),
        (Constant, 5),
        (Type, 6),
        (Enum, 7),
        (Interface, 8),
        (EnumMember, 9),
        (Class, 10),
    ] {
        assert_eq!(token_type_index(token_type), expected_index);
        assert!(!TOKEN_TYPES[expected_index].is_empty());
    }
}

#[test]
fn encodes_relative_delta_tuples() {
    let src = "fn main() {\n  let a = 1\n  print(a)\n}\n";
    let analysis = analyze_source(src);
    let tokens = analysis.semantic_tokens();
    let data = encode(&analysis, &tokens);
    assert!(!data.is_empty());
    assert_eq!(data.len() % 5, 0, "data must be 5-tuples");

    let mut prev_line = 0u32;
    let mut prev_start = 0u32;
    for chunk in data.chunks(5) {
        let (delta_line, delta_start, length) = (chunk[0], chunk[1], chunk[2]);
        let token_type = chunk[3] as usize;
        let modifiers = chunk[4];
        assert!(
            token_type < TOKEN_TYPES.len(),
            "token type index {token_type} outside legend"
        );
        assert!(
            modifiers < (1u32 << TOKEN_MODIFIERS.len()),
            "modifier bitmask {modifiers} outside legend"
        );
        assert!(length > 0);

        let line = prev_line + delta_line;
        let start = if delta_line == 0 {
            prev_start + delta_start
        } else {
            delta_start
        };
        assert!(line >= prev_line, "tokens must be ordered by line");
        prev_line = line;
        prev_start = start;
    }
}

#[test]
fn first_token_is_function_declaration() {
    let src = "fn main() {\n  let a = 1\n  print(a)\n}\n";
    let analysis = analyze_source(src);
    let tokens = analysis.semantic_tokens();
    // The document's first token is the `main` function name: line 0, column 3,
    // length 4, `function`, declaration bit set.
    assert!(matches!(
        tokens.first(),
        Some(SemanticToken {
            span: _,
            token_type: TokenType::Function,
            declaration: true
        })
    ));
    let data = encode(&analysis, &tokens);
    assert_eq!(
        &data[..5],
        &[0, 3, 4, token_type_index(TokenType::Function) as u32, 1]
    );
}
