//! End-to-end integration tests for L3.1 `textDocument/formatting`.
//!
//! These tests drive the full `initialize → didOpen → formatting →
//! shutdown → exit` loop over an in-memory `lsp_server::Connection`,
//! mirroring the style of `symbols_test.rs` / `code_lens_test.rs`. Unit
//! tests inside `src/formatting.rs` cover the pure renderer and edit
//! construction; these tests confirm the server dispatches the request
//! correctly and that the `document_formatting_provider` capability is
//! advertised end-to-end.
//!
//! Scenarios:
//!
//!   1. Non-canonical document → one whole-document TextEdit that
//!      reorders fields to the canonical layout.
//!   2. Already-canonical document → empty edit list (no client-side
//!      dirty-mark on a no-op format).
//!   3. Broken YAML document → empty edit list (graceful degrade —
//!      formatting a broken file must never corrupt it).
//!   4. `initialize` response advertises `documentFormattingProvider`.

use std::thread;
use std::time::{Duration, Instant};

use lsp_server::{Connection, Message, Notification, Request, RequestId, Response};
use lsp_types::notification::{
    DidOpenTextDocument, Exit, Initialized, Notification as _, PublishDiagnostics,
};
use lsp_types::request::{Formatting, Initialize, Request as _, Shutdown};
use lsp_types::{
    ClientCapabilities, DidOpenTextDocumentParams, DocumentFormattingParams, FormattingOptions,
    InitializeParams, InitializeResult, InitializedParams, OneOf, PublishDiagnosticsParams,
    TextDocumentIdentifier, TextDocumentItem, TextEdit, Url, WorkDoneProgressParams,
};

/// A deliberately-reordered `.tarn.yaml` document: top-level `steps:`
/// comes before `name:`, and inside the step `request:` precedes
/// `name:` with `url:` before `method:`. The formatter must rewrite all
/// three and emit a canonical version starting with `name:`.
const NON_CANONICAL: &str = "steps:\n- request:\n    url: http://localhost:3000/ping\n    method: GET\n  name: ping\nname: reorder me\n";

/// The canonical version of the fixture above. Used as the
/// "already-formatted" input for the no-op assertion and as the
/// expected output of formatting the non-canonical fixture.
const CANONICAL: &str = "name: reorder me\nsteps:\n- name: ping\n  request:\n    method: GET\n    url: http://localhost:3000/ping\n";

/// Unparseable YAML: a `[` with no matching `]`. `serde_yaml` rejects
/// this outright and `tarn::format::format_document` returns identity +
/// logs, which collapses to an empty edit list at the LSP layer.
const BROKEN: &str = "name: broken\nsteps: [\n  - name: oops\n";

#[test]
fn formatting_on_non_canonical_document_returns_single_whole_document_edit() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/fmt-non-canonical.tarn.yaml").unwrap();
    send_did_open(&client_conn, &uri, NON_CANONICAL);
    drain_publish_diagnostics_for(&client_conn, &uri);

    let edits = request_formatting(&client_conn, &uri);
    assert_eq!(
        edits.len(),
        1,
        "expected one whole-document edit, got {edits:?}"
    );
    let edit = &edits[0];
    assert_eq!(
        edit.new_text, CANONICAL,
        "formatted output must match the canonical layout"
    );
    // Range starts at (0, 0) — we replace the entire buffer, not a
    // subrange. The end point is past the last character so the edit
    // covers every line of the old content.
    assert_eq!(edit.range.start.line, 0);
    assert_eq!(edit.range.start.character, 0);
    assert!(
        edit.range.end.line > 0,
        "end line must be past the first line, got {:?}",
        edit.range.end
    );

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn formatting_on_already_canonical_document_returns_empty_edit_list() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/fmt-canonical.tarn.yaml").unwrap();
    send_did_open(&client_conn, &uri, CANONICAL);
    drain_publish_diagnostics_for(&client_conn, &uri);

    let edits = request_formatting(&client_conn, &uri);
    assert!(
        edits.is_empty(),
        "canonical document must yield zero edits, got {edits:?}"
    );

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn formatting_on_broken_document_returns_empty_edit_list() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });
    handshake(&client_conn);

    let uri = Url::parse("file:///tmp/fmt-broken.tarn.yaml").unwrap();
    send_did_open(&client_conn, &uri, BROKEN);
    // Drain whatever diagnostics the server publishes for this buffer
    // — we do not care whether the validator flags it, only that the
    // formatter degrades gracefully.
    drain_any_publish_diagnostics_for(&client_conn, &uri);

    let edits = request_formatting(&client_conn, &uri);
    assert!(
        edits.is_empty(),
        "broken YAML must yield zero edits (never corrupt the file), got {edits:?}"
    );

    shutdown_and_join(client_conn, server_thread);
}

