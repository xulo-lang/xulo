//! The Xulo Language Server: a minimal stdio LSP server over the analyzer
//! crate. The analyzer stays protocol-agnostic (its own `Range`/`Diagnostic`/
//! `Location`); this binary maps them onto `lsp-types` and manages the open
//! documents.
//!
//! Feature set (MVP): publish diagnostics on open/change, go-to-definition
//! (including across module files), hover, find-references (within a file),
//! and the document outline. Documents are re-analyzed by rebuilding the
//! in-memory workspace on each change — no incremental compiler state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use lsp_server::{Connection, Message, Notification, Request, Response};
use lsp_types::{
    Diagnostic as LspDiagnostic, DiagnosticSeverity, DidChangeTextDocumentParams,
    DidCloseTextDocumentParams, DidOpenTextDocumentParams, DocumentFormattingParams,
    DocumentSymbol, DocumentSymbolParams, Hover, HoverContents, HoverProviderCapability,
    InitializeParams, InitializeResult, Location, MarkupContent, MarkupKind, NumberOrString, OneOf,
    Position, PublishDiagnosticsParams, Range as LspRange, ReferenceParams, SemanticTokenModifier,
    SemanticTokenType, SemanticTokens, SemanticTokensFullOptions, SemanticTokensLegend,
    SemanticTokensOptions, SemanticTokensParams, SemanticTokensServerCapabilities,
    ServerCapabilities, SymbolKind, TextDocumentContentChangeEvent, TextDocumentPositionParams,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextEdit as LspTextEdit, Url, WorkDoneProgressOptions,
};
use serde::Serialize;
use serde_json::Value;

use xulo_analyzer::edit::TextEdit;
use xulo_ide::diagnostics::Severity;
use xulo_ide::line_index::{LineIndex, Pos};
use xulo_ide::object::OutlineKind;
use xulo_ide::workspace::{Located, Workspace};

/// The server's bookkeeping: the texts of every open document.
#[derive(Default)]
struct State {
    documents: HashMap<PathBuf, String>,
}

impl State {
    /// Rebuild the in-memory workspace from the open documents, rooting the
    /// module graph at `entry` (the file the query targets).
    fn workspace(&self, entry: &Path) -> Option<Workspace> {
        if self.documents.is_empty() {
            return None;
        }
        Workspace::open(&self.documents, entry).ok()
    }
}

fn main() {
    let (connection, io_threads) = Connection::stdio();
    let (id, params) = connection
        .initialize_start()
        .expect("client must send `initialize`");
    let _params: InitializeParams =
        serde_json::from_value(params).expect("valid `initialize` params");

    let capabilities = ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Options(
            TextDocumentSyncOptions {
                open_close: Some(true),
                change: Some(TextDocumentSyncKind::INCREMENTAL),
                will_save: None,
                will_save_wait_until: None,
                save: None,
            },
        )),
        definition_provider: Some(OneOf::Left(true)),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        references_provider: Some(OneOf::Left(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        document_formatting_provider: Some(OneOf::Left(true)),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                work_done_progress_options: WorkDoneProgressOptions {
                    work_done_progress: None,
                },
                legend: SemanticTokensLegend {
                    token_types: xulo_analyzer::semantic::TOKEN_TYPES
                        .iter()
                        .map(|s| SemanticTokenType::new(s))
                        .collect(),
                    token_modifiers: xulo_analyzer::semantic::TOKEN_MODIFIERS
                        .iter()
                        .map(|s| SemanticTokenModifier::new(s))
                        .collect(),
                },
                range: None,
                full: Some(SemanticTokensFullOptions::Bool(true)),
            },
        )),
        ..Default::default()
    };
    let result = InitializeResult {
        capabilities,
        server_info: None,
    };
    connection
        .initialize_finish(
            id,
            serde_json::to_value(&result).expect("serialize init result"),
        )
        .expect("send `initialize` response");

    let mut state = State::default();
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => {
                if req.method == "shutdown" {
                    let ok = Response::new_ok(req.id, Value::Null);
                    let _ = connection.sender.send(Message::Response(ok));
                    continue;
                }
                handle_request(&connection, &mut state, req);
            }
            Message::Notification(notification) => {
                if notification.method == "exit" {
                    break;
                }
                handle_notification(&connection, &mut state, notification);
            }
            Message::Response(_) => {}
        }
    }
    // Drop the connection (and with it the message sender) before joining the
    // LSP io threads: the writer thread only exits when its channel closes,
    // which dropping the sender is what triggers.
    drop(connection);
    io_threads.join().expect("join io threads");
}

