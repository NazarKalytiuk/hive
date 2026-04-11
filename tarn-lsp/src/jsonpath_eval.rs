//! `workspace/executeCommand` handler for `tarn.evaluateJsonpath`.
//!
//! Ships with L3.6 (NAZ-307) as the companion to the JSONPath hover
//! class in [`crate::hover`]. LSP clients (Claude Code, the upcoming
//! VS Code migration under Phase V, any generic LSP consumer) can
//! invoke this command to evaluate a JSONPath against either:
//!
//!   * an **inline response** — the client hands over the full
//!     response body as a JSON value, and the handler returns the
//!     matches without touching the filesystem.
//!   * a **step reference** — the client identifies a step in an
//!     open buffer by `(file, test, step)` triple, and the handler
//!     looks up the sidecar response via the same
//!     [`RecordedResponseSource`] trait the scaffold-assert code
//!     action already consumes.
//!
//! Both argument shapes share the same return envelope, documented
//! in [`EvaluationResult`], so the client always knows where the
//! matches live regardless of which lookup path fired.
//!
//! ## Error policy
//!
//! Every soft failure (parse error, missing step, missing sidecar,
//! empty / malformed arguments) collapses to [`lsp_server::ErrorCode::InvalidParams`]
//! with a human-readable message. This is stricter than the hover
//! provider — hover always wants to render *something*, whereas the
//! command is invoked programmatically and benefits from explicit
//! errors so the caller can retry with corrected arguments.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use lsp_server::{ErrorCode, ResponseError};
use lsp_types::{ExecuteCommandParams, Url};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tarn::jsonpath::{evaluate_path, JsonPathError};

use crate::code_actions::response_source::{DiskResponseSource, RecordedResponseSource};
use crate::server::ServerState;

/// Stable LSP command id advertised in [`crate::capabilities`] and
/// dispatched by [`crate::server::dispatch_request`]. Exposed as a
/// constant so the tests, the capability advertisement, and the
/// server wiring all reference one source of truth.
pub const EVALUATE_JSONPATH_COMMAND: &str = "tarn.evaluateJsonpath";

/// Arguments accepted by `tarn.evaluateJsonpath`.
///
/// Uses an untagged enum so the client can pick between the two
/// shapes based on whichever context it has available. Clients that
/// are already sitting on a recorded response choose
/// [`EvaluateArgs::Inline`]; clients that only know the enclosing
/// step choose [`EvaluateArgs::StepRef`].
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum EvaluateArgs {
    /// `{ "path": "<jsonpath>", "response": <inline-json-value> }`
    Inline {
        /// JSONPath expression to evaluate.
        path: String,
        /// Inline JSON response body to evaluate against. Any valid
        /// JSON value is accepted — object, array, scalar, `null`.
        response: Value,
    },
    /// `{ "path": "<jsonpath>", "step": { "file": "...", "test": "...", "step": "..." } }`
    StepRef {
        /// JSONPath expression to evaluate.
        path: String,
        /// Step reference used to look up the sidecar response.
        step: StepRef,
    },
}

/// A step reference used by [`EvaluateArgs::StepRef`] to resolve the
/// recorded response through the sidecar convention (NAZ-304).
#[derive(Debug, Clone, Deserialize)]
pub struct StepRef {
    /// Absolute filesystem path of the `.tarn.yaml` buffer. An LSP
    /// `file://` URI is also accepted — the handler converts it.
    pub file: String,
    /// Enclosing test group's name, or the sentinel `"setup"` /
    /// `"teardown"` / `"<flat>"` for steps outside any test.
    pub test: String,
    /// Step's `name:` value.
    pub step: String,
}

/// Return envelope from `tarn.evaluateJsonpath`.
///
/// The shape is deliberately explicit (`{ "matches": [...] }`) so
/// clients never have to guess whether a bare JSON value was
/// originally one match or the raw document. Future expansions
/// (e.g. match locations inside the response document) can add
/// fields without a source-breaking change.
#[derive(Debug, Clone, Serialize)]
pub struct EvaluationResult {
    /// Every match the JSONPath produced, in document order. An
    /// empty vector is a valid success response — it means "the
    /// path parsed but matched no values."
    pub matches: Vec<Value>,
}

/// Parse and validate the raw `ExecuteCommandParams.arguments`
/// payload into an [`EvaluateArgs`].
///
/// Callers that only need the argument-parse step (unit tests,
/// future command providers that want to reuse the shape) can call
/// this without invoking the full command dispatch.
pub fn parse_evaluate_args(args: &[Value]) -> Result<EvaluateArgs, ResponseError> {
    let first = args
        .first()
        .ok_or_else(|| invalid_params("tarn.evaluateJsonpath requires one argument object"))?;
    serde_json::from_value::<EvaluateArgs>(first.clone()).map_err(|e| {
        invalid_params(format!(
            "tarn.evaluateJsonpath: invalid argument shape: {e}. Expected {{\"path\": ..., \"response\": ...}} or {{\"path\": ..., \"step\": {{\"file\": ..., \"test\": ..., \"step\": ...}}}}"
        ))
    })
}

