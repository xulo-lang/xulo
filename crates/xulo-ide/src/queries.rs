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
    /// A rendered declaration preview (e.g. `fn Card(title: string): View` or
    /// the source line of a `let` binding), when the declaration is known.
    pub signature: Option<String>,
    /// The parameters of the declared function/component: `(name, type)`.
    pub params: Vec<(String, String)>,
    /// Doc/comment lines above the declaration, when any.
    pub comment: Option<String>,
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
            let mut info = HoverInfo {
                name: record.name.clone(),
                kind: kind_name(record.kind),
                type_name: record.type_.as_ref().map(|t| t.name()),
                def: record
                    .def
                    .as_ref()
                    .and_then(|def| self.line_index.span_to_range(&self.source, def)),
                signature: None,
                params: Vec::new(),
                comment: None,
            };
            self.enrich_from_def(&mut info, record.def.as_ref());
            return Some(info);
        }
        // The cursor may sit directly on a function declaration name (no
        // resolution record exists for a `fn`'s own name).
        if let Some(f) = self.fn_decl_at(byte) {
            return Some(self.fn_hover_info(f));
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
            signature: None,
            params: Vec::new(),
            comment: None,
        })
    }

    /// Fill in the declaration preview for a resolved name: a function/component
    /// shows its signature, parameters and doc comment; a variable shows the
    /// comment above its binding. Only the builtin `View` protocol gets a doc
    /// description; other components resolve from the `xulo ui` library (or the
    /// workspace) or have none.
    fn enrich_from_def(&self, info: &mut HoverInfo, def: Option<&std::ops::Range<usize>>) {
        if info.kind == "component"
            && let Some(doc) = builtin_component_doc(&info.name)
        {
            info.comment = Some(doc.to_string());
        }
        let Some(def) = def else {
            return;
        };
        if (info.kind == "function" || info.kind == "component")
            && let Some(f) = self.fn_by_span(def)
        {
            info.signature = Some(signature_of(f));
            info.params = f
                .params
                .iter()
                .map(|p| {
                    let ty = p
                        .type_annotation
                        .as_ref()
                        .map(|t| t.name())
                        .unwrap_or_else(|| "any".into());
                    (p.name.clone(), ty)
                })
                .collect();
        }
        info.comment = self
            .comment_before(def.start)
            .or_else(|| info.comment.take());
    }

    /// The hover preview of a function declaration (the cursor is on its name).
    fn fn_hover_info(&self, f: &xulo_core::ast::FnDef) -> HoverInfo {
        let range = self.line_index.span_to_range(&self.source, &f.name_span);
        HoverInfo {
            name: f.name.clone(),
            kind: "function".into(),
            type_name: f.return_type.as_ref().map(|t| t.name()),
            def: range,
            signature: Some(signature_of(f)),
            params: f
                .params
                .iter()
                .map(|p| {
                    let ty = p
                        .type_annotation
                        .as_ref()
                        .map(|t| t.name())
                        .unwrap_or_else(|| "any".into());
                    (p.name.clone(), ty)
                })
                .collect(),
            comment: self.comment_before(f.span.start),
        }
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

    /// The `FnDef` whose `name_span` is exactly `span` (a resolved function /
    /// component definition site), searching top-level and nested declarations.
    fn fn_by_span(&self, span: &std::ops::Range<usize>) -> Option<&xulo_core::ast::FnDef> {
        let program = self.program.as_ref()?;
        find_fn(&program.statements, span)
    }

    /// The function whose name covers `byte` (hovering directly on a
    /// declaration name).
    fn fn_decl_at(&self, byte: usize) -> Option<&xulo_core::ast::FnDef> {
        let program = self.program.as_ref()?;
        find_fn_covering(&program.statements, byte)
    }

    /// The consecutive `//` comment lines immediately above `byte` (doc
    /// comments), joined with newlines.
    fn comment_before(&self, byte: usize) -> Option<String> {
        let before = byte.min(self.source.len());
        let prefix = &self.source[..before];
        // Only scan *complete* lines: when `before` sits mid-line (e.g. a
        // resolved name_span), the trailing partial line is the declaration
        // itself and must not stop the scan.
        let complete = if prefix.ends_with('\n') {
            prefix
        } else {
            match prefix.rfind('\n') {
                Some(i) => &prefix[..i + 1],
                None => "",
            }
        };
        let lines: Vec<&str> = complete.lines().collect();
        let mut docs: Vec<String> = Vec::new();
        for line in lines.iter().rev() {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("//") {
                docs.push(rest.trim().to_string());
            } else {
                break;
            }
        }
        docs.reverse();
        if docs.is_empty() {
            None
        } else {
            Some(docs.join("\n"))
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
        UseKind::Component => "component".into(),
    }
}

/// Render a function/component's declaration signature:
/// `fn Card(title: string): View`.
fn signature_of(f: &xulo_core::ast::FnDef) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| {
            let ty = p
                .type_annotation
                .as_ref()
                .map(|t| t.name())
                .unwrap_or_else(|| "any".into());
            format!("{}: {}", p.name, ty)
        })
        .collect();
    let ret = f
        .return_type
        .as_ref()
        .map(|t| format!(": {}", t.name()))
        .unwrap_or_default();
    let kind = if f.is_async { "async fn" } else { "fn" };
    format!("{kind} {}({}){ret}", f.name, params.join(", "))
}