fn handle_request(connection: &Connection, state: &mut State, req: Request) {
    let Request { id, method, params } = req;
    let result = match method.as_str() {
        "textDocument/definition" => parse(params)
            .map(|p| definition(state, &p))
            .map(value_or_null),
        "textDocument/hover" => parse(params).map(|p| hover(state, &p)).map(value_or_null),
        "textDocument/references" => parse(params)
            .map(|p| references(state, &p))
            .map(value_or_null),
        "textDocument/documentSymbol" => parse(params)
            .map(|p| document_symbols(state, &p))
            .map(value_or_null),
        "textDocument/semanticTokens/full" => parse(params)
            .map(|p| semantic_tokens(state, &p))
            .map(value_or_null),
        "textDocument/formatting" => parse(params)
            .map(|p| formatting(state, &p))
            .map(value_or_null),
        other => Err(HandlerError::MethodNotFound(other.to_string())),
    };
    let response = match result {
        Ok(value) => Response::new_ok(id, value),
        Err(HandlerError::MethodNotFound(method)) => Response::new_err(
            id,
            lsp_server::ErrorCode::MethodNotFound as i32,
            format!("unhandled method `{method}`"),
        ),
        Err(HandlerError::InvalidParams(message)) => {
            Response::new_err(id, lsp_server::ErrorCode::InvalidParams as i32, message)
        }
    };
    let _ = connection.sender.send(Message::Response(response));
}

/// Deserialize request params or surface an `InvalidParams` error.
fn parse<T: serde::de::DeserializeOwned>(params: Value) -> Result<T, HandlerError> {
    serde_json::from_value(params).map_err(|e| HandlerError::InvalidParams(e.to_string()))
}

enum HandlerError {
    InvalidParams(String),
    MethodNotFound(String),
}

fn handle_notification(connection: &Connection, state: &mut State, notification: Notification) {
    match notification.method.as_str() {
        "textDocument/didOpen" => {
            let params: DidOpenTextDocumentParams =
                match serde_json::from_value(notification.params) {
                    Ok(params) => params,
                    Err(_) => return,
                };
            let path = uri_to_path(&params.text_document.uri);
            if let Some(path) = path {
                state.documents.insert(path, params.text_document.text);
            }
        }
        "textDocument/didChange" => {
            let params: DidChangeTextDocumentParams =
                match serde_json::from_value(notification.params) {
                    Ok(params) => params,
                    Err(_) => return,
                };
            if let Some(path) = uri_to_path(&params.text_document.uri)
                && let Some(document) = state.documents.get_mut(&path)
            {
                apply_changes(document, &params.content_changes);
            }
        }
        "textDocument/didClose" => {
            let params: DidCloseTextDocumentParams =
                match serde_json::from_value(notification.params) {
                    Ok(params) => params,
                    Err(_) => return,
                };
            if let Some(path) = uri_to_path(&params.text_document.uri) {
                state.documents.remove(&path);
            }
        }
        _ => {}
    }
    // Republish diagnostics for every open document after any change.
    if !state.documents.is_empty() {
        publish_all(connection, state);
    }
}

/// Apply incremental text changes (each an optional range splice). Column
/// offsets are UTF-16 LSP coordinates, converted via the analyzer's index.
fn apply_changes(document: &mut String, changes: &[TextDocumentContentChangeEvent]) {
    let edits: Vec<TextEdit> = changes
        .iter()
        .map(|change| TextEdit {
            range: change
                .range
                .map(|r| (to_our_pos(r.start), to_our_pos(r.end))),
            text: change.text.clone(),
        })
        .collect();
    xulo_analyzer::edit::apply_changes(document, &edits);
}