/// Resolve an [`EvaluateArgs`] to its underlying response body,
/// pulling the sidecar JSON through `source` for the step-ref branch
/// and passing inline values straight through otherwise.
///
/// Returns an `InvalidParams` response error on any lookup failure
/// so the command can surface a precise reason string to the client.
pub fn resolve_response(
    args: &EvaluateArgs,
    source: &dyn RecordedResponseSource,
) -> Result<Value, ResponseError> {
    match args {
        EvaluateArgs::Inline { response, .. } => Ok(response.clone()),
        EvaluateArgs::StepRef { step, .. } => {
            let path = step_file_to_pathbuf(&step.file);
            source.read(&path, &step.test, &step.step).ok_or_else(|| {
                invalid_params(format!(
                    "tarn.evaluateJsonpath: no recorded response found for step `{}` in test `{}` at `{}`. Run the step at least once to populate the sidecar.",
                    step.step,
                    step.test,
                    path.display()
                ))
            })
        }
    }
}

/// Convert a step-ref `file` field into a filesystem path. Accepts
/// both bare filesystem strings and `file://` URIs — the latter are
/// normalised through [`Url::to_file_path`] so Windows drive letters
/// come through intact.
fn step_file_to_pathbuf(file: &str) -> PathBuf {
    if let Ok(url) = Url::parse(file) {
        if let Ok(p) = url.to_file_path() {
            return p;
        }
    }
    PathBuf::from(file)
}

/// Dispatch one `workspace/executeCommand` request to the right
/// handler.
///
/// Today only `tarn.evaluateJsonpath` is registered. Unknown command
/// IDs fall through to [`ErrorCode::MethodNotFound`] so clients get
/// a clear "not implemented" signal rather than a silent null.
pub fn workspace_execute_command(
    _state: &ServerState,
    params: ExecuteCommandParams,
) -> Result<Option<Value>, ResponseError> {
    match params.command.as_str() {
        EVALUATE_JSONPATH_COMMAND => {
            let source: Arc<dyn RecordedResponseSource> = Arc::new(DiskResponseSource);
            let result = execute_evaluate_jsonpath(&params.arguments, source.as_ref())?;
            Ok(Some(
                serde_json::to_value(result).map_err(internal_error_from_serde)?,
            ))
        }
        other => Err(ResponseError {
            code: ErrorCode::MethodNotFound as i32,
            message: format!(
                "workspace/executeCommand: unknown command `{other}`. Known commands: [{EVALUATE_JSONPATH_COMMAND}]"
            ),
            data: None,
        }),
    }
}

/// Parse + dispatch + evaluate a `tarn.evaluateJsonpath` command.
///
/// Pure apart from the `source` parameter, so unit tests can wire an
/// [`crate::code_actions::response_source::InMemoryResponseSource`]
/// and exercise every branch without touching disk.
pub fn execute_evaluate_jsonpath(
    args: &[Value],
    source: &dyn RecordedResponseSource,
) -> Result<EvaluationResult, ResponseError> {
    let parsed = parse_evaluate_args(args)?;
    let path = match &parsed {
        EvaluateArgs::Inline { path, .. } | EvaluateArgs::StepRef { path, .. } => path.clone(),
    };
    let response = resolve_response(&parsed, source)?;
    let matches = evaluate_path(&path, &response).map_err(|JsonPathError::Parse(msg)| {
        invalid_params(format!(
            "tarn.evaluateJsonpath: invalid JSONPath expression `{path}`: {msg}"
        ))
    })?;
    Ok(EvaluationResult { matches })
}

fn invalid_params(message: impl Into<String>) -> ResponseError {
    ResponseError {
        code: ErrorCode::InvalidParams as i32,
        message: message.into(),
        data: None,
    }
}

fn internal_error_from_serde(err: serde_json::Error) -> ResponseError {
    ResponseError {
        code: ErrorCode::InternalError as i32,
        message: format!("tarn.evaluateJsonpath: failed to serialise result: {err}"),
        data: None,
    }
}

