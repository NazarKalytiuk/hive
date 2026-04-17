//! Step-through "debugger-lite" session runner (NAZ-256 Requirement B).
//!
//! Implements the four `tarn.debug*` commands plus the polling
//! `tarn.getCaptureState` sibling:
//!
//!   * `tarn.debugTest` — start a session, run setup, pause at the first test step.
//!   * `tarn.debugStepOver` — run the current step, pause again.
//!   * `tarn.debugContinue` — run every remaining step; stop at the next failure or end.
//!   * `tarn.debugRerunStep` — re-run the current step without advancing the index.
//!   * `tarn.debugRestart` — abort, re-run setup, start over.
//!   * `tarn.debugStop` — abort the session.
//!   * `tarn.getCaptureState` — read the session's current `(stepIndex, captures, lastResponse)` as a polling alternative to the `tarn/captureState` notification.
//!
//! ## Architecture
//!
//! The session runs on a dedicated worker thread. Commands from the
//! main LSP loop are translated into [`DebugCommand`] values and sent
//! over a bounded channel; the worker responds via a
//! [`DebugState`] snapshot kept behind a mutex so the polling command
//! can read it without blocking the runner. Each time the runner
//! finishes a step it:
//!
//!   1. Publishes a `tarn/captureState` notification through the
//!      [`NotificationSink`] so subscribed clients refresh their UI.
//!   2. Updates the shared [`DebugState`].
//!   3. Blocks on the command channel waiting for the next control
//!      message. [`DebugCommand::Continue`] lets the runner advance
//!      until the next failure or end of test; every other command
//!      pauses after one step.
//!
//! ## Concurrency
//!
//! Sessions live in a [`SessionRegistry`] keyed by a UUID the LSP
//! returns when `tarn.debugTest` is first invoked. Every subsequent
//! control command carries the session id so the server can route to
//! the correct worker. Sessions auto-close when the worker thread
//! finishes (normal end, aborted by `tarn.debugStop`, or panic).

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

use lsp_server::{ErrorCode, Notification, ResponseError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tarn::assert::types::{ResponseInfo, StepResult};
use tarn::env;
use tarn::parser;
use tarn::runner::{self, StepByStepOptions, StepControl, StepOutcome, StepPhase};

use crate::run_commands::{parse_file_arg, NotificationSink};

/// Stable command IDs advertised via [`crate::capabilities`].
pub const DEBUG_TEST_COMMAND: &str = "tarn.debugTest";
pub const DEBUG_CONTINUE_COMMAND: &str = "tarn.debugContinue";
pub const DEBUG_STEP_OVER_COMMAND: &str = "tarn.debugStepOver";
pub const DEBUG_RERUN_STEP_COMMAND: &str = "tarn.debugRerunStep";
pub const DEBUG_RESTART_COMMAND: &str = "tarn.debugRestart";
pub const DEBUG_STOP_COMMAND: &str = "tarn.debugStop";
pub const GET_CAPTURE_STATE_COMMAND: &str = "tarn.getCaptureState";

/// Notification method used by the runner to publish a per-step
/// `captureState` snapshot.
pub const CAPTURE_STATE_NOTIFICATION: &str = "tarn/captureState";

/// Command sent from the main LSP loop to a running debug session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugCommand {
    /// Advance one step and pause again.
    StepOver,
    /// Run every remaining step; pause only on the next failure or at
    /// the end of the test.
    Continue,
    /// Re-run the current step without advancing.
    RerunStep,
    /// Abort the current worker and re-run setup from scratch.
    Restart,
    /// Abort the session.
    Stop,
}

/// Arguments to `tarn.debugTest`.
#[derive(Debug, Clone, Deserialize)]
pub struct DebugTestArgs {
    pub file: String,
    pub test: String,
    #[serde(default)]
    pub env: Option<String>,
}

