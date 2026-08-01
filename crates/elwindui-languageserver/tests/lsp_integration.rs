//! End-to-end proof that the real `elwindui-languageserver` binary speaks LSP correctly: spawn it
//! as a subprocess (a real client would talk to it exactly this way over stdio), drive it through
//! `initialize` -> `initialized` -> `textDocument/didOpen`, and confirm a
//! `textDocument/publishDiagnostics` notification for a broken `.rs` file actually arrives.
//! Reuses `lsp_server::Message::read`/`write` (public API, same framing the server itself uses)
//! instead of hand-rolling `Content-Length` header parsing for the test client.

use lsp_server::{Message, Notification, Request, RequestId};
use lsp_types::notification::{
    DidOpenTextDocument, Initialized, Notification as _, PublishDiagnostics,
};
use lsp_types::request::{Initialize, Request as _, SemanticTokensFullRequest};
use lsp_types::{
    DidOpenTextDocumentParams, InitializeParams, InitializedParams, PublishDiagnosticsParams,
    SemanticTokensParams, SemanticTokensResult, TextDocumentIdentifier, TextDocumentItem, Uri,
};
use std::io::BufReader;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::sync::mpsc;
use std::time::Duration;

/// `lsp_server::Message::read` blocks on the underlying pipe with no timeout of its own — if the
/// server ever sends fewer messages than expected, a plain read-loop-with-a-deadline-check hangs
/// forever *inside* the one blocking read that never returns (this happened during development:
/// both the test and the child process were left running indefinitely). Reading on a dedicated
/// thread and funneling every message through a channel lets the main thread enforce a real
/// timeout via `recv_timeout`, regardless of how long any individual read blocks.
fn spawn_reader(mut reader: impl std::io::BufRead + Send + 'static) -> mpsc::Receiver<Message> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        while let Ok(Some(msg)) = Message::read(&mut reader) {
            if tx.send(msg).is_err() {
                break;
            }
        }
    });
    rx
}

const TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn publishes_diagnostics_for_a_broken_rust_file() {
    let dir = std::env::temp_dir().join(format!(
        "elwindui_lsp_integration_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let file_path = dir.join("broken.rs");
    // Deliberately unparseable as Rust (unclosed struct body) -- exercises diagnostics.rs's
    // syn::parse_file error path, not validate::validate's.
    let broken_src = "struct Broken {\n    field:\n";
    std::fs::write(&file_path, broken_src).expect("write broken.rs");

    let mut child = Command::new(env!("CARGO_BIN_EXE_elwindui-languageserver"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn elwindui-languageserver binary");

    let mut stdin = child.stdin.take().expect("child stdin");
    let stdout = child.stdout.take().expect("child stdout");
    let messages = spawn_reader(BufReader::new(stdout));

    // 1. initialize
    let init_req = Request::new(
        RequestId::from(1),
        Initialize::METHOD.to_string(),
        InitializeParams::default(),
    );
    Message::from(init_req)
        .write(&mut stdin)
        .expect("send initialize");

    let resp = messages
        .recv_timeout(TIMEOUT)
        .expect("initialize response within timeout");
    match resp {
        Message::Response(r) => assert_eq!(r.id, RequestId::from(1), "unexpected response id"),
        other => panic!("expected an initialize response, got {other:?}"),
    }

    // 2. initialized
    let initialized = Notification::new(Initialized::METHOD.to_string(), InitializedParams {});
    Message::from(initialized)
        .write(&mut stdin)
        .expect("send initialized");

    // 3. didOpen the broken file
    let file_uri = url::Url::from_file_path(&file_path).expect("file:// url");
    let uri = Uri::from_str(file_uri.as_str()).expect("lsp_types::Uri");
    let did_open = Notification::new(
        DidOpenTextDocument::METHOD.to_string(),
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri,
                language_id: "rust".to_string(),
                version: 0,
                text: broken_src.to_string(),
            },
        },
    );
    Message::from(did_open)
        .write(&mut stdin)
        .expect("send didOpen");

    // 4. wait for a non-empty publishDiagnostics notification, skipping any other messages,
    // within an overall deadline enforced by `recv_timeout` on the channel (not by the blocking
    // read itself).
    let deadline = std::time::Instant::now() + TIMEOUT;
    let mut found = false;
    loop {
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            break;
        };
        match messages.recv_timeout(remaining) {
            Ok(Message::Notification(n)) if n.method == PublishDiagnostics::METHOD => {
                let params: PublishDiagnosticsParams =
                    serde_json::from_value(n.params).expect("valid PublishDiagnosticsParams");
                if !params.diagnostics.is_empty() {
                    found = true;
                    break;
                }
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }

    // Best-effort clean shutdown; the assertion below is what matters, and this must not hang the
    // test if the server doesn't respond as expected.
    let shutdown_req = Request::new(
        RequestId::from(2),
        "shutdown".to_string(),
        serde_json::Value::Null,
    );
    Message::from(shutdown_req).write(&mut stdin).ok();
    let _ = messages.recv_timeout(TIMEOUT);
    let exit = Notification::new("exit".to_string(), serde_json::Value::Null);
    Message::from(exit).write(&mut stdin).ok();
    let _ = child.try_wait();
    child.kill().ok();
    std::fs::remove_dir_all(&dir).ok();

    assert!(
        found,
        "expected a non-empty textDocument/publishDiagnostics for the broken .rs file"
    );
}

/// End-to-end proof of the `view!`-scoped semantic tokens feature (Issue #14's "未解決の論点",
/// resolved in favor of this scoping — see `semantic_tokens.rs`'s own doc comment): a real
/// `textDocument/semanticTokens/full` request against a real elwindui component file, over the
/// actual LSP wire protocol, returns non-empty token data — mirrors
/// `publishes_diagnostics_for_a_broken_rust_file`'s shape one level up (a request/response instead
/// of a notification).
#[test]
fn returns_semantic_tokens_scoped_to_the_view_macro_body() {
    let dir = std::env::temp_dir().join(format!(
        "elwindui_lsp_semantic_tokens_test_{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let file_path = dir.join("window.rs");
    let src = "struct Window {\n    body: view! { TextBlock { text: \"hi\" } },\n}\n";
    std::fs::write(&file_path, src).expect("write window.rs");

    let mut child = Command::new(env!("CARGO_BIN_EXE_elwindui-languageserver"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn elwindui-languageserver binary");

    let mut stdin = child.stdin.take().expect("child stdin");
    let stdout = child.stdout.take().expect("child stdout");
    let messages = spawn_reader(BufReader::new(stdout));

    // 1. initialize
    let init_req = Request::new(
        RequestId::from(1),
        Initialize::METHOD.to_string(),
        InitializeParams::default(),
    );
    Message::from(init_req)
        .write(&mut stdin)
        .expect("send initialize");
    let resp = messages
        .recv_timeout(TIMEOUT)
        .expect("initialize response within timeout");
    match resp {
        Message::Response(r) => assert_eq!(r.id, RequestId::from(1), "unexpected response id"),
        other => panic!("expected an initialize response, got {other:?}"),
    }

    // 2. initialized
    let initialized = Notification::new(Initialized::METHOD.to_string(), InitializedParams {});
    Message::from(initialized)
        .write(&mut stdin)
        .expect("send initialized");

    // 3. didOpen the real file
    let file_uri = url::Url::from_file_path(&file_path).expect("file:// url");
    let uri = Uri::from_str(file_uri.as_str()).expect("lsp_types::Uri");
    let did_open = Notification::new(
        DidOpenTextDocument::METHOD.to_string(),
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri: uri.clone(),
                language_id: "rust".to_string(),
                version: 0,
                text: src.to_string(),
            },
        },
    );
    Message::from(did_open)
        .write(&mut stdin)
        .expect("send didOpen");

    // 4. textDocument/semanticTokens/full
    let tokens_req = Request::new(
        RequestId::from(2),
        SemanticTokensFullRequest::METHOD.to_string(),
        SemanticTokensParams {
            text_document: TextDocumentIdentifier { uri },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        },
    );
    Message::from(tokens_req)
        .write(&mut stdin)
        .expect("send semanticTokens/full");

    let deadline = std::time::Instant::now() + TIMEOUT;
    let mut token_count = None;
    loop {
        let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
            break;
        };
        match messages.recv_timeout(remaining) {
            Ok(Message::Response(r)) if r.id == RequestId::from(2) => {
                let result: Option<SemanticTokensResult> = r
                    .result
                    .and_then(|v| serde_json::from_value(v).ok());
                token_count = match result {
                    Some(SemanticTokensResult::Tokens(t)) => Some(t.data.len()),
                    _ => Some(0),
                };
                break;
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }

    let shutdown_req = Request::new(
        RequestId::from(3),
        "shutdown".to_string(),
        serde_json::Value::Null,
    );
    Message::from(shutdown_req).write(&mut stdin).ok();
    let _ = messages.recv_timeout(TIMEOUT);
    let exit = Notification::new("exit".to_string(), serde_json::Value::Null);
    Message::from(exit).write(&mut stdin).ok();
    let _ = child.try_wait();
    child.kill().ok();
    std::fs::remove_dir_all(&dir).ok();

    // `TextBlock`/`text`/`"hi"` — `SemanticTokens::data` is `Vec<SemanticToken>`, already decoded
    // from LSP's five-`u32`-per-token wire encoding into one struct per token, so this is a token
    // count, not a `u32` count.
    assert_eq!(
        token_count,
        Some(3),
        "expected 3 tokens for the view! body"
    );
}