/// Canonical absolute path used by tests — exposed so integration
/// tests and unit tests resolve against the same location.
#[doc(hidden)]
pub fn _test_file_path(name: &str) -> PathBuf {
    Path::new("/tmp/jsonpath-eval").join(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_actions::response_source::InMemoryResponseSource;
    use serde_json::json;

    #[test]
    fn parse_args_inline_happy_path() {
        let raw = vec![json!({"path": "$.x", "response": {"x": 1}})];
        let args = parse_evaluate_args(&raw).expect("parse ok");
        match args {
            EvaluateArgs::Inline { path, response } => {
                assert_eq!(path, "$.x");
                assert_eq!(response, json!({"x": 1}));
            }
            EvaluateArgs::StepRef { .. } => panic!("expected Inline"),
        }
    }

    #[test]
    fn parse_args_step_ref_happy_path() {
        let raw = vec![json!({
            "path": "$.id",
            "step": { "file": "/tmp/f.tarn.yaml", "test": "main", "step": "list" }
        })];
        let args = parse_evaluate_args(&raw).expect("parse ok");
        match args {
            EvaluateArgs::StepRef { path, step } => {
                assert_eq!(path, "$.id");
                assert_eq!(step.file, "/tmp/f.tarn.yaml");
                assert_eq!(step.test, "main");
                assert_eq!(step.step, "list");
            }
            EvaluateArgs::Inline { .. } => panic!("expected StepRef"),
        }
    }

    #[test]
    fn parse_args_missing_arg_returns_invalid_params() {
        let raw: Vec<Value> = Vec::new();
        let err = parse_evaluate_args(&raw).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams as i32);
        assert!(err.message.contains("one argument object"));
    }

    #[test]
    fn parse_args_malformed_object_returns_invalid_params() {
        let raw = vec![json!({"nonsense": true})];
        let err = parse_evaluate_args(&raw).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams as i32);
        assert!(err.message.contains("invalid argument shape"));
    }

    #[test]
    fn execute_inline_happy_path_returns_matches() {
        let source = InMemoryResponseSource::empty();
        let raw =
            vec![json!({"path": "$.items[*].id", "response": {"items": [{"id": 1}, {"id": 2}]}})];
        let result = execute_evaluate_jsonpath(&raw, &source).expect("ok");
        assert_eq!(result.matches, vec![json!(1), json!(2)]);
    }

    #[test]
    fn execute_inline_no_match_returns_empty_matches_not_error() {
        let source = InMemoryResponseSource::empty();
        let raw = vec![json!({"path": "$.missing", "response": {"present": 1}})];
        let result = execute_evaluate_jsonpath(&raw, &source).expect("ok");
        assert!(result.matches.is_empty());
    }

    #[test]
    fn execute_step_ref_happy_path_uses_in_memory_source() {
        let response = json!({"items": [{"id": 42}]});
        let source = InMemoryResponseSource::new(response);
        let raw = vec![json!({
            "path": "$.items[0].id",
            "step": { "file": "/tmp/any.tarn.yaml", "test": "main", "step": "list" }
        })];
        let result = execute_evaluate_jsonpath(&raw, &source).expect("ok");
        assert_eq!(result.matches, vec![json!(42)]);
    }

    #[test]
    fn execute_step_ref_missing_sidecar_returns_invalid_params() {
        let source = InMemoryResponseSource::empty();
        let raw = vec![json!({
            "path": "$.x",
            "step": { "file": "/tmp/any.tarn.yaml", "test": "main", "step": "list" }
        })];
        let err = execute_evaluate_jsonpath(&raw, &source).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams as i32);
        assert!(err.message.contains("no recorded response found"));
    }

    #[test]
    fn execute_inline_bad_jsonpath_returns_invalid_params() {
        let source = InMemoryResponseSource::empty();
        let raw = vec![json!({"path": "$.[not valid", "response": {}})];
        let err = execute_evaluate_jsonpath(&raw, &source).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams as i32);
        assert!(err.message.contains("invalid JSONPath expression"));
    }

    #[test]
    fn execute_inline_with_scalar_response_still_works() {
        let source = InMemoryResponseSource::empty();
        let raw = vec![json!({"path": "$", "response": 42})];
        let result = execute_evaluate_jsonpath(&raw, &source).expect("ok");
        assert_eq!(result.matches, vec![json!(42)]);
    }

    #[test]
    fn step_file_to_pathbuf_accepts_plain_path() {
        let p = step_file_to_pathbuf("/tmp/x.tarn.yaml");
        assert_eq!(p, PathBuf::from("/tmp/x.tarn.yaml"));
    }

    #[test]
    fn step_file_to_pathbuf_accepts_file_url() {
        let p = step_file_to_pathbuf("file:///tmp/x.tarn.yaml");
        assert_eq!(p, PathBuf::from("/tmp/x.tarn.yaml"));
    }

    #[test]
    fn evaluation_result_serialises_as_matches_envelope() {
        let r = EvaluationResult {
            matches: vec![json!("alpha"), json!(true)],
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v, json!({"matches": ["alpha", true]}));
    }

    #[test]
    fn workspace_execute_unknown_command_returns_method_not_found() {
        let state = ServerState::new();
        let params = ExecuteCommandParams {
            command: "tarn.unknownCommand".to_owned(),
            arguments: Vec::new(),
            work_done_progress_params: Default::default(),
        };
        let err = workspace_execute_command(&state, params).unwrap_err();
        assert_eq!(err.code, ErrorCode::MethodNotFound as i32);
        assert!(err.message.contains("tarn.unknownCommand"));
        assert!(err.message.contains(EVALUATE_JSONPATH_COMMAND));
    }
}