#[test]
fn initialize_response_advertises_document_formatting_provider() {
    let (server_conn, client_conn) = Connection::memory();
    let server_thread = thread::spawn(move || {
        tarn_lsp::run_with_connection(server_conn).expect("server loop failed");
    });

    let init_id: RequestId = 42.into();
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

    let response = loop {
        let msg = client_conn
            .receiver
            .recv()
            .expect("connection closed before initialize response");
        if let Message::Response(resp) = msg {
            if resp.id == init_id {
                assert!(resp.error.is_none(), "initialize failed: {:?}", resp.error);
                break resp;
            }
        }
    };

    let result: InitializeResult =
        serde_json::from_value(response.result.expect("initialize result")).unwrap();
    // Must advertise the capability so clients know they can send
    // `textDocument/formatting`. `OneOf::Left(true)` is the minimal
    // form `capabilities.rs` emits.
    assert!(
        matches!(
            result.capabilities.document_formatting_provider,
            Some(OneOf::Left(true))
        ),
        "document_formatting_provider must be advertised, got {:?}",
        result.capabilities.document_formatting_provider
    );
    // Range formatting is deliberately NOT advertised — the formatter
    // re-renders the whole buffer so there is no meaningful way to
    // edit a single range without touching the surrounding YAML.
    assert!(
        result
            .capabilities
            .document_range_formatting_provider
            .is_none(),
        "range formatting must NOT be advertised (NAZ-302 out-of-scope),\
         got {:?}",
        result.capabilities.document_range_formatting_provider
    );

    // Finish the handshake so the server loop can exit cleanly via
    // `shutdown_and_join`.
    client_conn
        .sender
        .send(Message::Notification(Notification {
            method: Initialized::METHOD.to_owned(),
            params: serde_json::to_value(InitializedParams {}).unwrap(),
        }))
        .unwrap();

    shutdown_and_join(client_conn, server_thread);
}

// ---------------------------------------------------------------------
// helpers (mirror the ones in symbols_test.rs / code_lens_test.rs)
// ---------------------------------------------------------------------

fn request_formatting(client_conn: &Connection, uri: &Url) -> Vec<TextEdit> {
    let req_id: RequestId = 7301.into();
    let params = DocumentFormattingParams {
        text_document: TextDocumentIdentifier { uri: uri.clone() },
        options: FormattingOptions {
            tab_size: 2,
            insert_spaces: true,
            ..Default::default()
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
    };
    client_conn
        .sender
        .send(Message::Request(Request {
            id: req_id.clone(),
            method: Formatting::METHOD.to_owned(),
            params: serde_json::to_value(params).unwrap(),
        }))
        .unwrap();

    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for formatting response");
        }
        match client_conn.receiver.recv_timeout(remaining) {
            Ok(Message::Response(Response { id, result, error })) if id == req_id => {
                assert!(error.is_none(), "formatting returned error: {error:?}");
                // The LSP spec allows either `null` or `[]` for "no
                // edits"; our server always returns an array. Accept
                // both shapes so the test is robust to small lsp-types
                // serialisation tweaks.
                let value = result.expect("formatting had neither result nor error");
                if value.is_null() {
                    return Vec::new();
                }
                return serde_json::from_value(value).expect("formatting response shape");
            }
            Ok(_) => {}
            Err(e) => panic!("recv failed while waiting for formatting response: {e}"),
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

/// Like [`drain_publish_diagnostics_for`], but does not panic if the
/// server never publishes for `expected` within the deadline. Used for
/// the broken-YAML scenario, where we want to consume any diagnostic
/// that fires but also want the test to pass if none does.
fn drain_any_publish_diagnostics_for(client_conn: &Connection, expected: &Url) {
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return;
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
            // Timeout while draining is fine — nothing to consume.
            Err(_) => return,
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
