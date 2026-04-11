//! End-to-end integration tests for L2.3 `textDocument/rename` and
//! `textDocument/prepareRename`.
//!
//! These drive the full `initialize → didOpen → textDocument/rename*
//! → shutdown → exit` loop over an in-memory `lsp_server::Connection`,
//! the same harness L1.3/L1.4/L1.5/L2.1/L2.2 already use. The pure
//! renderer is unit-tested inside `src/rename.rs`; this file confirms
//! the dispatch wiring is correct and that the cross-file walk actually
//! picks up edits in files the client never opens.

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidOpenTextDocument, Exit, Initialized, Notification as _, PublishDiagnostics,
};
use lsp_types::request::{Initialize, PrepareRenameRequest, Rename, Request as _, Shutdown};
use lsp_types::{
    ClientCapabilities, DidOpenTextDocumentParams, InitializeParams, InitializedParams, Position,
    PrepareRenameResponse, PublishDiagnosticsParams, RenameParams, TextDocumentIdentifier,
    TextDocumentItem, TextDocumentPositionParams, Url, WorkDoneProgressParams, WorkspaceEdit,
    WorkspaceFolder,
};
use tempfile::TempDir;

const CAPTURE_FIXTURE: &str = r#"name: cap
tests:
  main:
    steps:
      - name: login
        request:
          method: POST
          url: "http://x/auth"
        capture:
          token: $.id
      - name: list
        request:
          method: GET
          url: "http://x/{{ capture.token }}"
      - name: detail
        request:
          method: GET
          url: "http://x/items?k={{ capture.token }}"
"#;

const ALPHA_FIXTURE: &str = r#"name: alpha
env:
  base_url: http://localhost:3000
tests:
  main:
    steps:
      - name: list
        request:
          method: GET
          url: "{{ env.base_url }}/items"
"#;

const BETA_FIXTURE: &str = r#"name: beta
tests:
  main:
    steps:
      - name: ping
        request:
          method: GET
          url: "{{ env.base_url }}/ping"
      - name: pong
        request:
          method: GET
          url: "{{ env.base_url }}/pong"
"#;

const BUILTIN_FIXTURE: &str = r#"name: builtin
steps:
  - name: gen
    request:
      method: GET
      url: "http://localhost/{{ $uuid }}"
"#;

