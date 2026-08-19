//! Position-driven queries over an [`Analysis`]: go-to-definition,
//! find-references, hover, the document outline, and per-symbol groups.
//!
//! Definitions and references are driven by the checker's [`UseRecord`]s:
//! each name-use carries its own span and the span of the declaration it
//! resolved to, so no separate symbol table is needed at this layer.

use xulo_semantic::{AnalysisResult, UseKind, UseRecord};

use crate::analysis::Analysis;
use crate::line_index::{Pos, Range};
use crate::object::{OutlineSymbol, SymbolInfo, collect_symbols, outline};

/// A source location in LSP coordinate space (URI-less; the server layer
/// attaches the document's URI).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Location {
    pub range: Range,
}

/// The answer to a definition request: the definition site(s) plus every
/// reference that resolved to them (the "find all references" view).
#[derive(Debug, Clone, PartialEq)]
pub struct DefinitionsResult {
    pub definitions: Vec<Location>,
    pub references: Vec<Location>,
}

/// The text shown on hover: what the name is, its inferred type, and where it
/// was declared.
#[derive(Debug, Clone, PartialEq)]
pub struct HoverInfo {
    pub name: String,
    pub kind: String,
    pub type_name: Option<String>,
    pub def: Option<Range>,
}

impl Analysis {
    /// The definition of the name referenced at `pos`, if statically known.
    pub fn go_to_definition(&self, pos: Pos) -> Option<Location> {
        let byte = self.position_to_byte(pos)?;
        let result = self.result.as_ref()?;
        if let Some(record) = resolve_record(result, byte)
            && let Some(def) = &record.def
        {
            return self
                .line_index
                .span_to_range(&self.source, def)
                .map(Location::from);
        }
        // The cursor may sit directly on a declaration: report the declaration
        // itself as its own definition.
        result
            .resolutions
            .iter()
            .filter_map(|record| record.def.as_ref())
            .find(|def| def.contains(&byte))
            .and_then(|def| self.line_index.span_to_range(&self.source, def))
            .map(Location::from)
    }

    /// Every reference to the symbol referenced at `pos`, including the
    /// declaration site itself.
    pub fn find_references(&self, pos: Pos) -> Vec<Location> {
        let Some(byte) = self.position_to_byte(pos) else {
            return Vec::new();
        };
        let Some(result) = &self.result else {
            return Vec::new();
        };
        let key = resolve_record(result, byte)
            .map(|record| (record.name.clone(), record.def.clone()))
            .or_else(|| {
                result
                    .resolutions
                    .iter()
                    .find(|record| record.def.as_ref().is_some_and(|def| def.contains(&byte)))
                    .map(|record| (record.name.clone(), record.def.clone()))
            });
        let Some((name, def)) = key else {
            return Vec::new();
        };
        let same_symbol = |record: &&UseRecord| match &def {
            Some(def) => record.def.as_ref() == Some(def),
            None => record.name == name,
        };
        let mut locations = Vec::new();
        if let Some(def) = &def
            && let Some(range) = self.line_index.span_to_range(&self.source, def)
        {
            locations.push(Location::from(range));
        }
        for record in result.resolutions.iter().filter(same_symbol) {
            if let Some(range) = self.line_index.span_to_range(&self.source, &record.span) {
                locations.push(Location::from(range));
            }
        }
        locations.sort_by_key(|loc| (loc.range.start.line, loc.range.start.character));
        locations.dedup_by(|a, b| a.range == b.range);
        locations
    }

    /// Both views of a name at `pos` in one query: definitions and references.
    pub fn definitions(&self, pos: Pos) -> DefinitionsResult {
        DefinitionsResult {
            definitions: self.go_to_definition(pos).into_iter().collect(),
            references: self.find_references(pos),
        }
    }

    /// Hover information for the name or literal under `pos`. Prefers a
    /// resolved name-use; falls back to the inferred type of the innermost
    /// expression covering the cursor (literals, member accesses, ...).
    pub fn hover_at(&self, pos: Pos) -> Option<HoverInfo> {
        let byte = self.position_to_byte(pos)?;
        let result = self.result.as_ref()?;
        if let Some(record) = resolve_record(result, byte) {
            return Some(HoverInfo {
                name: record.name.clone(),
                kind: kind_name(record.kind),
                type_name: record.type_.as_ref().map(|t| t.name()),
                def: record
                    .def
                    .as_ref()
                    .and_then(|def| self.line_index.span_to_range(&self.source, def)),
            });
        }
        let (_, ty) = result
            .expr_types
            .iter()
            .find(|(span, _)| span.contains(&byte))?;
        Some(HoverInfo {
            name: String::new(),
            kind: "expression".into(),
            type_name: Some(ty.name()),
            def: None,
        })
    }

    /// The top-level declarations of this document, with members nested.
    pub fn document_symbols(&self) -> Vec<OutlineSymbol> {
        match &self.program {
            Some(program) => outline(program, &self.line_index, &self.source),
            None => Vec::new(),
        }
    }

    /// Every symbol referenced in the document group by definition site.
    pub fn symbols(&self) -> Vec<SymbolInfo> {
        match &self.result {
            Some(result) => collect_symbols(result),
            None => Vec::new(),
        }
    }
}

/// The [`UseRecord`] whose span contains `byte`, preferring the most specific
/// (innermost) record.
fn resolve_record(result: &AnalysisResult, byte: usize) -> Option<&UseRecord> {
    result
        .resolutions
        .iter()
        .filter(|record| record.span.contains(&byte))
        .max_by_key(|record| record.span.len())
}

fn kind_name(kind: UseKind) -> String {
    match kind {
        UseKind::Variable => "variable".into(),
        UseKind::State => "state".into(),
        UseKind::Store => "store".into(),
        UseKind::Function => "function".into(),
    }
}

impl From<Range> for Location {
    fn from(range: Range) -> Self {
        Location { range }
    }
}
