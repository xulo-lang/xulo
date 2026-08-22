//! Semantic tokenization: protocol-neutral highlight tokens (identifier byte
//! spans with a token type and a `declaration` flag), driven by the checker's
//! resolution records and the AST's declaration sites. The LSP server maps
//! these onto its `textDocument/semanticTokens` legend and delta encoding.

use std::collections::HashMap;
use std::ops::Range as ByteRange;

use xulo_core::ast::{ExportItem, Statement};
use xulo_semantic::UseKind;

use crate::analysis::Analysis;

/// The semantic category of one token span. Names match VS Code's default
/// semantic-token types (so stock themes color them); the LSP layer maps the
/// enum to its legend index. The relative order only drives which of two
/// identical-span tokens wins (more specific kinds come later).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TokenType {
    Variable,
    Parameter,
    Function,
    Method,
    Property,
    Constant,
    Type,
    Enum,
    Interface,
    EnumMember,
    Class,
}

/// One highlightable span (bytes): what it is, and whether it *is* the
/// declaration (versus a use). Byte spans throughout; convert with the
/// document's `LineIndex`.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticToken {
    pub span: ByteRange<usize>,
    pub token_type: TokenType,
    pub declaration: bool,
}

impl Analysis {
    /// Semantic tokens for this document: identifier-level highlights merged
    /// from the checker's resolutions (every name-use plus `let` declaration
    /// sites) and the AST's declaration sites (top-level and nested `fn`s,
    /// `@State`/`@Environment` bindings, type aliases, enums and their
    /// variants, traits and their methods, `impl` methods, `for`/`catch`
    /// bindings). Types never overlap after flattening, so the LSP delta
    /// encoding stays valid.
    pub fn semantic_tokens(&self) -> Vec<SemanticToken> {
        let mut tokens: Vec<SemanticToken> = Vec::new();
        if let Some(result) = &self.result {
            for record in &result.resolutions {
                if let Some(token_type) = use_kind_token_type(record.kind) {
                    tokens.push(SemanticToken {
                        span: record.span.clone(),
                        token_type,
                        declaration: record.def.as_ref() == Some(&record.span),
                    });
                }
            }
        }
        if let Some(program) = &self.program {
            collect_decls(&program.statements, &mut tokens);
        }
        flatten(tokens)
    }
}

fn use_kind_token_type(kind: UseKind) -> Option<TokenType> {
    Some(match kind {
        UseKind::Variable => TokenType::Variable,
        UseKind::State => TokenType::Property,
        UseKind::Store => TokenType::Property,
        UseKind::Function => TokenType::Function,
        UseKind::Component => TokenType::Class,
    })
}

/// Push a declaration-site token (a name definition always counts as its own
/// declaration).
fn decl(tokens: &mut Vec<SemanticToken>, span: ByteRange<usize>, token_type: TokenType) {
    tokens.push(SemanticToken {
        span,
        token_type,
        declaration: true,
    });
}