/// Arguments shared by every follow-up control command
/// (`tarn.debugContinue`, `tarn.debugStepOver`, etc.).
#[derive(Debug, Clone, Deserialize)]
pub struct DebugControlArgs {
    #[serde(rename = "sessionId")]
    pub session_id: String,
}

/// Payload published on every `tarn/captureState` notification.
#[derive(Debug, Clone, Serialize)]
pub struct CaptureStateNotification {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "stepIndex")]
    pub step_index: usize,
    pub phase: String,
    pub captures: HashMap<String, Value>,
    #[serde(rename = "lastResponse")]
    pub last_response: Option<Value>,
    #[serde(rename = "lastStep")]
    pub last_step: Option<Value>,
    pub done: bool,
}

/// Snapshot returned by `tarn.getCaptureState`.
#[derive(Debug, Clone, Serialize)]
pub struct DebugSnapshot {
    #[serde(rename = "sessionId")]
    pub session_id: String,
    #[serde(rename = "stepIndex")]
    pub step_index: usize,
    pub phase: String,
    pub captures: HashMap<String, Value>,
    #[serde(rename = "lastResponse")]
    pub last_response: Option<Value>,
    pub done: bool,
}

/// Shared state updated by the worker and read by the polling command.
#[derive(Debug)]
struct DebugState {
    step_index: usize,
    phase: StepPhase,
    captures: HashMap<String, Value>,
    last_response: Option<Value>,
    last_step: Option<Value>,
    done: bool,
}

impl Default for DebugState {
    fn default() -> Self {
        // Setup is the default phase — the session starts before any
        // test step has run, so `setup` is the most accurate label to
        // surface through `tarn.getCaptureState`.
        Self {
            step_index: 0,
            phase: StepPhase::Setup,
            captures: HashMap::new(),
            last_response: None,
            last_step: None,
            done: false,
        }
    }
}

impl DebugState {
    fn snapshot(&self, session_id: &str) -> DebugSnapshot {
        DebugSnapshot {
            session_id: session_id.to_string(),
            step_index: self.step_index,
            phase: phase_label(self.phase).to_string(),
            captures: self.captures.clone(),
            last_response: self.last_response.clone(),
            done: self.done,
        }
    }
}

fn phase_label(phase: StepPhase) -> &'static str {
    match phase {
        StepPhase::Setup => "setup",
        StepPhase::Test => "test",
        StepPhase::Teardown => "teardown",
    }
}

