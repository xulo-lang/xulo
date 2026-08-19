//! LSP-shaped diagnostics derived from `XuloError`s: the failed-check error
//! (lex/parse/semantic) plus any non-fatal warnings the checker collected.
//! The server layer maps these onto `lsp_types::Diagnostic`.

use xulo_core::error::ErrorKind;

use crate::analysis::Analysis;
use crate::line_index::Range;

/// Severity levels mirroring LSP `DiagnosticSeverity`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Information,
    Hint,
}

/// A diagnostic anchored to a source range, ready to map onto LSP.
#[derive(Debug, Clone, PartialEq)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: Severity,
    pub code: Option<String>,
    pub message: String,
}

impl Analysis {
    /// The document's diagnostics: the failed-check error (if any) plus every
    /// non-fatal warning the checker raised during a successful analysis.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        if let Some(err) = &self.error {
            out.push(self.diagnostic(err));
        }
        if let Some(result) = &self.result {
            out.extend(
                result
                    .warnings
                    .iter()
                    .map(|warning| self.diagnostic(warning)),
            );
        }
        out
    }

    fn diagnostic(&self, err: &xulo_core::error::XuloError) -> Diagnostic {
        let range = err
            .span
            .as_ref()
            .and_then(|span| self.line_index.span_to_range(&self.source, span))
            .unwrap_or_default();
        Diagnostic {
            range,
            severity: if err.kind == ErrorKind::Warning {
                Severity::Warning
            } else {
                Severity::Error
            },
            code: Some(code(err.kind).to_string()),
            message: err.message.clone(),
        }
    }
}

/// Stable diagnostic codes keyed by error kind (the same scheme the CLI's
/// `xulo_core::diagnostics` renderer uses).
fn code(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Lex => "E0001",
        ErrorKind::Parse => "E0002",
        ErrorKind::Semantic => "E0003",
        ErrorKind::Io => "E0004",
        ErrorKind::Codegen => "E0005",
        ErrorKind::Runtime => "E0006",
        ErrorKind::Warning => "W0001",
    }
}