fn publish_all(connection: &Connection, state: &State) {
    for path in state.documents.keys() {
        let workspace = state.workspace(path);
        let analysis = workspace.as_ref().and_then(|ws| ws.analysis(path));
        let diagnostics = match analysis {
            Some(analysis) => analysis.diagnostics(),
            None => Vec::new(),
        };
        let lsp_diagnostics: Vec<LspDiagnostic> =
            diagnostics.iter().map(to_lsp_diagnostic).collect();
        let Some(uri) = path_to_uri(path) else {
            continue;
        };
        let params = PublishDiagnosticsParams {
            uri,
            diagnostics: lsp_diagnostics,
            version: None,
        };
        let notification = Notification::new(
            "textDocument/publishDiagnostics".to_string(),
            serde_json::to_value(&params).expect("serialize diagnostics"),
        );
        let _ = connection.sender.send(Message::Notification(notification));
    }
}

fn definition(state: &State, params: &TextDocumentPositionParams) -> Option<Value> {
    let path = uri_to_path(&params.text_document.uri)?;
    let workspace = state.workspace(&path)?;
    let located = workspace.go_to_definition(&path, to_our_pos(params.position))?;
    to_value(to_lsp_location(&located))
}

fn to_lsp_location(located: &Located) -> Location {
    Location {
        uri: path_to_uri(&located.file).expect("document uri"),
        range: to_lsp_range(located.range),
    }
}

fn hover(state: &State, params: &TextDocumentPositionParams) -> Option<Value> {
    let path = uri_to_path(&params.text_document.uri)?;
    let workspace = state.workspace(&path)?;
    let analysis = workspace.analysis(&path)?;
    let info = analysis.hover_at(to_our_pos(params.position))?;
    let contents = render_hover(&info);
    let range = info.def.map(to_lsp_range);
    to_value(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: contents,
        }),
        range,
    })
}

fn render_hover(info: &xulo_ide::queries::HoverInfo) -> String {
    if info.kind == "expression" {
        return match &info.type_name {
            Some(ty) => format!("```\n{ty}\n```"),
            None => String::new(),
        };
    }
    let mut text = format!("**{kind}** `{name}`", kind = info.kind, name = info.name);
    if let Some(ty) = &info.type_name {
        text.push_str(&format!("\n\n```xulo\n{ty}\n```"));
    }
    text
}

fn references(state: &State, params: &ReferenceParams) -> Option<Value> {
    let path = uri_to_path(&params.text_document_position.text_document.uri)?;
    let workspace = state.workspace(&path)?;
    let analysis = workspace.analysis(&path)?;
    let uri = path_to_uri(&path)?;
    let locations: Vec<Location> = analysis
        .find_references(to_our_pos(params.text_document_position.position))
        .into_iter()
        .map(|loc| Location {
            uri: uri.clone(),
            range: to_lsp_range(loc.range),
        })
        .collect();
    to_value(locations)
}

fn document_symbols(state: &State, params: &DocumentSymbolParams) -> Option<Value> {
    let path = uri_to_path(&params.text_document.uri)?;
    let workspace = state.workspace(&path)?;
    let analysis = workspace.analysis(&path)?;
    let symbols: Vec<DocumentSymbol> = analysis
        .document_symbols()
        .into_iter()
        .map(to_document_symbol)
        .collect();
    to_value(symbols)
}