/// Find the `FnDef` whose `name_span` equals `span`, at any nesting depth.
fn find_fn<'a>(
    statements: &'a [xulo_core::ast::Statement],
    span: &std::ops::Range<usize>,
) -> Option<&'a xulo_core::ast::FnDef> {
    for statement in statements {
        match statement {
            xulo_core::ast::Statement::Fn(f) => {
                if &f.name_span == span {
                    return Some(f);
                }
                if let Some(found) = find_fn(&f.body.statements, span) {
                    return Some(found);
                }
            }
            xulo_core::ast::Statement::Expr(e) => {
                if let Some(found) = find_fn_expr(&e.expr, span) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_fn_expr<'a>(
    expr: &'a xulo_core::ast::Expression,
    span: &std::ops::Range<usize>,
) -> Option<&'a xulo_core::ast::FnDef> {
    match expr {
        // Anonymous closures have no `name_span`, but may hold named `fn`
        // declarations in their body.
        xulo_core::ast::Expression::FnExpr(f) => find_fn(&f.body.statements, span),
        xulo_core::ast::Expression::Call(call) => {
            for arg in &call.arguments {
                if let Some(found) = find_fn_expr(&arg.value, span) {
                    return Some(found);
                }
            }
            None
        }
        xulo_core::ast::Expression::If(if_expr) => {
            if let Some(found) = find_fn(&if_expr.then_branch.statements, span) {
                return Some(found);
            }
            if let Some(else_branch) = &if_expr.else_branch
                && let Some(found) = find_fn(&else_branch.statements, span)
            {
                return Some(found);
            }
            None
        }
        xulo_core::ast::Expression::Match(m) => {
            for arm in &m.arms {
                if let Some(found) = find_fn_expr(&arm.value, span) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

/// Find the `FnDef` whose name covers `byte`.
fn find_fn_covering(
    statements: &[xulo_core::ast::Statement],
    byte: usize,
) -> Option<&xulo_core::ast::FnDef> {
    for statement in statements {
        match statement {
            xulo_core::ast::Statement::Fn(f) => {
                if f.name_span.contains(&byte) {
                    return Some(f);
                }
                if let Some(found) = find_fn_covering(&f.body.statements, byte) {
                    return Some(found);
                }
            }
            xulo_core::ast::Statement::Expr(e) => {
                if let Some(found) = find_fn_expr_covering(&e.expr, byte) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

fn find_fn_expr_covering(
    expr: &xulo_core::ast::Expression,
    byte: usize,
) -> Option<&xulo_core::ast::FnDef> {
    match expr {
        xulo_core::ast::Expression::FnExpr(f) => find_fn_covering(&f.body.statements, byte),
        xulo_core::ast::Expression::Call(call) => {
            for arg in &call.arguments {
                if let Some(found) = find_fn_expr_covering(&arg.value, byte) {
                    return Some(found);
                }
            }
            None
        }
        xulo_core::ast::Expression::If(if_expr) => {
            if let Some(found) = find_fn_covering(&if_expr.then_branch.statements, byte) {
                return Some(found);
            }
            if let Some(else_branch) = &if_expr.else_branch
                && let Some(found) = find_fn_covering(&else_branch.statements, byte)
            {
                return Some(found);
            }
            None
        }
        xulo_core::ast::Expression::Match(m) => {
            for arm in &m.arms {
                if let Some(found) = find_fn_expr_covering(&arm.value, byte) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

/// The doc description of the one component protocol the xulo core defines
/// (`View`). Every other component (`Text`, `Button`, `VStack`, ...) belongs to
/// the `xulo ui` library — it has no builtin description here, and if it is not
/// imported into the workspace it simply has no preview to show.
fn builtin_component_doc(name: &str) -> Option<&'static str> {
    match name {
        "View" => Some("The `View` component protocol: the root layout container of a component."),
        _ => None,
    }
}

impl From<Range> for Location {
    fn from(range: Range) -> Self {
        Location { range }
    }
}
