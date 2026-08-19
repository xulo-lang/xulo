//! Document outline symbols and per-symbol reference groups, extracted from a
//! parsed program and the checker's resolution records.

use std::ops::Range as ByteRange;

use xulo_core::ast::{ExportItem, Program, Statement};
use xulo_semantic::AnalysisResult;

use crate::line_index::{LineIndex, Range};

/// The category of a declaration for the document outline. The VS Code layer
/// maps these to LSP `SymbolKind` values later; the values stay parser-agnostic
/// here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutlineKind {
    Function,
    Method,
    Variable,
    Constant,
    State,
    Store,
    Trait,
    TypeAlias,
    Enum,
    EnumMember,
    Impl,
}

impl OutlineKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Function => "function",
            Self::Method => "method",
            Self::Variable => "variable",
            Self::Constant => "constant",
            Self::State => "state",
            Self::Store => "store",
            Self::Trait => "trait",
            Self::TypeAlias => "type_alias",
            Self::Enum => "enum",
            Self::EnumMember => "enum_member",
            Self::Impl => "impl",
        }
    }
}

/// A declaration in the document outline: its name, kind, full `range`, and the
/// narrower selection `range` of the name itself (the anchor LSP jumps to).
#[derive(Debug, Clone, PartialEq)]
pub struct OutlineSymbol {
    pub name: String,
    pub kind: OutlineKind,
    pub range: Range,
    pub selection_range: Range,
    pub children: Vec<OutlineSymbol>,
}

/// A symbol's definition site and every name-use that resolved to it
/// (references). Byte spans throughout; convert with the document's index.
#[derive(Debug, Clone, PartialEq)]
pub struct SymbolInfo {
    pub name: String,
    pub def: Option<ByteRange<usize>>,
    pub uses: Vec<ByteRange<usize>>,
}

/// Build the document outline: top-level declarations, with enum variants,
/// trait methods, and `impl` methods nested under their parent.
pub fn outline(program: &Program, index: &LineIndex, source: &str) -> Vec<OutlineSymbol> {
    program
        .statements
        .iter()
        .filter_map(|statement| outline_statement(statement, index, source))
        .collect()
}

fn outline_statement(
    statement: &Statement,
    index: &LineIndex,
    source: &str,
) -> Option<OutlineSymbol> {
    match statement {
        Statement::Fn(f) => Some(sym(
            f.name.clone(),
            OutlineKind::Function,
            &f.name_span,
            Some(&f.span),
            Vec::new(),
            index,
            source,
        )),
        Statement::Export(export) => match &export.item {
            ExportItem::Fn(f) => Some(sym(
                f.name.clone(),
                OutlineKind::Function,
                &f.name_span,
                Some(&f.span),
                Vec::new(),
                index,
                source,
            )),
            ExportItem::Let(b) => Some(sym(
                b.name.clone(),
                binding_kind(b.is_const),
                &b.name_span,
                None,
                Vec::new(),
                index,
                source,
            )),
            ExportItem::Type(alias) => Some(sym(
                alias.name.clone(),
                OutlineKind::TypeAlias,
                &alias.name_span,
                None,
                Vec::new(),
                index,
                source,
            )),
            ExportItem::Enum(e) => Some(enum_symbol(e, index, source)),
            ExportItem::Trait(t) => Some(trait_symbol(t, index, source)),
            ExportItem::Names(_) => None,
        },
        Statement::Let(b) => Some(sym(
            b.name.clone(),
            binding_kind(b.is_const),
            &b.name_span,
            None,
            Vec::new(),
            index,
            source,
        )),
        Statement::State(s) => Some(sym(
            s.binding.name.clone(),
            OutlineKind::State,
            &s.binding.name_span,
            None,
            Vec::new(),
            index,
            source,
        )),
        Statement::Store(_) => {
            // `@Store` bindings have no recorded name span yet, so they are not
            // outlined (a plain name and `{ a, b: c }` destructure alike).
            None
        }
        Statement::Environment(e) => Some(sym(
            e.name.clone(),
            OutlineKind::Variable,
            &e.name_span,
            None,
            Vec::new(),
            index,
            source,
        )),
        Statement::TypeAlias(alias) => Some(sym(
            alias.name.clone(),
            OutlineKind::TypeAlias,
            &alias.name_span,
            None,
            Vec::new(),
            index,
            source,
        )),
        Statement::Enum(e) => Some(enum_symbol(e, index, source)),
        Statement::Trait(t) => Some(trait_symbol(t, index, source)),
        Statement::Impl(imp) => Some(impl_symbol(imp, index, source)),
        Statement::Return(_)
        | Statement::For(_)
        | Statement::While(_)
        | Statement::Assign(_)
        | Statement::Expr(_)
        | Statement::Block(_)
        | Statement::Try(_)
        | Statement::Throw(_)
        | Statement::Import(_)
        | Statement::Effect(_)
        | Statement::Component(_) => None,
    }
}