/// Every naming declaration reachable through the statement tree, carrying its
/// kind and byte span. `@Store` bindings (a destructure pattern with no name
/// span) contribute nothing at the declaration site; their *uses* still color
/// via the checker's resolutions.
fn collect_decls(statements: &[Statement], tokens: &mut Vec<SemanticToken>) {
    for statement in statements {
        match statement {
            Statement::Fn(fn_def) => {
                decl(tokens, fn_def.name_span.clone(), TokenType::Function);
                collect_decls(&fn_def.body.statements, tokens);
            }
            Statement::Let(binding) => decl(
                tokens,
                binding.name_span.clone(),
                binding_kind(binding.is_const),
            ),
            Statement::State(state) => {
                decl(tokens, state.binding.name_span.clone(), TokenType::Property);
            }
            Statement::Environment(env) => decl(tokens, env.name_span.clone(), TokenType::Variable),
            Statement::TypeAlias(alias) => decl(tokens, alias.name_span.clone(), TokenType::Type),
            Statement::Enum(enum_def) => {
                decl(tokens, enum_def.name_span.clone(), TokenType::Enum);
                for variant in &enum_def.variants {
                    decl(tokens, variant.name_span.clone(), TokenType::EnumMember);
                }
            }
            Statement::Trait(trait_decl) => {
                decl(tokens, trait_decl.name_span.clone(), TokenType::Interface);
                for method in &trait_decl.methods {
                    decl(tokens, method.name_span.clone(), TokenType::Method);
                }
            }
            Statement::Impl(impl_decl) => {
                for method in &impl_decl.methods {
                    decl(tokens, method.name_span.clone(), TokenType::Method);
                }
            }
            Statement::Export(export) => match &export.item {
                ExportItem::Fn(fn_def) => {
                    decl(tokens, fn_def.name_span.clone(), TokenType::Function);
                }
                ExportItem::Let(binding) => decl(
                    tokens,
                    binding.name_span.clone(),
                    binding_kind(binding.is_const),
                ),
                ExportItem::Type(alias) => decl(tokens, alias.name_span.clone(), TokenType::Type),
                ExportItem::Enum(enum_def) => {
                    decl(tokens, enum_def.name_span.clone(), TokenType::Enum);
                    for variant in &enum_def.variants {
                        decl(tokens, variant.name_span.clone(), TokenType::EnumMember);
                    }
                }
                ExportItem::Trait(trait_decl) => {
                    decl(tokens, trait_decl.name_span.clone(), TokenType::Interface);
                    for method in &trait_decl.methods {
                        decl(tokens, method.name_span.clone(), TokenType::Method);
                    }
                }
                ExportItem::Names(_) => {}
            },
            // No name span on the store pattern; nothing to declare.
            Statement::Store(_) => {}
            Statement::For(for_stmt) => {
                decl(tokens, for_stmt.iter_var_span.clone(), TokenType::Variable);
                collect_decls(&for_stmt.body.statements, tokens);
            }
            Statement::While(while_stmt) => collect_decls(&while_stmt.body.statements, tokens),
            Statement::Try(try_stmt) => {
                decl(tokens, try_stmt.catch_var_span.clone(), TokenType::Variable);
                collect_decls(&try_stmt.try_block.statements, tokens);
                collect_decls(&try_stmt.catch_block.statements, tokens);
            }
            Statement::Block(block) => collect_decls(&block.statements, tokens),
            Statement::Return(_)
            | Statement::Assign(_)
            | Statement::Expr(_)
            | Statement::Throw(_)
            | Statement::Import(_)
            | Statement::Effect(_)
            | Statement::Component(_)
            | Statement::Break
            | Statement::Continue => {}
        }
    }
}

fn binding_kind(is_const: bool) -> TokenType {
    if is_const {
        TokenType::Constant
    } else {
        TokenType::Variable
    }
}

/// Merge overlapping spans into a sorted, non-overlapping stream — the shape
/// LSP's `semanticTokens` delta encoding requires:
///
/// * identical spans keep the more informative entry (a `const` declaration
///   wins over the resolution's plain `variable`, a declaration over a use);
/// * half-overlapping spans (rare: the checker records name-tight spans) drop
///   the later one, in iterator order.
fn flatten(tokens: Vec<SemanticToken>) -> Vec<SemanticToken> {
    let mut by_span: HashMap<ByteRange<usize>, SemanticToken> = HashMap::new();
    for token in tokens {
        let replace = match by_span.get(&token.span) {
            Some(existing) => better(existing, &token),
            None => true,
        };
        if replace {
            by_span.insert(token.span.clone(), token);
        }
    }
    let mut tokens: Vec<SemanticToken> = by_span.into_values().collect();
    tokens.sort_by_key(|token| (token.span.start, token.span.end));
    let mut out: Vec<SemanticToken> = Vec::with_capacity(tokens.len());
    for token in tokens {
        let last_end = out.last().map(|t| t.span.end).unwrap_or(0);
        if token.span.start < last_end {
            continue;
        }
        out.push(token);
    }
    out
}

/// Is `candidate` a stricter classification of the same span than `existing`?
fn better(existing: &SemanticToken, candidate: &SemanticToken) -> bool {
    if candidate.declaration != existing.declaration {
        return candidate.declaration;
    }
    candidate.token_type > existing.token_type
}
