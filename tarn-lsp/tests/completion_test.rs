//! End-to-end integration tests for L1.4 completion.
//!
//! These mirror the L1.3 hover integration tests in
//! `tests/hover_test.rs`: a memory-transport `Connection` drives the
//! full `initialize → didOpen → completion → shutdown → exit` round
//! trip, so the dispatch wiring and `CompletionParams` deserializer
//! are exercised together. Pure context-detection and per-scope
//! renderer logic is covered by unit tests inside `src/completion.rs`.

use std::thread;
use std::time::{Duration, Instant};

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidOpenTextDocument, Exit, Initialized, Notification as _, PublishDiagnostics,
};
use lsp_types::request::{Completion, Initialize, Request as _, Shutdown};
use lsp_types::{
    ClientCapabilities, CompletionItem, CompletionItemKind, CompletionParams, CompletionResponse,
    DidOpenTextDocumentParams, InitializeParams, InitializedParams, PartialResultParams, Position,
    TextDocumentIdentifier, TextDocumentItem, TextDocumentPositionParams, Url,
    WorkDoneProgressParams,
};

/// Fixture covering every completion surface L1.4 ships.
/// Line numbers (0-based):
///   0  name: completion fixture
///   1  env:
///   2    base_url: http://localhost:3000
///   3    api_key: secret
///   4  setup:
///   5    - name: seed
///   6      request:
///   7        method: POST
///   8        url: "{{ env. }}/seed"
///   9      capture:
///  10        seed_id: $.id
///  11  tests:
///  12    main:
///  13      steps:
///  14        - name: read
///  15          request:
///  16            method: GET
///  17            url: "{{ capture. }}/items"
///  18            body:
///  19              id: "{{ $ }}"
const FIXTURE: &str = r#"name: completion fixture
env:
  base_url: http://localhost:3000
  api_key: secret
setup:
  - name: seed
    request:
      method: POST
      url: "{{ env. }}/seed"
    capture:
      seed_id: $.id
tests:
  main:
    steps:
      - name: read
        request:
          method: GET
          url: "{{ capture. }}/items"
          body:
            id: "{{ $ }}"
"#;

