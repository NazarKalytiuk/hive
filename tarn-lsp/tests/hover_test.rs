//! End-to-end integration tests for L1.3 hover.
//!
//! These tests drive `tarn-lsp` over an in-memory
//! `lsp_server::Connection`, mirroring the structure of
//! `diagnostics_test.rs`. They exercise the full `initialize → didOpen →
//! hover → shutdown → exit` round trip, which is the only place where
//! the hover dispatch wiring and the request deserializer are tested
//! together. The pure helpers in `src/hover.rs` cover the rest.

use std::thread;
use std::time::{Duration, Instant};

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidOpenTextDocument, Exit, Initialized, Notification as _, PublishDiagnostics,
};
use lsp_types::request::{HoverRequest, Initialize, Request as _, Shutdown};
use lsp_types::{
    ClientCapabilities, DidOpenTextDocumentParams, Hover, HoverContents, HoverParams,
    InitializeParams, InitializedParams, MarkupKind, Position, TextDocumentIdentifier,
    TextDocumentItem, TextDocumentPositionParams, Url, WorkDoneProgressParams,
};

/// Fixture document with one of each hover token kind we care about.
/// Line numbers (0-based):
///   0  name: hover fixture
///   1  env:
///   2    base_url: http://localhost:3000
///   3  setup:
///   4    - name: seed
///   5      request:
///   6        method: POST
///   7        url: "{{ env.base_url }}/seed"
///   8      capture:
///   9        seed_id: $.id
///  10  tests:
///  11    main:
///  12      steps:
///  13        - name: read
///  14          request:
///  15            method: GET
///  16            url: "{{ env.base_url }}/items/{{ capture.seed_id }}"
///  17          assert:
///  18            status: 200
const FIXTURE: &str = r#"name: hover fixture
env:
  base_url: http://localhost:3000
setup:
  - name: seed
    request:
      method: POST
      url: "{{ env.base_url }}/seed"
    capture:
      seed_id: $.id
tests:
  main:
    steps:
      - name: read
        request:
          method: GET
          url: "{{ env.base_url }}/items/{{ capture.seed_id }}"
        assert:
          status: 200
"#;