fn enum_symbol(e: &xulo_core::ast::EnumDef, index: &LineIndex, source: &str) -> OutlineSymbol {
    let children = e
        .variants
        .iter()
        .map(|variant| {
            sym(
                variant.name.clone(),
                OutlineKind::EnumMember,
                &variant.name_span,
                None,
                Vec::new(),
                index,
                source,
            )
        })
        .collect();
    sym(
        e.name.clone(),
        OutlineKind::Enum,
        &e.name_span,
        None,
        children,
        index,
        source,
    )
}

fn trait_symbol(t: &xulo_core::ast::TraitDecl, index: &LineIndex, source: &str) -> OutlineSymbol {
    let children = t
        .methods
        .iter()
        .map(|method| {
            sym(
                method.name.clone(),
                OutlineKind::Method,
                &method.name_span,
                Some(&method.span),
                Vec::new(),
                index,
                source,
            )
        })
        .collect();
    sym(
        t.name.clone(),
        OutlineKind::Trait,
        &t.name_span,
        Some(&t.span),
        children,
        index,
        source,
    )
}

fn impl_symbol(imp: &xulo_core::ast::ImplDecl, index: &LineIndex, source: &str) -> OutlineSymbol {
    let children = imp
        .methods
        .iter()
        .map(|method| {
            sym(
                method.name.clone(),
                OutlineKind::Method,
                &method.name_span,
                Some(&method.span),
                Vec::new(),
                index,
                source,
            )
        })
        .collect();
    // An `impl` block has no single name span; anchor its outline entry at the
    // block's own start.
    let range = index.span_to_range(source, &imp.span).unwrap_or_default();
    OutlineSymbol {
        name: format!("impl {} for {}", imp.trait_name, imp.type_name),
        kind: OutlineKind::Impl,
        range,
        selection_range: range,
        children,
    }
}

fn sym(
    name: String,
    kind: OutlineKind,
    name_span: &ByteRange<usize>,
    whole: Option<&ByteRange<usize>>,
    children: Vec<OutlineSymbol>,
    index: &LineIndex,
    source: &str,
) -> OutlineSymbol {
    let selection_range = index.span_to_range(source, name_span).unwrap_or_default();
    let range = whole
        .and_then(|span| index.span_to_range(source, span))
        .unwrap_or(selection_range);
    OutlineSymbol {
        name,
        kind,
        range,
        selection_range,
        children,
    }
}

fn binding_kind(is_const: bool) -> OutlineKind {
    if is_const {
        OutlineKind::Constant
    } else {
        OutlineKind::Variable
    }
}

/// Group every checker-resolved name-use by the declaration it resolved to.
/// Uses with no statically-known definition (imports, builtins) are grouped by
/// name alone. A name that resolves to different declarations (shadowing)
/// yields one group per declaration.
pub fn collect_symbols(result: &AnalysisResult) -> Vec<SymbolInfo> {
    let mut groups: std::collections::HashMap<(String, Option<ByteRange<usize>>), SymbolInfo> =
        std::collections::HashMap::new();
    for record in &result.resolutions {
        let entry = groups
            .entry((record.name.clone(), record.def.clone()))
            .or_insert_with(|| SymbolInfo {
                name: record.name.clone(),
                def: record.def.clone(),
                uses: Vec::new(),
            });
        entry.uses.push(record.span.clone());
    }
    let mut symbols: Vec<SymbolInfo> = groups.into_values().collect();
    symbols.sort_by(|a, b| {
        (a.def.as_ref().map(|span| span.start), &a.name)
            .cmp(&(b.def.as_ref().map(|span| span.start), &b.name))
    });
    symbols
}
