//! The top-level entry point: tokenize, parse, and semantically analyze a
//! source buffer, aggregating everything an editor needs into one [`Analysis`].
//!
//! The analysis never enters the compile pipeline. It re-uses the
//! out-of-band records `xulo-semantic`'s checker already collects
//! (`AnalysisResult.expr_types` / `.resolutions`) plus the parsed AST. Query
//! methods live in [`crate::diagnostics`], [`crate::object`], and
//! [`crate::queries`].

use xulo_core::ast::Program;
use xulo_core::error::XuloError;
use xulo_semantic::{AnalysisResult, analyze_partial};

use crate::line_index::{LineIndex, Pos};

/// Everything the analyzer knows about one document: the parsed AST, the
/// checker's out-of-band records, and the byte↔position index. `line_index`
/// converts between the AST's byte spans and LSP's UTF-16 coordinates.
#[derive(Debug, Clone)]
pub struct Analysis {
    pub source: String,
    pub program: Option<Program>,
    pub result: Option<AnalysisResult>,
    /// The first fatal error (lex, parse, or semantic). When present, `result`
    /// is `None`; `program` may still hold the parsed AST for outline queries.
    pub error: Option<XuloError>,
    pub line_index: LineIndex,
}

impl Analysis {
    /// The byte↔position index for this document.
    pub fn line_index(&self) -> &LineIndex {
        &self.line_index
    }

    /// The parsed AST, when lexing and parsing succeeded.
    pub fn program(&self) -> Option<&Program> {
        self.program.as_ref()
    }

    /// The checker's out-of-band records, when semantic analysis completed
    /// without error.
    pub fn result(&self) -> Option<&AnalysisResult> {
        self.result.as_ref()
    }

    /// Convert a byte offset to an LSP position (0-based line, UTF-16 column).
    pub fn byte_to_position(&self, offset: usize) -> Option<Pos> {
        self.line_index.byte_to_position(&self.source, offset)
    }

    /// Convert an LSP position back to a byte offset in `source`.
    pub fn position_to_byte(&self, pos: Pos) -> Option<usize> {
        self.line_index.position_to_byte(&self.source, pos)
    }
}

/// Tokenize, parse, and analyze a `source` buffer. Errors are reported
/// out-of-band via [`Analysis::diagnostics`]; single-file analysis seeds no
/// imports, so names brought in by `import` statements are treated opaquely
/// (cross-module seeding is the workspace layer's job).
pub fn analyze_source(source: &str) -> Analysis {
    let line_index = LineIndex::new(source);
    let tokens = match xulo_lexer::tokenize(source) {
        Err(err) => {
            return Analysis {
                source: source.to_string(),
                program: None,
                result: None,
                error: Some(err),
                line_index,
            };
        }
        Ok(tokens) => tokens,
    };
    let program = match xulo_parser::parse_program(&tokens) {
        Err(err) => {
            return Analysis {
                source: source.to_string(),
                program: None,
                result: None,
                error: Some(err),
                line_index,
            };
        }
        Ok(program) => program,
    };
    // Partial analysis: a single failing statement keeps the resolutions/types
    // gathered around it, so hover / go-to-definition survive error files.
    let (result, error) = analyze_partial(&program, &[], &[], &[]);
    Analysis {
        source: source.to_string(),
        program: Some(program),
        result: Some(result),
        error,
        line_index,
    }
}