fn semantic_tokens(state: &State, params: &SemanticTokensParams) -> Option<Value> {
    let path = uri_to_path(&params.text_document.uri)?;
    let workspace = state.workspace(&path)?;
    let analysis = workspace.analysis(&path)?;
    let data: Vec<lsp_types::SemanticToken> =
        xulo_analyzer::semantic::encode(analysis, &analysis.semantic_tokens())
            .chunks_exact(5)
            .map(|chunk| lsp_types::SemanticToken {
                delta_line: chunk[0],
                delta_start: chunk[1],
                length: chunk[2],
                token_type: chunk[3],
                token_modifiers_bitset: chunk[4],
            })
            .collect();
    to_value(SemanticTokens {
        result_id: None,
        data,
    })
}

fn formatting(state: &State, params: &DocumentFormattingParams) -> Option<Value> {
    let path = uri_to_path(&params.text_document.uri)?;
    let source = state.documents.get(&path)?;
    let formatted = xulo_ide::format::format(source).ok()?;
    if formatted == *source {
        return to_value(Vec::<LspTextEdit>::new());
    }
    // A single full-document edit: from the start to the end of the source.
    let index = LineIndex::new(source);
    let end = index.byte_to_position(source, source.len())?;
    let edit = LspTextEdit {
        range: LspRange {
            start: Position {
                line: 0,
                character: 0,
            },
            end: to_lsp_position(end),
        },
        new_text: formatted,
    };
    to_value(vec![edit])
}

#[allow(deprecated)]
fn to_document_symbol(symbol: xulo_ide::object::OutlineSymbol) -> DocumentSymbol {
    DocumentSymbol {
        name: symbol.name,
        detail: None,
        kind: to_symbol_kind(symbol.kind),
        tags: None,
        deprecated: None,
        range: to_lsp_range(symbol.range),
        selection_range: to_lsp_range(symbol.selection_range),
        children: Some(
            symbol
                .children
                .into_iter()
                .map(to_document_symbol)
                .collect(),
        ),
    }
}

fn to_symbol_kind(kind: OutlineKind) -> SymbolKind {
    match kind {
        OutlineKind::Function => SymbolKind::FUNCTION,
        OutlineKind::Method => SymbolKind::METHOD,
        OutlineKind::Variable => SymbolKind::VARIABLE,
        OutlineKind::Constant => SymbolKind::CONSTANT,
        OutlineKind::State | OutlineKind::Store => SymbolKind::PROPERTY,
        OutlineKind::Trait => SymbolKind::INTERFACE,
        OutlineKind::TypeAlias => SymbolKind::STRUCT,
        OutlineKind::Enum => SymbolKind::ENUM,
        OutlineKind::EnumMember => SymbolKind::ENUM_MEMBER,
        OutlineKind::Impl => SymbolKind::CLASS,
    }
}

fn to_lsp_diagnostic(diagnostic: &xulo_ide::diagnostics::Diagnostic) -> LspDiagnostic {
    LspDiagnostic {
        range: to_lsp_range(diagnostic.range),
        severity: Some(match diagnostic.severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
            Severity::Information => DiagnosticSeverity::INFORMATION,
            Severity::Hint => DiagnosticSeverity::HINT,
        }),
        code: diagnostic.code.clone().map(NumberOrString::String),
        code_description: None,
        source: Some("xulo".to_string()),
        message: diagnostic.message.clone(),
        related_information: None,
        tags: None,
        data: None,
    }
}

fn to_lsp_range(range: xulo_ide::line_index::Range) -> LspRange {
    LspRange {
        start: to_lsp_position(range.start),
        end: to_lsp_position(range.end),
    }
}

fn to_lsp_position(pos: Pos) -> Position {
    Position {
        line: pos.line,
        character: pos.character,
    }
}

fn to_our_pos(position: Position) -> Pos {
    Pos {
        line: position.line,
        character: position.character,
    }
}

fn uri_to_path(uri: &Url) -> Option<PathBuf> {
    uri.to_file_path().ok()
}

fn path_to_uri(path: &Path) -> Option<Url> {
    Url::from_file_path(path).ok()
}

fn to_value<T: Serialize>(value: T) -> Option<Value> {
    serde_json::to_value(value).ok()
}

fn value_or_null(value: Option<Value>) -> Value {
    value.unwrap_or(Value::Null)
}