#[test]
fn completion_inside_env_interpolation_returns_env_keys() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/cmp-env.tarn.yaml").unwrap();
    send_did_open(&client_conn, &uri, FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Line 8: `      url: "{{ env. }}/seed"`. Cursor immediately after
    // the `.` in `env.` — find it.
    let env_dot_col = find_substring_column(FIXTURE, 8, "env.") + "env.".len();
    let items = request_completion(&client_conn, &uri, Position::new(8, env_dot_col as u32))
        .expect("completion returned null inside env scope");

    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        labels.contains(&"base_url"),
        "missing base_url in {labels:?}"
    );
    assert!(labels.contains(&"api_key"), "missing api_key in {labels:?}");
    // Env keys must carry Variable kind with the resolved value in detail.
    let base_url = items
        .iter()
        .find(|i| i.label == "base_url")
        .expect("base_url not returned");
    assert_eq!(base_url.kind, Some(CompletionItemKind::VARIABLE));
    assert_eq!(base_url.detail.as_deref(), Some("http://localhost:3000"));
    assert!(base_url.sort_text.is_some(), "env items need sort_text");

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn completion_inside_capture_interpolation_returns_visible_captures() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/cmp-capture.tarn.yaml").unwrap();
    send_did_open(&client_conn, &uri, FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Line 17: `            url: "{{ capture. }}/items"`.
    let cap_dot_col = find_substring_column(FIXTURE, 17, "capture.") + "capture.".len();
    let items = request_completion(&client_conn, &uri, Position::new(17, cap_dot_col as u32))
        .expect("completion returned null inside capture scope");

    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(
        labels.contains(&"seed_id"),
        "missing seed_id capture in {labels:?}"
    );
    let seed_id = items.iter().find(|i| i.label == "seed_id").unwrap();
    assert_eq!(seed_id.kind, Some(CompletionItemKind::VARIABLE));

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn completion_inside_builtin_interpolation_returns_all_five_functions() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/cmp-builtin.tarn.yaml").unwrap();
    send_did_open(&client_conn, &uri, FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Line 19: `            id: "{{ $ }}"`. Cursor immediately after `$`.
    let dollar_col = find_substring_column(FIXTURE, 19, "$") + 1;
    let items = request_completion(&client_conn, &uri, Position::new(19, dollar_col as u32))
        .expect("completion returned null inside builtin scope");

    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"uuid"));
    assert!(labels.contains(&"timestamp"));
    assert!(labels.contains(&"now_iso"));
    assert!(labels.contains(&"random_hex"));
    assert!(labels.contains(&"random_int"));
    for item in &items {
        assert_eq!(item.kind, Some(CompletionItemKind::FUNCTION));
    }

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn completion_at_blank_top_level_line_returns_schema_keys() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/cmp-root.tarn.yaml").unwrap();
    // A minimal root fixture with one blank line where the cursor sits.
    let src = "name: root-fixture\n\nsteps: []\n";
    send_did_open(&client_conn, &uri, src);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Line 1 is the blank line.
    let items = request_completion(&client_conn, &uri, Position::new(1, 0))
        .expect("completion returned null at blank root line");

    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"name"), "missing name in {labels:?}");
    assert!(labels.contains(&"tests"), "missing tests in {labels:?}");
    assert!(labels.contains(&"env"), "missing env in {labels:?}");
    for item in &items {
        assert_eq!(item.kind, Some(CompletionItemKind::PROPERTY));
    }

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn completion_nested_inside_request_offers_request_fields() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/cmp-nested-request.tarn.yaml").unwrap();
    // Blank line directly under `request:` — the nested walker
    // should surface the Request schema's children (method, url,
    // headers, body, form, multipart, …).
    let src = "name: nested-request\nsteps:\n  - name: ping\n    request:\n      \n";
    send_did_open(&client_conn, &uri, src);
    drain_publish_diagnostics_for(&client_conn, &uri);

    let items = request_completion(&client_conn, &uri, Position::new(4, 6))
        .expect("nested request completion returned null");
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    for want in ["method", "url", "headers", "body", "form", "multipart"] {
        assert!(
            labels.contains(&want),
            "missing `{want}` in nested-request completion: {labels:?}"
        );
    }
    // Must NOT bleed back to the step-level hard-coded key list.
    assert!(
        !labels.contains(&"request"),
        "leaked step-level key into nested completion: {labels:?}"
    );
    assert!(
        !labels.contains(&"capture"),
        "leaked step-level key into nested completion: {labels:?}"
    );

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn completion_nested_inside_assert_body_jsonpath_offers_matchers() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/cmp-nested-assert-body.tarn.yaml").unwrap();
    // Cursor is one level deeper than `"$.id":` — completion should
    // offer BodyAssertionOperators (eq, matches, length, …).
    let src = concat!(
        "name: nested-assert\n",
        "steps:\n",
        "  - name: check\n",
        "    request:\n",
        "      method: GET\n",
        "      url: http://localhost/x\n",
        "    assert:\n",
        "      body:\n",
        "        \"$.id\":\n",
        "          \n",
    );
    send_did_open(&client_conn, &uri, src);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Line 9 is `          ` (10 spaces). Cursor col 10.
    let items = request_completion(&client_conn, &uri, Position::new(9, 10))
        .expect("nested assert.body completion returned null");
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    for want in ["eq", "matches", "length", "type", "is_uuid", "contains"] {
        assert!(
            labels.contains(&want),
            "missing matcher `{want}` in nested assert.body completion: {labels:?}"
        );
    }

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn completion_nested_inside_poll_offers_pollconfig_fields() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/cmp-nested-poll.tarn.yaml").unwrap();
    let src = concat!(
        "name: nested-poll\n",
        "steps:\n",
        "  - name: await\n",
        "    request:\n",
        "      method: GET\n",
        "      url: http://localhost/x\n",
        "    poll:\n",
        "      \n",
    );
    send_did_open(&client_conn, &uri, src);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Line 7 is `      ` (6 spaces). Cursor col 6.
    let items = request_completion(&client_conn, &uri, Position::new(7, 6))
        .expect("nested poll completion returned null");
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.contains(&"until"), "missing until in {labels:?}");
    assert!(
        labels.contains(&"interval"),
        "missing interval in {labels:?}"
    );
    assert!(
        labels.contains(&"max_attempts"),
        "missing max_attempts in {labels:?}"
    );

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn completion_in_plain_text_returns_null() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/cmp-none.tarn.yaml").unwrap();
    send_did_open(&client_conn, &uri, FIXTURE);
    drain_publish_diagnostics_for(&client_conn, &uri);

    // Line 0 col 4 is inside `name:` — partially typed key, not a blank
    // line, not an interpolation. Should get a null response.
    let items = request_completion(&client_conn, &uri, Position::new(0, 4));
    assert!(
        items.is_none(),
        "expected null completion for mid-identifier cursor, got {items:?}"
    );

    shutdown_and_join(client_conn, server_thread);
}

// ---------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------

fn request_completion(
    client_conn: &Connection,
    uri: &Url,
    position: Position,
) -> Option<Vec<CompletionItem>> {
    let req_id: RequestId = 8001.into();
    let params = CompletionParams {
        text_document_position: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            position,
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
        partial_result_params: PartialResultParams::default(),
        context: None,
    };
    client_conn
        .sender
        .send(Message::Request(Request {
            id: req_id.clone(),
            method: Completion::METHOD.to_owned(),
            params: serde_json::to_value(params).unwrap(),
        }))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for completion response");
        }
        match client_conn.receiver.recv_timeout(remaining) {
            Ok(Message::Response(Response { id, result, error })) if id == req_id => {
                assert!(error.is_none(), "completion returned error: {error:?}");
                let value = result.expect("completion had neither result nor error");
                if value.is_null() {
                    return None;
                }
                let response: CompletionResponse =
                    serde_json::from_value(value).expect("completion response shape");
                return Some(match response {
                    CompletionResponse::Array(items) => items,
                    CompletionResponse::List(list) => list.items,
                });
            }
            Ok(_) => {}
            Err(e) => panic!("recv failed while waiting for completion response: {e}"),
        }
    }
}

/// Locate `needle` on `line` (0-based) in `source` and return the
/// 0-based column of its first character. Panics if not found.
fn find_substring_column(source: &str, line: usize, needle: &str) -> usize {
    let line_text = source.lines().nth(line).unwrap_or("");
    line_text.find(needle).unwrap_or_else(|| {
        panic!("substring `{needle}` not found on line {line}: `{line_text}`");
    })
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
