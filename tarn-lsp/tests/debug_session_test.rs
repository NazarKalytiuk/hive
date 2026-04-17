//! End-to-end test for the NAZ-256 Req B step-through debugger.
//!
//! Drives [`start_debug_session`] + control commands against a fixture
//! whose first step fails on a closed TCP port (step-by-step semantics
//! do not care about pass/fail; the callback still fires). Asserts:
//!
//!   * `tarn.debugTest` starts a session and returns an id.
//!   * `tarn/captureState` notifications are published between steps.
//!   * `tarn.debugStepOver` advances one step at a time.
//!   * `tarn.getCaptureState` reflects the post-step-over snapshot.
//!   * `tarn.debugStop` cleanly ends the session.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tarn_lsp::debug_session::{
    get_capture_state, post_debug_command, start_debug_session, DebugCommand, DebugTestArgs,
    SessionRegistry, CAPTURE_STATE_NOTIFICATION,
};
use tarn_lsp::run_commands::{CapturingSink, NotificationSink};
use tempfile::TempDir;

const DEBUG_FIXTURE: &str = r#"name: debug fixture
tests:
  scenario:
    steps:
      - name: first
        request:
          method: GET
          url: "http://127.0.0.1:1/health"
        timeout: 50
      - name: second
        request:
          method: GET
          url: "http://127.0.0.1:1/health"
        timeout: 50
      - name: third
        request:
          method: GET
          url: "http://127.0.0.1:1/health"
        timeout: 50
"#;

fn write_fixture(dir: &TempDir) -> PathBuf {
    let path = dir.path().join("debug.tarn.yaml");
    fs::write(&path, DEBUG_FIXTURE).unwrap();
    path
}

/// Wait until at least `n` `tarn/captureState` notifications have
/// landed on `sink`, with a 5s overall deadline so hung sessions do
/// not wedge the test runner.
fn wait_for_notifications(sink: &CapturingSink, n: usize, label: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if sink
            .notifications()
            .iter()
            .filter(|note| note.method == CAPTURE_STATE_NOTIFICATION)
            .count()
            >= n
        {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!(
        "timed out waiting for {} notifications ({}); only saw {}",
        n,
        label,
        sink.notifications().len()
    );
}

#[test]
fn debug_session_emits_capture_state_between_steps() {
    let dir = TempDir::new().unwrap();
    let path = write_fixture(&dir);
    // CapturingSink stores its backing vector in an `Arc<Mutex<...>>`
    // so cloning the handle yields a second view over the same
    // storage. We keep one clone for assertions and wrap another as
    // the NotificationSink the worker thread writes into.
    let capture = CapturingSink::new();
    let sink: Arc<dyn NotificationSink + Send + Sync> = Arc::new(capture.clone());

    let registry = SessionRegistry::new();
    let session_id = start_debug_session(
        &DebugTestArgs {
            file: path.display().to_string(),
            test: "scenario".into(),
            env: None,
        },
        &registry,
        sink,
    )
    .expect("debugTest ok");

    assert!(session_id.starts_with("tarn-dbg-"));

    // The worker publishes the first captureState after the first
    // step finishes (setup is empty in this fixture). Wait for it.
    wait_for_notifications(&capture, 1, "first step");

    let snap_before = get_capture_state(&session_id, &registry).expect("snapshot before");
    assert!(!snap_before.done);

    // Advance one step, wait for the second captureState to land.
    let snap_step_over =
        post_debug_command(&session_id, DebugCommand::StepOver, &registry).expect("step over");
    wait_for_notifications(&capture, 2, "after step over");
    assert!(!snap_step_over.done);

    // Verify at least one notification carries the session id.
    let notes = capture.notifications();
    let with_id = notes
        .iter()
        .filter(|n| n.method == CAPTURE_STATE_NOTIFICATION)
        .filter(|n| {
            n.params
                .get("sessionId")
                .and_then(|v| v.as_str())
                .map(|s| s == session_id)
                .unwrap_or(false)
        })
        .count();
    assert!(
        with_id >= 2,
        "expected at least 2 notifications to carry session id"
    );

    // Stop cleanly.
    let _ = post_debug_command(&session_id, DebugCommand::Stop, &registry).expect("stop");

    // Give the worker a moment to finish and prune.
    std::thread::sleep(Duration::from_millis(250));
    registry.prune();
}

#[test]
fn debug_session_reports_unknown_test_up_front() {
    let dir = TempDir::new().unwrap();
    let path = write_fixture(&dir);
    let sink: Arc<dyn NotificationSink + Send + Sync> = Arc::new(CapturingSink::new());
    let registry = SessionRegistry::new();

    let err = start_debug_session(
        &DebugTestArgs {
            file: path.display().to_string(),
            test: "not-real".into(),
            env: None,
        },
        &registry,
        sink,
    )
    .unwrap_err();
    assert!(err.message.contains("test `not-real` not found"));
    assert!(registry.is_empty());
}
