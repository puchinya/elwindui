//! Incremental parse/diagnostics/`vm.field` completion for `.rs` files using elwindui's
//! Rust-macro frontend (`#[elwindui::component]`/`#[elwindui::viewmodel]`/`#[elwindui::dsl_enum]`).
//! See docs/elwindui_tool_languageserver_design.md.
//!
//! Phase 7 (`docs/elwindui_implementation_status.md`) retargeted this crate from the retired
//! `.elwind`-directory model to a single-`.rs`-file model, matching elwindui's own unification onto
//! the Rust-macro frontend — real-time diagnostics (`diagnostics`, reusing
//! `elwindui_codegen::{component_frontend, validate}`) and `vm.field` member completion
//! (`completion`, built on `elwindui_codegen::codegen::SymbolTable::resolve`). There is no semantic-
//! tokens provider: a `.rs` file already gets real Rust syntax highlighting from rust-analyzer, and
//! retrofitting the old `.elwind`-specific tokenizer for a `view! { .. }` macro body's worth of
//! text wasn't judged worth the added complexity (see the removed `semantic_tokens.rs`, deleted
//! along with this retarget). Generated-code preview and hover (付録B.2 items 2/3) and the
//! offscreen-rendering pipeline (付録B.3) remain later phases, not attempted here.

pub mod completion;
pub mod diagnostics;

use lsp_server::{
    Connection, Message, Notification as ServerNotification, Request as ServerRequest, RequestId,
    Response,
};
use lsp_types::notification::{
    DidChangeTextDocument, DidOpenTextDocument, DidSaveTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{Completion, Request as _};
use lsp_types::{
    CompletionOptions, CompletionParams, CompletionResponse, DidChangeTextDocumentParams,
    DidOpenTextDocumentParams, DidSaveTextDocumentParams, PublishDiagnosticsParams,
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
/// with the `.elwind`-directory model it existed for, see `diagnostics.rs`'s own doc comment) and
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