#[test]
fn hover_over_env_token_returns_markdown_with_value_and_source() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/hover-env.tarn.yaml").unwrap();
    send_did_open(&client_conn, &uri, FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Line 7 is `      url: "{{ env.base_url }}/seed"`. Column 20 sits
    // inside `base_url`.
    let hover = request_hover(&client_conn, &uri, Position::new(7, 20));
    let hover = hover.expect("expected a Hover for {{ env.base_url }}");
    let body = markdown_body(&hover);
    assert!(body.contains("env.base_url"));
    assert!(body.contains("http://localhost:3000"));
    // Inline env block has no file path, but the source label should
    // still identify where the value came from.
    assert!(body.contains("inline env: block"));
    assert!(body.contains("Redacted: `no`"));

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn hover_over_capture_token_returns_step_and_source() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/hover-capture.tarn.yaml").unwrap();
    send_did_open(&client_conn, &uri, FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Line 16 has `{{ capture.seed_id }}`. The `s` in `seed_id` starts
    // around column 55. Hover inside the identifier.
    let seed_id_col = find_substring_column(FIXTURE, 16, "seed_id");
    let hover = request_hover(&client_conn, &uri, Position::new(16, seed_id_col as u32));
    let hover = hover.expect("expected a Hover for {{ capture.seed_id }}");
    let body = markdown_body(&hover);
    assert!(body.contains("capture.seed_id"));
    assert!(body.contains("step `seed`"));
    assert!(body.contains("JSONPath `$.id`"));

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn hover_over_builtin_function_returns_signature_and_doc() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/hover-builtin.tarn.yaml").unwrap();
    // A minimal fixture using a builtin. We don't need the main
    // FIXTURE here because builtin hovers don't consult the document
    // at all — but we still have to open a valid file or the hover
    // handler would refuse to look the URI up in the store.
    let src = r#"name: builtin
steps:
  - name: bootstrap
    request:
      method: POST
      url: "http://localhost/widget"
      body:
        id: "{{ $uuid }}"
    assert:
      status: 201
"#;
    send_did_open(&client_conn, &uri, src);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Line 7: `        id: "{{ $uuid }}"`. The `u` of `$uuid` is around
    // col 17.
    let uuid_col = find_substring_column(src, 7, "uuid");
    let hover = request_hover(&client_conn, &uri, Position::new(7, uuid_col as u32));
    let hover = hover.expect("expected a Hover for {{ $uuid }}");
    let body = markdown_body(&hover);
    assert!(body.contains("$uuid"));
    assert!(body.contains("UUID"));

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn hover_over_top_level_schema_key_returns_description() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/hover-schema.tarn.yaml").unwrap();
    send_did_open(&client_conn, &uri, FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Line 18 is `          assert:` — but schema key hover fires on
    // the inner key name. Line 19 is `            status: 200` (1-based
    // line 20 in the file). Cursor on `status`.
    let status_col = find_substring_column(FIXTURE, 18, "status");
    let hover = request_hover(&client_conn, &uri, Position::new(18, status_col as u32));
    let hover = hover.expect("expected a Hover for `status:`");
    let body = markdown_body(&hover);
    assert!(body.contains("`status`"));
    // The description should mention the HTTP status code semantics.
    assert!(
        body.to_lowercase().contains("status code"),
        "schema description missing, got: {body}"
    );

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn hover_over_plain_text_returns_null() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/hover-none.tarn.yaml").unwrap();
    send_did_open(&client_conn, &uri, FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Line 0, column 14 — somewhere inside the literal text "hover
    // fixture" on the `name:` line. Not an interpolation token, not a
    // known schema key position, must come back as JSON null.
    let hover = request_hover(&client_conn, &uri, Position::new(0, 14));
    assert!(hover.is_none(), "expected null hover, got {hover:?}");

    shutdown_and_join(client_conn, server_thread);
}

// ---------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------

fn markdown_body(hover: &Hover) -> String {
    match &hover.contents {
        HoverContents::Markup(m) => {
            assert_eq!(m.kind, MarkupKind::Markdown, "hover must be markdown");
            m.value.clone()
        }
        other => panic!("expected markup hover, got {other:?}"),
    }
}

/// Locate `needle` on `line` (0-based) in `source` and return the
/// 0-based column of its first character. Panics if the substring is
/// missing — tests should never call this with an impossible target.
fn find_substring_column(source: &str, line: usize, needle: &str) -> usize {
    let line_text = source.lines().nth(line).unwrap_or("");
    line_text.find(needle).unwrap_or_else(|| {
        panic!("substring `{needle}` not found on line {line}: `{line_text}`");
    })
}

fn request_hover(client_conn: &Connection, uri: &Url, position: Position) -> Option<Hover> {
    let req_id: RequestId = 7001.into();
    let params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            position,
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
    };
    client_conn
        .sender
        .send(Message::Request(Request {
            id: req_id.clone(),
            method: HoverRequest::METHOD.to_owned(),
            params: serde_json::to_value(params).unwrap(),
        }))
        .unwrap();

    // Drain unrelated messages (publishDiagnostics, etc.) until the
    // hover response arrives.
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for hover response");
        }
        match client_conn.receiver.recv_timeout(remaining) {
            Ok(Message::Response(Response { id, result, error })) if id == req_id => {
                assert!(error.is_none(), "hover returned error: {error:?}");
                let value = result.expect("hover returned neither result nor error");
                if value.is_null() {
                    return None;
                }
                let hover: Hover = serde_json::from_value(value).expect("hover response shape");
                return Some(hover);
            }
            Ok(_) => {}
            Err(e) => panic!("recv failed while waiting for hover response: {e}"),
        }
    }
}

/// Drain the `publishDiagnostics` notification that `didOpen` triggers,
/// so subsequent `recv` calls don't pick it up by accident.
fn drain_publish_diagnostics_for(client_conn: &Connection, expected: &Url) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for publishDiagnostics for {expected}");
        }
        match client_conn.receiver.recv_timeout(remaining) {
            Ok(Message::Notification(note)) if note.method == PublishDiagnostics::METHOD => {
                let params: lsp_types::PublishDiagnosticsParams =
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

fn handshake(client_conn: &Connection) {
    let init_id: RequestId = 1.into();
    client_conn
        .sender
        .send(Message::Request(Request {
            id: init_id.clone(),
            method: Initialize::METHOD.to_owned(),
            params: serde_json::to_value(InitializeParams {
                capabilities: ClientCapabilities::default(),
                ..Default::default()
            })
            .unwrap(),
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
    let shutdown_id: RequestId = 9001.into();
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
