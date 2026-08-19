//! Protocol-free server helpers that are worth testing in isolation.
//!
//! The rest of the server (`main.rs`) stays in the binary target; only the
//! pieces that benefit from direct unit tests live here as a library.

pub mod edit {
    //! Apply text changes to a document using the analyzer's UTF-16-aware
    //! byte index — the same index the LSP protocol speaks in.

    use xulo_ide::line_index::{LineIndex, Pos};

    /// One LSP-style text change: an optional UTF-16 `range` splice with the
    /// replacement `text`. A `None` range replaces the whole document.
    #[derive(Debug, Clone, PartialEq)]
    pub struct TextEdit {
        pub range: Option<(Pos, Pos)>,
        pub text: String,
    }

    /// Apply a sequence of changes to `document`, left-to-right (the editor
    /// delivers them in order).
    pub fn apply_changes(document: &mut String, changes: &[TextEdit]) {
        for change in changes {
            let Some((start_pos, end_pos)) = change.range else {
                *document = change.text.clone();
                continue;
            };
            let index = LineIndex::new(document);
            let start = index
                .position_to_byte(document, start_pos)
                .unwrap_or(document.len());
            let end = index
                .position_to_byte(document, end_pos)
                .unwrap_or(document.len());
            if start <= end && end <= document.len() {
                document.replace_range(start..end, &change.text);
            }
        }
    }
}

pub mod semantic {
    //! Encode the analyzer's protocol-neutral semantic tokens into LSP's
    //! `semanticTokens` data array (relative `(line, startChar, length, type,
    //! modifiers)` 5-tuples) against a legend the server and the editor agree
    //! on.

    use xulo_ide::analysis::Analysis;
    use xulo_ide::semantic_tokens::{SemanticToken, TokenType};

    /// The legend's token types, ordered to match [`TokenType`]'s
    /// discriminants (see [`token_type_index`]) so an advertised legend and
    /// the encoded indices always agree.
    pub const TOKEN_TYPES: [&str; 11] = [
        "variable",
        "parameter",
        "function",
        "method",
        "property",
        "constant",
        "type",
        "enum",
        "interface",
        "enumMember",
        "class",
    ];

    /// The legend's token modifiers (`declaration` is bit 0).
    pub const TOKEN_MODIFIERS: [&str; 1] = ["declaration"];

    /// The legend index of a token type (its slot in [`TOKEN_TYPES`]).
    pub fn token_type_index(token_type: TokenType) -> usize {
        match token_type {
            TokenType::Variable => 0,
            TokenType::Parameter => 1,
            TokenType::Function => 2,
            TokenType::Method => 3,
            TokenType::Property => 4,
            TokenType::Constant => 5,
            TokenType::Type => 6,
            TokenType::Enum => 7,
            TokenType::Interface => 8,
            TokenType::EnumMember => 9,
            TokenType::Class => 10,
        }
    }

    /// Encode `tokens` for the document behind `analysis` into LSP's
    /// `semanticTokens` data array. `tokens` must be sorted by position and
    /// non-overlapping (as `Analysis::semantic_tokens` returns them).
    pub fn encode(analysis: &Analysis, tokens: &[SemanticToken]) -> Vec<u32> {
        let mut data = Vec::with_capacity(tokens.len() * 5);
        let mut prev_line = 0u32;
        let mut prev_start = 0u32;
        for token in tokens {
            let Some(range) = analysis
                .line_index()
                .span_to_range(&analysis.source, &token.span)
            else {
                continue;
            };
            let line = range.start.line;
            let start = range.start.character;
            let delta_line = line.saturating_sub(prev_line);
            let delta_start = if delta_line == 0 {
                start.saturating_sub(prev_start)
            } else {
                start
            };
            data.push(delta_line);
            data.push(delta_start);
            data.push(range.end.character.saturating_sub(start));
            data.push(token_type_index(token.token_type) as u32);
            data.push(u32::from(token.declaration));
            prev_line = line;
            prev_start = start;
        }
        data
    }
}
