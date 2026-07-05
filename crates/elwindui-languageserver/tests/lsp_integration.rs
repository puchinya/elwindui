//! End-to-end proof that the real `elwindui-languageserver` binary speaks LSP correctly: spawn it
//! as a subprocess (a real client would talk to it exactly this way over stdio), drive it through
//! `initialize` -> `initialized` -> `textDocument/didOpen`, and confirm a
//! `textDocument/publishDiagnostics` notification for a broken `.elwind` file actually arrives.
//! Reuses `lsp_server::Message::read`/`write` (public API, same framing the server itself uses)
//! instead of hand-rolling `Content-Length` header parsing for the test client.

use lsp_server::{Message, Notification, Request, RequestId};
use lsp_types::notification::{DidOpenTextDocument, Initialized, Notification as _, PublishDiagnostics};
use lsp_types::request::{Initialize, Request as _};
use lsp_types::{
    DidOpenTextDocumentParams, InitializeParams, InitializedParams, PublishDiagnosticsParams,
    TextDocumentItem, Uri,
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
fn publishes_diagnostics_for_a_broken_elwind_file() {
    let dir = std::env::temp_dir()
        .join(format!("elwindui_lsp_integration_test_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let file_path = dir.join("broken.elwind");
    let broken_src = "viewmodel { #[observable] }";
    std::fs::write(&file_path, broken_src).expect("write broken.elwind");

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
    let init_req =
        Request::new(RequestId::from(1), Initialize::METHOD.to_string(), InitializeParams::default());
    Message::from(init_req).write(&mut stdin).expect("send initialize");

    let resp = messages.recv_timeout(TIMEOUT).expect("initialize response within timeout");
    match resp {
        Message::Response(r) => assert_eq!(r.id, RequestId::from(1), "unexpected response id"),
        other => panic!("expected an initialize response, got {other:?}"),
    }

    // 2. initialized
    let initialized = Notification::new(Initialized::METHOD.to_string(), InitializedParams {});
    Message::from(initialized).write(&mut stdin).expect("send initialized");

    // 3. didOpen the broken file
    let file_uri = url::Url::from_file_path(&file_path).expect("file:// url");
    let uri = Uri::from_str(file_uri.as_str()).expect("lsp_types::Uri");
    let did_open = Notification::new(
        DidOpenTextDocument::METHOD.to_string(),
        DidOpenTextDocumentParams {
            text_document: TextDocumentItem {
                uri,
                language_id: "elwind".to_string(),
                version: 0,
                text: broken_src.to_string(),
            },
        },
    );
    Message::from(did_open).write(&mut stdin).expect("send didOpen");

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
    let shutdown_req =
        Request::new(RequestId::from(2), "shutdown".to_string(), serde_json::Value::Null);
    Message::from(shutdown_req).write(&mut stdin).ok();
    let _ = messages.recv_timeout(TIMEOUT);
    let exit = Notification::new("exit".to_string(), serde_json::Value::Null);
    Message::from(exit).write(&mut stdin).ok();
    let _ = child.try_wait();
    child.kill().ok();
    std::fs::remove_dir_all(&dir).ok();

    assert!(found, "expected a non-empty textDocument/publishDiagnostics for the broken .elwind file");
}
