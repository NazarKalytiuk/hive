//! End-to-end integration tests for L2.2 `textDocument/references`.
//!
//! These drive the full `initialize → didOpen → textDocument/references
//! → shutdown → exit` loop over an in-memory `lsp_server::Connection`,
//! the same harness L1.3/L1.4/L1.5/L2.1 already use. The pure renderer
//! and workspace index are unit-tested inside `src/references.rs` and
//! `src/workspace.rs`; this file confirms the dispatch wiring is correct
//! and that the cross-file walk actually picks up references in files
//! the client never opens.

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidOpenTextDocument, Exit, Initialized, Notification as _, PublishDiagnostics,
};
use lsp_types::request::{Initialize, References, Request as _, Shutdown};
use lsp_types::{
    ClientCapabilities, DidOpenTextDocumentParams, InitializeParams, InitializedParams, Location,
    PartialResultParams, Position, PublishDiagnosticsParams, ReferenceContext, ReferenceParams,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams, Url,
    WorkDoneProgressParams, WorkspaceFolder,
};
use tempfile::TempDir;

const FIXTURE_A: &str = r#"name: alpha
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

const FIXTURE_B: &str = r#"name: beta
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

#[test]
fn references_walks_cross_file_workspace_for_env_key() {
    let dir = TempDir::new().unwrap();
    let alpha_path = write(dir.path(), "alpha.tarn.yaml", FIXTURE_A);
    let _beta_path = write(dir.path(), "beta.tarn.yaml", FIXTURE_B);

    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake_with_root(&client_conn, &dir);

    let alpha_uri = Url::from_file_path(&alpha_path).unwrap();
    send_did_open(&client_conn, &alpha_uri, FIXTURE_A);
    drain_publish_diagnostics_for(&client_conn, &alpha_uri);

    // Cursor inside `env.base_url` on alpha.tarn.yaml line 9 (0-based).
    // Position 20 lands inside the `base_url` identifier.
    let refs = request_references(&client_conn, &alpha_uri, Position::new(9, 20), false);
    assert_eq!(
        refs.len(),
        3,
        "1 use site in alpha + 2 use sites in beta = 3 (got {:?})",
        refs
    );
    let alpha_count = refs.iter().filter(|r| r.uri == alpha_uri).count();
    assert_eq!(alpha_count, 1, "alpha.tarn.yaml has one use site");
    let beta_count = refs.iter().filter(|r| r.uri != alpha_uri).count();
    assert_eq!(beta_count, 2, "beta.tarn.yaml has two use sites");

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn references_with_include_declaration_adds_inline_env_key() {
    let dir = TempDir::new().unwrap();
    let alpha_path = write(dir.path(), "alpha.tarn.yaml", FIXTURE_A);

    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake_with_root(&client_conn, &dir);

    let alpha_uri = Url::from_file_path(&alpha_path).unwrap();
    send_did_open(&client_conn, &alpha_uri, FIXTURE_A);
    drain_publish_diagnostics_for(&client_conn, &alpha_uri);

    let with_decl = request_references(&client_conn, &alpha_uri, Position::new(9, 20), true);
    let without_decl = request_references(&client_conn, &alpha_uri, Position::new(9, 20), false);

    assert!(
        with_decl.len() == without_decl.len() + 1,
        "include_declaration=true should return one extra entry; with={:?} without={:?}",
        with_decl,
        without_decl
    );
    assert!(
        with_decl
            .iter()
            .any(|loc| loc.uri == alpha_uri && loc.range.start.line == 2),
        "expected the inline env declaration on line 2; got {:?}",
        with_decl
    );

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn references_for_capture_token_returns_use_sites_in_same_file() {
    let dir = TempDir::new().unwrap();
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
          url: "http://x/items?key={{ capture.token }}"
"#;
    let cap_path = write(dir.path(), "cap.tarn.yaml", CAPTURE_FIXTURE);

    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake_with_root(&client_conn, &dir);

    let cap_uri = Url::from_file_path(&cap_path).unwrap();
    send_did_open(&client_conn, &cap_uri, CAPTURE_FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &cap_uri);

    // Cursor inside the first `capture.token` use site on line 13 (0-based).
    let refs = request_references(&client_conn, &cap_uri, Position::new(13, 30), false);
    assert_eq!(refs.len(), 2, "two use sites in same file (got {:?})", refs);
    assert!(refs.iter().all(|r| r.uri == cap_uri));

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn references_on_builtin_token_returns_empty_array() {
    let dir = TempDir::new().unwrap();
    const BUILTIN_FIXTURE: &str = r#"name: builtin
steps:
  - name: ping
    request:
      method: GET
      url: "http://localhost/{{ $uuid }}"
"#;
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
    let refs = request_references(&client_conn, &uri, Position::new(5, 30), true);
    assert!(
        refs.is_empty(),
        "builtin token must produce empty references (got {:?})",
        refs
    );

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

fn request_references(
    client_conn: &Connection,
    uri: &Url,
    position: Position,
    include_declaration: bool,
) -> Vec<Location> {
    let req_id: RequestId = 9301.into();
    let params = ReferenceParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            position,
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
        context: ReferenceContext {
            include_declaration,
        },
    };
    client_conn
        .sender
        .send(Message::Request(Request {
            id: req_id.clone(),
            method: References::METHOD.to_owned(),
            params: serde_json::to_value(params).unwrap(),
        }))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for references response");
        }
        match client_conn.receiver.recv_timeout(remaining) {
            Ok(Message::Response(Response { id, result, error })) if id == req_id => {
                assert!(error.is_none(), "references returned error: {error:?}");
                let value = result.expect("references had neither result nor error");
                return serde_json::from_value(value).expect("references response shape");
            }
            Ok(_) => {}
            Err(e) => panic!("recv failed while waiting for references response: {e}"),
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