#[test]
fn rename_capture_edits_declaration_and_every_use_site_in_current_file() {
    let dir = TempDir::new().unwrap();
    let path = write(dir.path(), "cap.tarn.yaml", CAPTURE_FIXTURE);

    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake_with_root(&client_conn, &dir);

    let uri = Url::from_file_path(&path).unwrap();
    send_did_open(&client_conn, &uri, CAPTURE_FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Cursor inside `capture.token` on line 13 of CAPTURE_FIXTURE
    // (0-based), well inside `token`.
    let edit = request_rename(&client_conn, &uri, Position::new(13, 30), "auth_token")
        .expect("rename returned no error");
    let changes = edit.changes.expect("changes present");
    assert_eq!(
        changes.len(),
        1,
        "capture rename touches only the current file"
    );
    let edits = changes.get(&uri).expect("edits for current uri");
    // 1 declaration + 2 use sites.
    assert_eq!(edits.len(), 3, "got {:?}", edits);
    for e in edits {
        assert_eq!(e.new_text, "auth_token");
    }

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn rename_env_key_edits_declaration_and_uses_across_two_files() {
    let dir = TempDir::new().unwrap();
    let alpha_path = write(dir.path(), "alpha.tarn.yaml", ALPHA_FIXTURE);
    let beta_path = write(dir.path(), "beta.tarn.yaml", BETA_FIXTURE);

    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake_with_root(&client_conn, &dir);

    let alpha_uri = Url::from_file_path(&alpha_path).unwrap();
    let beta_uri = Url::from_file_path(&beta_path).unwrap();
    send_did_open(&client_conn, &alpha_uri, ALPHA_FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &alpha_uri);

    // Cursor inside `env.base_url` on line 9 (0-based), well inside
    // `base_url`. ALPHA_FIXTURE's use site line is:
    //   `          url: "{{ env.base_url }}/items"`
    let edit = request_rename(&client_conn, &alpha_uri, Position::new(9, 20), "api_url")
        .expect("rename returned no error");
    let changes = edit.changes.expect("changes present");

    let alpha_edits = changes.get(&alpha_uri).expect("alpha has edits");
    // 1 declaration + 1 use site.
    assert_eq!(alpha_edits.len(), 2, "alpha edits: {:?}", alpha_edits);

    let beta_edits = changes.get(&beta_uri).expect("beta has edits");
    // 2 use sites.
    assert_eq!(beta_edits.len(), 2, "beta edits: {:?}", beta_edits);

    for e in alpha_edits.iter().chain(beta_edits.iter()) {
        assert_eq!(e.new_text, "api_url");
    }

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn rename_with_invalid_identifier_returns_invalid_params_error() {
    let dir = TempDir::new().unwrap();
    let path = write(dir.path(), "cap.tarn.yaml", CAPTURE_FIXTURE);

    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake_with_root(&client_conn, &dir);

    let uri = Url::from_file_path(&path).unwrap();
    send_did_open(&client_conn, &uri, CAPTURE_FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &uri);

    let response = request_rename_raw(&client_conn, &uri, Position::new(13, 30), "2bad");
    let err = response
        .error
        .expect("invalid identifier must produce an error");
    // InvalidParams = -32602
    assert_eq!(err.code, -32602);
    assert!(err.message.contains("2bad"), "message: {}", err.message);

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn prepare_rename_for_capture_token_returns_identifier_range() {
    let dir = TempDir::new().unwrap();
    let path = write(dir.path(), "cap.tarn.yaml", CAPTURE_FIXTURE);

    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake_with_root(&client_conn, &dir);

    let uri = Url::from_file_path(&path).unwrap();
    send_did_open(&client_conn, &uri, CAPTURE_FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &uri);

    let resp = request_prepare_rename(&client_conn, &uri, Position::new(13, 30))
        .expect("prepareRename returned some range");
    let PrepareRenameResponse::Range(range) = resp else {
        panic!("expected a plain Range response, got {resp:?}");
    };
    // The identifier lives on line index 13 (0-based), and the text
    // `token` appears after `capture.` in the interpolation.
    assert_eq!(range.start.line, 13);
    // `token` is 5 characters long.
    assert_eq!(range.end.character - range.start.character, 5);

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn prepare_rename_on_builtin_token_returns_null() {
    let dir = TempDir::new().unwrap();
    let path = write(dir.path(), "builtin.tarn.yaml", BUILTIN_FIXTURE);

    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake_with_root(&client_conn, &dir);

    let uri = Url::from_file_path(&path).unwrap();
    send_did_open(&client_conn, &uri, BUILTIN_FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Cursor inside `$uuid` on line 5 (0-based).
    let resp = request_prepare_rename(&client_conn, &uri, Position::new(5, 30));
    assert!(resp.is_none(), "builtin must decline prepareRename");

    shutdown_and_join(client_conn, server_thread);
}

// ---------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------

fn write(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).expect("write fixture");
    path
}

fn request_rename(
    client_conn: &Connection,
    uri: &Url,
    position: Position,
    new_name: &str,
) -> Result<WorkspaceEdit, String> {
    let response = request_rename_raw(client_conn, uri, position, new_name);
    if let Some(err) = response.error {
        return Err(format!("rpc error: {} (code {})", err.message, err.code));
    }
    let value = response
        .result
        .expect("response had neither result nor error");
    Ok(serde_json::from_value(value).expect("rename response shape"))
}

fn request_rename_raw(
    client_conn: &Connection,
    uri: &Url,
    position: Position,
    new_name: &str,
) -> Response {
    let req_id: RequestId = 9401.into();
    let params = RenameParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            position,
        },
        new_name: new_name.to_owned(),
        work_done_progress_params: WorkDoneProgressParams::default(),
    };
    client_conn
        .sender
        .send(Message::Request(Request {
            id: req_id.clone(),
            method: Rename::METHOD.to_owned(),
            params: serde_json::to_value(params).unwrap(),
        }))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for rename response");
        }
        match client_conn.receiver.recv_timeout(remaining) {
            Ok(Message::Response(resp)) if resp.id == req_id => return resp,
            Ok(_) => {}
            Err(e) => panic!("recv failed while waiting for rename response: {e}"),
        }
    }
}

fn request_prepare_rename(
    client_conn: &Connection,
    uri: &Url,
    position: Position,
) -> Option<PrepareRenameResponse> {
    let req_id: RequestId = 9402.into();
    let params = TextDocumentPositionParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        position,
    };
    client_conn
        .sender
        .send(Message::Request(Request {
            id: req_id.clone(),
            method: PrepareRenameRequest::METHOD.to_owned(),
            params: serde_json::to_value(params).unwrap(),
        }))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for prepareRename response");
        }
        match client_conn.receiver.recv_timeout(remaining) {
            Ok(Message::Response(Response { id, result, error })) if id == req_id => {
                assert!(error.is_none(), "prepareRename returned error: {error:?}");
                let value = result.expect("prepareRename had neither result nor error");
                if value.is_null() {
                    return None;
                }
                return Some(serde_json::from_value(value).expect("prepareRename response shape"));
            }
            Ok(_) => {}
            Err(e) => panic!("recv failed while waiting for prepareRename response: {e}"),
        }
    }
}

