//! Incremental parse/diagnostics/hover and preview-instance generation for `.elwind` files.
//! See docs/elwindui_tool_languageserver_design.md.
//!
//! Phase 1 (this crate, currently): real-time diagnostics only (付録B.2 item 1), reusing
//! `elwindui_codegen::{parser, validate}` as-is via the `diagnostics` module. Generated-code
//! preview, hover (付録B.2 items 2/3), and the offscreen-rendering pipeline (付録B.3) are later
//! phases, not attempted here.

pub mod diagnostics;
pub mod semantic_tokens;

use lsp_server::{Connection, Message, Notification as ServerNotification, Request as ServerRequest, Response};
use lsp_types::notification::{
    DidChangeTextDocument, DidOpenTextDocument, DidSaveTextDocument, Notification as _,
    PublishDiagnostics,
};
use lsp_types::request::{Request as _, SemanticTokensFullRequest};
use lsp_types::{
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, DidSaveTextDocumentParams,
    PublishDiagnosticsParams, SemanticTokens, SemanticTokensFullOptions, SemanticTokensLegend,
    SemanticTokensOptions, SemanticTokensParams, SemanticTokensResult,
    SemanticTokensServerCapabilities, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncKind, Uri,
};
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub fn run() {
    let (connection, io_threads) = Connection::stdio();

    let server_capabilities = serde_json::to_value(ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                legend: SemanticTokensLegend {
                    token_types: semantic_tokens::TOKEN_TYPES.to_vec(),
                    token_modifiers: vec![],
                },
                full: Some(SemanticTokensFullOptions::Bool(true)),
                ..Default::default()
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
    if req.method != SemanticTokensFullRequest::METHOD {
        // Phase 1 handles no other requests (no hover/completion/etc. yet).
        return;
    }
    let Ok(params) = serde_json::from_value::<SemanticTokensParams>(req.params) else {
        return;
    };
    let result = uri_to_path(&params.text_document.uri)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|src| {
            SemanticTokensResult::Tokens(SemanticTokens {
                result_id: None,
                data: semantic_tokens::semantic_tokens_for_source(&src),
            })
        });
    let response = Response {
        id: req.id,
        result: Some(serde_json::to_value(result).unwrap_or(serde_json::Value::Null)),
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

/// Re-checks the whole directory `uri` lives in (cross-file `bind!`/`vm.field` references need
/// every `.elwind` file in it visible at once — the same unit `compile_dir`/`diagnostics_for_dir`
/// process) and publishes each file's diagnostics, including empty lists for files that turned out
/// clean (so previously-reported problems get cleared once fixed).
fn publish_for_document(connection: &Connection, uri: &Uri) {
    let Some(path) = uri_to_path(uri) else {
        return;
    };
    let Some(dir) = path.parent() else {
        return;
    };

    for (file_path, diags) in diagnostics::diagnostics_for_dir(dir) {
        let Some(file_uri) = path_to_uri(&file_path) else {
            continue;
        };
        let params = PublishDiagnosticsParams { uri: file_uri, diagnostics: diags, version: None };
        let notification = ServerNotification::new(PublishDiagnostics::METHOD.to_string(), params);
        if connection.sender.send(Message::Notification(notification)).is_err() {
            return; // client disconnected
        }
    }
}

/// `lsp_types::Uri` (0.97+) is a thin `fluent_uri` wrapper with no `to_file_path`/`from_file_path`
/// of its own — round-tripping through `url::Url` (a well-tested, standard implementation of
/// exactly this conversion) is simpler and safer than hand-rolling percent-decoding against
/// `fluent_uri`'s lower-level API.
fn uri_to_path(uri: &Uri) -> Option<PathBuf> {
    url::Url::parse(uri.as_str()).ok()?.to_file_path().ok()
}

fn path_to_uri(path: &Path) -> Option<Uri> {
    let url = url::Url::from_file_path(path).ok()?;
    Uri::from_str(url.as_str()).ok()
}