/// Handle on a running debug session. Owns the worker thread handle and
/// the channel the main thread uses to post control commands.
pub struct DebugSession {
    id: String,
    state: Arc<(Mutex<DebugState>, Condvar)>,
    command_slot: Arc<(Mutex<Option<DebugCommand>>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl DebugSession {
    /// True when the worker thread has exited. Useful for tests and for
    /// the registry's garbage collector.
    pub fn is_finished(&self) -> bool {
        self.worker
            .as_ref()
            .map(|h| h.is_finished())
            .unwrap_or(true)
    }
}

impl Drop for DebugSession {
    fn drop(&mut self) {
        // Best-effort: tell the worker to stop and reap the thread so
        // the process does not leak a session when the registry drops.
        let (lock, cv) = &*self.command_slot;
        if let Ok(mut slot) = lock.lock() {
            *slot = Some(DebugCommand::Stop);
            cv.notify_all();
        }
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

/// Thread-safe registry of active debug sessions. Keyed by session id
/// so control commands can find the right worker.
#[derive(Default)]
pub struct SessionRegistry {
    sessions: Mutex<HashMap<String, Arc<Mutex<DebugSession>>>>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn insert(&self, session: DebugSession) -> String {
        let id = session.id.clone();
        let mut map = self.sessions.lock().expect("session registry mutex");
        map.insert(id.clone(), Arc::new(Mutex::new(session)));
        id
    }

    fn get(&self, session_id: &str) -> Option<Arc<Mutex<DebugSession>>> {
        let map = self.sessions.lock().expect("session registry mutex");
        map.get(session_id).cloned()
    }

    fn remove(&self, session_id: &str) {
        let mut map = self.sessions.lock().expect("session registry mutex");
        map.remove(session_id);
    }

    /// Drop any sessions whose worker thread has exited. Called by every
    /// control command so long-lived LSP servers do not accumulate
    /// finished sessions.
    pub fn prune(&self) {
        let mut map = self.sessions.lock().expect("session registry mutex");
        let finished: Vec<String> = map
            .iter()
            .filter_map(|(k, v)| {
                let guard = v.lock().expect("session mutex");
                if guard.is_finished() {
                    Some(k.clone())
                } else {
                    None
                }
            })
            .collect();
        for id in finished {
            map.remove(&id);
        }
    }

    /// Number of tracked sessions (including finished ones until the
    /// next `prune`).
    pub fn len(&self) -> usize {
        self.sessions.lock().expect("session registry mutex").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Launch a new debug session. Returns the session id on success; the
/// id is then passed back by every follow-up control command.
pub fn start_debug_session(
    args: &DebugTestArgs,
    registry: &SessionRegistry,
    sink: Arc<dyn NotificationSink + Send + Sync>,
) -> Result<String, ResponseError> {
    let file_path = parse_file_arg(&args.file)?;
    let test_file = parser::parse_file(&file_path).map_err(|e| {
        invalid_params(format!("failed to parse `{}`: {}", file_path.display(), e))
    })?;
    // Validate the test exists up front so a bad `tarn.debugTest` fails
    // fast instead of after a worker spin-up round trip.
    let is_named_group = test_file.tests.contains_key(&args.test);
    let is_flat_steps_alias =
        test_file.name == args.test && !test_file.steps.is_empty();
    if !is_named_group && !is_flat_steps_alias {
        return Err(invalid_params(format!(
            "test `{}` not found in `{}`",
            args.test,
            file_path.display()
        )));
    }

    let project_root = file_path
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf();
    let resolved_env = env::resolve_env_with_profiles(
        &test_file.env,
        args.env.as_deref(),
        &[],
        &project_root,
        "tarn.env.yaml",
        &HashMap::new(),
    )
    .map_err(|e| invalid_params(format!("env resolution failed: {e}")))?;

    let session_id = generate_session_id();
    let state = Arc::new((Mutex::new(DebugState::default()), Condvar::new()));
    let command_slot: Arc<(Mutex<Option<DebugCommand>>, Condvar)> =
        Arc::new((Mutex::new(None), Condvar::new()));

    // Clone context for the worker thread.
    let worker_file = file_path.clone();
    let worker_test = args.test.clone();
    let worker_sink = sink.clone();
    let worker_session_id = session_id.clone();
    let worker_state = state.clone();
    let worker_slot = command_slot.clone();
    let worker_test_file = test_file.clone();
    let worker_env = resolved_env.clone();
    let worker_opts = StepByStepOptions::default();

    let worker = thread::Builder::new()
        .name(format!("tarn-debug-{session_id}"))
        .spawn(move || {
            run_worker(WorkerContext {
                session_id: worker_session_id,
                file: worker_file,
                test_name: worker_test,
                test_file: worker_test_file,
                env: worker_env,
                opts: worker_opts,
                state: worker_state,
                command_slot: worker_slot,
                sink: worker_sink,
            });
        })
        .map_err(|e| internal_error(format!("spawn worker: {e}")))?;

    let session = DebugSession {
        id: session_id.clone(),
        state,
        command_slot,
        worker: Some(worker),
    };
    registry.insert(session);
    Ok(session_id)
}

/// Post a [`DebugCommand`] to an existing session. Returns the post-
/// command state snapshot so the client can render the resulting UI
/// without a second round-trip.
pub fn post_debug_command(
    session_id: &str,
    command: DebugCommand,
    registry: &SessionRegistry,
) -> Result<DebugSnapshot, ResponseError> {
    registry.prune();
    let handle = registry
        .get(session_id)
        .ok_or_else(|| invalid_params(format!("unknown debug session `{session_id}`")))?;
    let (state, cmd_slot, should_remove) = {
        let session = handle.lock().expect("session mutex");
        (
            session.state.clone(),
            session.command_slot.clone(),
            command == DebugCommand::Stop,
        )
    };

    {
        let (lock, cv) = &*cmd_slot;
        let mut slot = lock.lock().expect("command slot mutex");
        *slot = Some(command);
        cv.notify_all();
    }

    // Wait for the worker to process the command and either publish a
    // new state or finish. We poll the done flag + step index so we can
    // surface snapshots even when the worker is still running a
    // `Continue` burst. The poll is short (10ms) so the caller never
    // waits longer than the slowest single step.
    //
    // If the command was Stop, we also remove the session from the
    // registry so a follow-up control command gets "unknown session"
    // rather than a stale pointer.
    let snapshot = wait_for_snapshot(session_id, &state, &cmd_slot);
    if should_remove {
        registry.remove(session_id);
    }
    Ok(snapshot)
}

/// Fetch the current state snapshot without advancing the worker.
pub fn get_capture_state(
    session_id: &str,
    registry: &SessionRegistry,
) -> Result<DebugSnapshot, ResponseError> {
    registry.prune();
    let handle = registry
        .get(session_id)
        .ok_or_else(|| invalid_params(format!("unknown debug session `{session_id}`")))?;
    let state = {
        let session = handle.lock().expect("session mutex");
        session.state.clone()
    };
    let (lock, _cv) = &*state;
    let guard = lock.lock().expect("debug state mutex");
    Ok(guard.snapshot(session_id))
}

fn wait_for_snapshot(
    session_id: &str,
    state: &Arc<(Mutex<DebugState>, Condvar)>,
    command_slot: &Arc<(Mutex<Option<DebugCommand>>, Condvar)>,
) -> DebugSnapshot {
    // The worker clears the slot back to `None` once it has consumed
    // the command. We wait for that clear so the snapshot we return
    // reflects the post-command state.
    let (slot_lock, slot_cv) = &**command_slot;
    let mut slot = slot_lock.lock().expect("command slot mutex");
    let timeout = std::time::Duration::from_secs(10);
    while slot.is_some() {
        let (guard, result) = slot_cv
            .wait_timeout(slot, timeout)
            .expect("command slot cv");
        slot = guard;
        if result.timed_out() {
            break;
        }
    }
    drop(slot);
    let (state_lock, _state_cv) = &**state;
    let guard = state_lock.lock().expect("debug state mutex");
    guard.snapshot(session_id)
}

/// Context bundle passed to the worker thread so its spawn closure
/// stays short and the parent can drop every clone it made.
struct WorkerContext {
    session_id: String,
    file: std::path::PathBuf,
    test_name: String,
    test_file: tarn::model::TestFile,
    env: HashMap<String, String>,
    opts: StepByStepOptions,
    state: Arc<(Mutex<DebugState>, Condvar)>,
    command_slot: Arc<(Mutex<Option<DebugCommand>>, Condvar)>,
    sink: Arc<dyn NotificationSink + Send + Sync>,
}

fn run_worker(ctx: WorkerContext) {
    loop {
        let control_state = ctx.state.clone();
        let control_slot = ctx.command_slot.clone();
        let sink = ctx.sink.clone();
        let session_id = ctx.session_id.clone();

        // Mode flag that the Continue command flips so subsequent
        // steps skip the pause step until a failure.
        let continue_until_failure = Arc::new(Mutex::new(false));
        let continue_flag = continue_until_failure.clone();

        let callback_session_id = session_id.clone();
        let on_step = move |outcome: &StepOutcome| -> StepControl {
            publish_state(&control_state, &sink, &callback_session_id, outcome);
            let should_pause = if !outcome.result.passed {
                *continue_flag.lock().expect("continue flag") = false;
                true
            } else {
                !*continue_flag.lock().expect("continue flag")
            };
            if !should_pause {
                return StepControl::Continue;
            }
            wait_for_command(&control_slot, &continue_flag)
        };

        let file_str = ctx.file.display().to_string();
        let run_result = runner::run_test_steps(
            &ctx.test_file,
            &file_str,
            &ctx.env,
            &ctx.test_name,
            &ctx.opts,
            on_step,
        );

        // Mark the session done so polling readers see the terminal
        // snapshot. Publish a sentinel `captureState` with `done=true`
        // so subscribed clients refresh their UI.
        {
            let (lock, cv) = &*ctx.state;
            let mut guard = lock.lock().expect("debug state mutex");
            guard.done = true;
            if let Ok(r) = &run_result {
                if guard.last_response.is_none() {
                    if let Some(last) = r.test_results.last().or(r.setup_results.last()) {
                        if let Some(resp) = &last.response_info {
                            guard.last_response = Some(response_to_value(resp));
                        }
                    }
                }
            }
            cv.notify_all();
        }
        publish_done_notification(
            &ctx.sink,
            &session_id,
            run_result.as_ref().ok().map(|r| &r.captures),
        );

        // If the last command received was a Restart request, reset
        // state and loop back to re-run setup + test. Every other
        // terminal state (worker finished, Stop) exits the outer loop.
        let should_restart = {
            let (slot_lock, _slot_cv) = &*ctx.command_slot;
            let mut slot = slot_lock.lock().expect("command slot mutex");
            matches!(slot.take(), Some(DebugCommand::Restart))
        };
        if !should_restart {
            break;
        }
        // Reset state for a fresh run.
        {
            let (lock, _cv) = &*ctx.state;
            let mut guard = lock.lock().expect("debug state mutex");
            *guard = DebugState::default();
        }
    }
}

fn publish_state(
    state: &Arc<(Mutex<DebugState>, Condvar)>,
    sink: &Arc<dyn NotificationSink + Send + Sync>,
    session_id: &str,
    outcome: &StepOutcome,
) {
    let last_response = outcome
        .result
        .response_info
        .as_ref()
        .map(response_to_value);
    let last_step = step_result_to_value(&outcome.result);
    {
        let (lock, cv) = &**state;
        let mut guard = lock.lock().expect("debug state mutex");
        guard.step_index = outcome.step_index;
        guard.phase = outcome.phase;
        guard.captures = outcome.captures.clone();
        guard.last_response = last_response.clone();
        guard.last_step = Some(last_step.clone());
        guard.done = false;
        cv.notify_all();
    }
    let payload = CaptureStateNotification {
        session_id: session_id.to_string(),
        step_index: outcome.step_index,
        phase: phase_label(outcome.phase).to_string(),
        captures: outcome.captures.clone(),
        last_response,
        last_step: Some(last_step),
        done: false,
    };
    if let Ok(params) = serde_json::to_value(&payload) {
        let _ = sink.send(Notification {
            method: CAPTURE_STATE_NOTIFICATION.to_string(),
            params,
        });
    }
}

fn publish_done_notification(
    sink: &Arc<dyn NotificationSink + Send + Sync>,
    session_id: &str,
    captures: Option<&HashMap<String, Value>>,
) {
    let payload = CaptureStateNotification {
        session_id: session_id.to_string(),
        step_index: 0,
        phase: "finished".to_string(),
        captures: captures.cloned().unwrap_or_default(),
        last_response: None,
        last_step: None,
        done: true,
    };
    if let Ok(params) = serde_json::to_value(&payload) {
        let _ = sink.send(Notification {
            method: CAPTURE_STATE_NOTIFICATION.to_string(),
            params,
        });
    }
}

/// Block on the command slot until a new control command arrives, then
/// map it to a [`StepControl`] value. Restart and Stop both map to Stop
/// because the step-by-step runner re-entry happens at the worker
/// level (outer loop). Continue flips the "run until failure" flag so
/// subsequent steps advance without pausing.
fn wait_for_command(
    command_slot: &Arc<(Mutex<Option<DebugCommand>>, Condvar)>,
    continue_until_failure: &Arc<Mutex<bool>>,
) -> StepControl {
    let (lock, cv) = &**command_slot;
    let mut slot = lock.lock().expect("command slot mutex");
    while slot.is_none() {
        slot = cv.wait(slot).expect("command slot cv");
    }
    let cmd = slot.take().expect("waited for command");
    cv.notify_all();
    match cmd {
        DebugCommand::StepOver => {
            *continue_until_failure.lock().expect("continue flag") = false;
            StepControl::Continue
        }
        DebugCommand::Continue => {
            *continue_until_failure.lock().expect("continue flag") = true;
            StepControl::Continue
        }
        DebugCommand::RerunStep => {
            *continue_until_failure.lock().expect("continue flag") = false;
            StepControl::Retry
        }
        DebugCommand::Restart | DebugCommand::Stop => StepControl::Stop,
    }
}

/// Serialize a step result into a compact JSON object so the
/// captureState notification does not carry the full `StepResult`
/// shape (which is internal and may evolve).
fn step_result_to_value(step: &StepResult) -> Value {
    serde_json::json!({
        "name": step.name,
        "passed": step.passed,
        "duration_ms": step.duration_ms,
        "response_status": step.response_status,
        "response_summary": step.response_summary,
        "captures_set": step.captures_set,
        "assertion_failures": step
            .assertion_results
            .iter()
            .filter(|a| !a.passed)
            .map(|a| serde_json::json!({
                "assertion": a.assertion,
                "expected": a.expected,
                "actual": a.actual,
                "message": a.message,
            }))
            .collect::<Vec<_>>(),
    })
}

fn response_to_value(resp: &ResponseInfo) -> Value {
    serde_json::json!({
        "status": resp.status,
        "headers": resp.headers,
        "body": resp.body,
    })
}

fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    // 64-bit random bits from a fresh PRNG call — deterministic seeding
    // would defeat the point of a session id, but collisions with a
    // PID-suffixed nanosecond counter are vanishingly unlikely in a
    // single process.
    format!("tarn-dbg-{}-{}", std::process::id(), nanos)
}

fn invalid_params(msg: impl Into<String>) -> ResponseError {
    ResponseError {
        code: ErrorCode::InvalidParams as i32,
        message: msg.into(),
        data: None,
    }
}

fn internal_error(msg: impl Into<String>) -> ResponseError {
    ResponseError {
        code: ErrorCode::InternalError as i32,
        message: msg.into(),
        data: None,
    }
}

/// Dispatcher hook for the server: handles every `tarn.debug*` command.
#[allow(clippy::too_many_arguments)]
pub fn dispatch(
    command: &str,
    arguments: &[Value],
    registry: &SessionRegistry,
    sink: Arc<dyn NotificationSink + Send + Sync>,
) -> Option<Result<Value, ResponseError>> {
    let first = arguments.first().cloned();
    let result = match command {
        DEBUG_TEST_COMMAND => {
            let arg = match first {
                Some(v) => v,
                None => {
                    return Some(Err(invalid_params(
                        "tarn.debugTest requires one argument object",
                    )))
                }
            };
            let args: DebugTestArgs = match serde_json::from_value(arg) {
                Ok(v) => v,
                Err(e) => return Some(Err(invalid_params(format!("tarn.debugTest: {e}")))),
            };
            start_debug_session(&args, registry, sink).map(|id| {
                serde_json::json!({
                    "schema_version": 1,
                    "data": {
                        "sessionId": id,
                    }
                })
            })
        }
        DEBUG_STEP_OVER_COMMAND
        | DEBUG_CONTINUE_COMMAND
        | DEBUG_RERUN_STEP_COMMAND
        | DEBUG_RESTART_COMMAND
        | DEBUG_STOP_COMMAND => {
            let arg = match first {
                Some(v) => v,
                None => {
                    return Some(Err(invalid_params(format!(
                        "{command} requires a sessionId argument"
                    ))))
                }
            };
            let args: DebugControlArgs = match serde_json::from_value(arg) {
                Ok(v) => v,
                Err(e) => return Some(Err(invalid_params(format!("{command}: {e}")))),
            };
            let cmd = match command {
                DEBUG_STEP_OVER_COMMAND => DebugCommand::StepOver,
                DEBUG_CONTINUE_COMMAND => DebugCommand::Continue,
                DEBUG_RERUN_STEP_COMMAND => DebugCommand::RerunStep,
                DEBUG_RESTART_COMMAND => DebugCommand::Restart,
                DEBUG_STOP_COMMAND => DebugCommand::Stop,
                _ => unreachable!(),
            };
            post_debug_command(&args.session_id, cmd, registry).and_then(|snap| {
                serde_json::to_value(snap).map_err(|e| internal_error(format!("serialize: {e}")))
            })
        }
        GET_CAPTURE_STATE_COMMAND => {
            let arg = match first {
                Some(v) => v,
                None => {
                    return Some(Err(invalid_params(
                        "tarn.getCaptureState requires a sessionId argument",
                    )))
                }
            };
            let args: DebugControlArgs = match serde_json::from_value(arg) {
                Ok(v) => v,
                Err(e) => {
                    return Some(Err(invalid_params(format!("tarn.getCaptureState: {e}"))))
                }
            };
            get_capture_state(&args.session_id, registry).and_then(|snap| {
                serde_json::to_value(snap).map_err(|e| internal_error(format!("serialize: {e}")))
            })
        }
        _ => return None,
    };
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_commands::CapturingSink;
    use serde_json::json;

    #[test]
    fn phase_label_covers_every_variant() {
        assert_eq!(phase_label(StepPhase::Setup), "setup");
        assert_eq!(phase_label(StepPhase::Test), "test");
        assert_eq!(phase_label(StepPhase::Teardown), "teardown");
    }

    #[test]
    fn registry_starts_empty() {
        let r = SessionRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
    }

    #[test]
    fn get_capture_state_rejects_unknown_session() {
        let r = SessionRegistry::new();
        let err = get_capture_state("not-a-session", &r).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams as i32);
        assert!(err.message.contains("unknown debug session"));
    }

    #[test]
    fn post_debug_command_rejects_unknown_session() {
        let r = SessionRegistry::new();
        let err = post_debug_command("not-a-session", DebugCommand::Stop, &r).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams as i32);
    }

    #[test]
    fn dispatch_returns_none_for_unrelated_command() {
        let r = SessionRegistry::new();
        let sink: Arc<dyn NotificationSink + Send + Sync> = Arc::new(CapturingSink::new());
        let res = dispatch("tarn.not.ours", &[json!({})], &r, sink);
        assert!(res.is_none());
    }

    #[test]
    fn dispatch_control_requires_session_id() {
        let r = SessionRegistry::new();
        let sink: Arc<dyn NotificationSink + Send + Sync> = Arc::new(CapturingSink::new());
        let res = dispatch(DEBUG_STOP_COMMAND, &[json!({})], &r, sink);
        let err = res.unwrap().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams as i32);
    }

    #[test]
    fn dispatch_debug_test_requires_arguments() {
        let r = SessionRegistry::new();
        let sink: Arc<dyn NotificationSink + Send + Sync> = Arc::new(CapturingSink::new());
        let res = dispatch(DEBUG_TEST_COMMAND, &[], &r, sink);
        let err = res.unwrap().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams as i32);
    }
}