fn drain_publish_diagnostics_for(client_conn: &Connection, expected: &Url) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for publishDiagnostics for {expected}");
        }
        match client_conn.receiver.recv_timeout(remaining) {
            Ok(Message::Notification(note)) if note.method == PublishDiagnostics::METHOD => {
                let params: PublishDiagnosticsParams =
                    serde_json::from_value(note.params).expect("publishDiagnostics shape");
                if &params.uri == expected {
                    return;
                }
            }
            Ok(_) => {}
            Err(e) => panic!("recv failed while draining diagnostics: {e}"),
        }
    }
}

fn handshake_with_root(client_conn: &Connection, root: &TempDir) {
    let init_id: RequestId = 1.into();
    let root_uri = Url::from_directory_path(root.path()).unwrap();
    #[allow(deprecated)]
    let init_params = InitializeParams {
        capabilities: ClientCapabilities::default(),
        workspace_folders: Some(vec![WorkspaceFolder {
            uri: root_uri.clone(),
            name: "test-root".to_owned(),
        }]),
        root_uri: Some(root_uri),
        ..Default::default()
    };
    client_conn
        .sender
        .send(Message::Request(Request {
            id: init_id.clone(),
            method: Initialize::METHOD.to_owned(),
            params: serde_json::to_value(init_params).unwrap(),
        }))
        .unwrap();

    loop {
        let msg = client_conn
            .receiver
            .recv()
            .expect("connection closed before initialize response");
        if let Message::Response(resp) = msg {
            if resp.id == init_id {
                assert!(resp.error.is_none(), "initialize failed: {:?}", resp.error);
                break;
            }
        }
    }

    client_conn
        .sender
        .send(Message::Notification(Notification {
            method: Initialized::METHOD.to_owned(),
            params: serde_json::to_value(InitializedParams {}).unwrap(),
        }))
        .unwrap();
}

fn send_did_open(client_conn: &Connection, uri: &Url, text: &str) {
    let open_params = DidOpenTextDocumentParams {
        text_document: TextDocumentItem {
            uri: uri.clone(),
            language_id: "tarn".to_owned(),
            version: 1,
            text: text.to_owned(),
        },
    };
    client_conn
        .sender
        .send(Message::Notification(Notification {
            method: DidOpenTextDocument::METHOD.to_owned(),
            params: serde_json::to_value(open_params).unwrap(),
        }))
        .unwrap();
}

fn shutdown_and_join(client_conn: Connection, server_thread: thread::JoinHandle<()>) {
    let shutdown_id: RequestId = 9901.into();
    client_conn
        .sender
        .send(Message::Request(Request {
            id: shutdown_id.clone(),
            method: Shutdown::METHOD.to_owned(),
            params: serde_json::Value::Null,
        }))
        .unwrap();

    loop {
        match client_conn.receiver.recv() {
            Ok(Message::Response(resp)) if resp.id == shutdown_id => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }

    client_conn
        .sender
        .send(Message::Notification(Notification {
            method: Exit::METHOD.to_owned(),
            params: serde_json::Value::Null,
        }))
        .unwrap();
    drop(client_conn);
    server_thread.join().expect("server thread panicked");
}
