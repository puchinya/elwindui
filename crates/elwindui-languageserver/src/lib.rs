//! Incremental parse/diagnostics/`vm.field` completion for `.rs` files using elwindui's
//! Rust-macro frontend (`#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]`).
//! See docs/design/tools/languageserver_design.md.
//!
//! Phase 7 (`docs/status/implementation_status.md`) retargeted this crate from the retired
//! directory-based model to a single-`.rs`-file model, matching elwindui's own unification onto
//! the Rust-macro frontend — real-time diagnostics (`diagnostics`, reusing
//! `elwindui_codegen::{component_frontend, validate}`) and `vm.field` member completion
//! (`completion`, built on `elwindui_codegen::codegen::SymbolTable::resolve`). Semantic tokens
//! (`semantic_tokens`) were dropped at that same retarget (a `.rs` file already gets real Rust
//! syntax highlighting from rust-analyzer everywhere else) and later reinstated scoped to just
//! `view! { .. }` macro bodies (Issue #14's "未解決の論点" — the one part of the file rust-analyzer
//! can't highlight, since `view!` is never a real macro) rather than the whole file, avoiding any
//! double-coloring/conflict with rust-analyzer's own semantic tokens. Generated-code preview and
//! hover (docs/design/tools/languageserver_design.md) and the offscreen-rendering pipeline
//! (docs/design/tools/preview_design.md) remain later phases, not attempted here.

pub mod completion;
pub mod diagnostics;
pub mod semantic_tokens;

use lsp_server::{
    Connection, Message, Notification as ServerNotification, Request as ServerRequest, RequestId,
    Response,
};
use lsp_types::notification::{
    DidChangeTextDocument, DidOpenTextDocument, DidSaveTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{Completion, Request as _, SemanticTokensFullRequest};
use lsp_types::{
    CompletionOptions, CompletionParams, CompletionResponse, DidChangeTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, PublishDiagnosticsParams,
    SemanticTokens, SemanticTokensFullOptions, SemanticTokensLegend, SemanticTokensOptions,
    SemanticTokensParams, SemanticTokensResult, SemanticTokensServerCapabilities,
    ServerCapabilities, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
};
use std::path::PathBuf;

pub fn run() {
    let (connection, io_threads) = Connection::stdio();

    let server_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".to_string()]),
            ..Default::default()
        }),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                legend: SemanticTokensLegend {
                    token_types: semantic_tokens::TOKEN_TYPES.to_vec(),
                    token_modifiers: Vec::new(),
                },
                full: Some(SemanticTokensFullOptions::Bool(true)),
                range: None,
                work_done_progress_options: Default::default(),
            },
        )),
        ..Default::default()
    })
    .expect("ServerCapabilities always serializes");

    match connection.initialize(server_capabilities) {
        Ok(_client_params) => {}
        Err(e) => {
            eprintln!("elwindui-languageserver: initialize handshake failed: {e}");
            return;
        }
    }

    main_loop(&connection);

    if let Err(e) = io_threads.join() {
        eprintln!("elwindui-languageserver: io threads did not shut down cleanly: {e}");
    }
}

fn main_loop(connection: &Connection) {
    for msg in &connection.receiver {
        match msg {
            Message::Request(req) => match connection.handle_shutdown(&req) {
                Ok(true) => return,
                Ok(false) => handle_request(connection, req),
                Err(e) => {
                    eprintln!("elwindui-languageserver: error during shutdown handling: {e}");
                    return;
                }
            },
            Message::Notification(not) => handle_notification(connection, not),
            Message::Response(_) => {}
        }
    }
}

fn handle_request(connection: &Connection, req: ServerRequest) {
    match req.method.as_str() {
        Completion::METHOD => handle_completion_request(connection, req),
        SemanticTokensFullRequest::METHOD => handle_semantic_tokens_request(connection, req),
        // Phase 7 handles no other requests (no hover/etc. yet).
        _ => {}
    }
}

fn handle_completion_request(connection: &Connection, req: ServerRequest) {
    let Ok(params) = serde_json::from_value::<CompletionParams>(req.params) else {
        return;
    };
    let position = params.text_document_position.position;
    let result = uri_to_path(&params.text_document_position.text_document.uri).and_then(|path| {
        let src = std::fs::read_to_string(&path).ok()?;
        Some(CompletionResponse::Array(completion::completions_at(
            &src, position,
        )))
    });
    send_response(connection, req.id, serde_json::to_value(result));
}

fn handle_semantic_tokens_request(connection: &Connection, req: ServerRequest) {
    let Ok(params) = serde_json::from_value::<SemanticTokensParams>(req.params) else {
        return;
    };
    let result = uri_to_path(&params.text_document.uri).and_then(|path| {
        let src = std::fs::read_to_string(&path).ok()?;
        Some(SemanticTokensResult::Tokens(SemanticTokens {
            result_id: None,
            data: semantic_tokens::semantic_tokens_for_file(&src),
        }))
    });
    send_response(connection, req.id, serde_json::to_value(result));
}

fn send_response(
    connection: &Connection,
    id: RequestId,
    result: serde_json::Result<serde_json::Value>,
) {
    let response = Response {
        id,
        result: Some(result.unwrap_or(serde_json::Value::Null)),
        error: None,
    };
    connection.sender.send(Message::Response(response)).ok();
}

fn handle_notification(connection: &Connection, not: ServerNotification) {
    let uri = match not.method.as_str() {
        DidOpenTextDocument::METHOD => not
            .extract::<DidOpenTextDocumentParams>(DidOpenTextDocument::METHOD)
            .ok()
            .map(|p| p.text_document.uri),
        DidChangeTextDocument::METHOD => not
            .extract::<DidChangeTextDocumentParams>(DidChangeTextDocument::METHOD)
            .ok()
            .map(|p| p.text_document.uri),
        DidSaveTextDocument::METHOD => not
            .extract::<DidSaveTextDocumentParams>(DidSaveTextDocument::METHOD)
            .ok()
            .map(|p| p.text_document.uri),
        _ => None,
    };
    if let Some(uri) = uri {
        publish_for_document(connection, &uri);
    }
}

/// Re-checks just the one file `uri` names (Phase 7 — the old directory-wide re-check is gone along
/// with the directory-based model it existed for, see `diagnostics.rs`'s own doc comment) and
/// publishes its diagnostics, including an empty list when it turned out clean (so previously
/// reported problems get cleared once fixed).
fn publish_for_document(connection: &Connection, uri: &Uri) {
    let Some(path) = uri_to_path(uri) else {
        return;
    };
    let Ok(src) = std::fs::read_to_string(&path) else {
        return;
    };

    let params = PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics: diagnostics::diagnostics_for_source(&src),
        version: None,
    };
    let notification = ServerNotification::new(PublishDiagnostics::METHOD.to_string(), params);
    connection
        .sender
        .send(Message::Notification(notification))
        .ok();
}

/// `lsp_types::Uri` (0.97+) is a thin `fluent_uri` wrapper with no `to_file_path`/`from_file_path`
/// of its own — round-tripping through `url::Url` (a well-tested, standard implementation of
/// exactly this conversion) is simpler and safer than hand-rolling percent-decoding against
/// `fluent_uri`'s lower-level API.
fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    url::Url::parse(uri.as_str()).ok()?.to_file_path().ok()
}
