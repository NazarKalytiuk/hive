//! `tarn.diffLastPassing` command handler (NAZ-256 Requirement C).
//!
//! Reads two sidecar response fixtures for a given `(file, test, step)`
//! triple and returns a structured diff describing the shift:
//!
//!   * `status` — `{ was, now }` tuple when the HTTP status changed.
//!   * `headers_added` — header names present in "now" but not in "was".
//!   * `headers_removed` — header names present in "was" but not in "now".
//!   * `headers_changed` — per-header `{ name, was, now }` entries when the value changed. Names are case-insensitive.
//!   * `body_keys_added` — JSON paths that are new in the "now" body.
//!   * `body_keys_removed` — JSON paths that were dropped.
//!   * `body_values_changed` — per-path `{ path, was, now }` entries.
//!
//! Fixture format (documented by the NAZ-252/254 agent):
//!
//! ```text
//! .tarn/fixtures/<hash>/<test>/<step>.json
//! .tarn/fixtures/<hash>/<test>/<step>.latest-passed.json
//! ```
//!
//! The handler compares the current (`<step>.json`) fixture to the
//! most recent passing fixture (`<step>.latest-passed.json`). When the
//! passing fixture does not exist, the handler returns
//! `{ error: "no_baseline", message: "..." }` so clients can surface a
//! UI hint instead of an actual diff.
//!
//! # Integration points
//!
//! The fixture path resolution is pure — tests supply an in-memory
//! [`FixtureSource`] that fakes the disk, just like
//! `code_actions::response_source` does for `tarn.evaluateJsonpath`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use lsp_server::{ErrorCode, ResponseError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::run_commands::parse_file_arg;

/// Stable LSP command id advertised in [`crate::capabilities`].
pub const DIFF_LAST_PASSING_COMMAND: &str = "tarn.diffLastPassing";

/// Arguments to `tarn.diffLastPassing`.
#[derive(Debug, Clone, Deserialize)]
pub struct DiffLastPassingArgs {
    /// Absolute path (or `file://` URI) to the `.tarn.yaml` buffer.
    pub file: String,
    /// Test group name (or the file's own `name:` for flat-step files).
    pub test: String,
    /// Zero-based step index within the test. Matches the index the
    /// code-lens uses.
    pub step: usize,
}

/// Abstraction over the fixture store so tests can feed synthetic data.
pub trait FixtureSource {
    /// Return the current (most-recent-run) fixture for a step.
    fn read_current(&self, file: &Path, test: &str, step: usize) -> Option<Value>;

    /// Return the most-recent-passing fixture for a step. `None` means
    /// the caller should surface a `no_baseline` error.
    fn read_last_passing(&self, file: &Path, test: &str, step: usize) -> Option<Value>;
}

/// Filesystem-backed [`FixtureSource`] used in production. Looks under
/// `<project_root>/.tarn/fixtures/<hash>/<test>/<step>.json` and
/// `<project_root>/.tarn/fixtures/<hash>/<test>/<step>.latest-passed.json`.
/// The `<hash>` is derived from the absolute path of the test file so
/// two files with the same name in different directories never collide.
#[derive(Debug, Clone, Default)]
pub struct DiskFixtureSource;

impl DiskFixtureSource {
    fn current_path(file: &Path, test: &str, step: usize) -> PathBuf {
        fixture_base(file, test).join(format!("{step}.json"))
    }

    fn last_passing_path(file: &Path, test: &str, step: usize) -> PathBuf {
        fixture_base(file, test).join(format!("{step}.latest-passed.json"))
    }
}

impl FixtureSource for DiskFixtureSource {
    fn read_current(&self, file: &Path, test: &str, step: usize) -> Option<Value> {
        let path = Self::current_path(file, test, step);
        read_json(&path).ok()
    }

    fn read_last_passing(&self, file: &Path, test: &str, step: usize) -> Option<Value> {
        let path = Self::last_passing_path(file, test, step);
        read_json(&path).ok()
    }
}

fn read_json(path: &Path) -> Result<Value, String> {
    let content = std::fs::read_to_string(path).map_err(|e| format!("read: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("parse: {e}"))
}

/// Build the `<project_root>/.tarn/fixtures/<hash>/<test>/` path that
/// sidecar fixtures live under. The hash is derived from the file's
/// absolute path so two files with the same basename in different
/// directories never share a fixture directory.
fn fixture_base(file: &Path, test: &str) -> PathBuf {
    // The NAZ-252 agent owns the exact hash shape. We use a short
    // deterministic hex derived from the canonicalised absolute path
    // so both agents produce the same layout even when the process
    // has different working directories.
    let canonical = file.canonicalize().unwrap_or_else(|_| file.to_path_buf());
    let hash = stable_hash(&canonical.display().to_string());
    let project_root = find_project_root(file).unwrap_or_else(|| {
        file.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."))
    });
    project_root
        .join(".tarn")
        .join("fixtures")
        .join(hash)
        .join(test)
}

/// Walk upward looking for the project root (marked by
/// `tarn.config.yaml` or `.tarn/`). Mirrors the CLI's
/// `config::find_project_root` behaviour so fixture paths line up with
/// every other `.tarn/` artifact.
fn find_project_root(start: &Path) -> Option<PathBuf> {
    let start = start
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let mut current: &Path = start.as_path();
    loop {
        if current.join("tarn.config.yaml").exists() || current.join(".tarn").is_dir() {
            return Some(current.to_path_buf());
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => break,
        }
    }
    None
}

/// FNV-1a 64-bit hash rendered as hex. Stable across runs and
/// platforms so two processes always produce the same fixture path.
fn stable_hash(input: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in input.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{:016x}", hash)
}

/// Outer envelope returned by the command: either a structured diff or
/// a `no_baseline` marker.
#[derive(Debug, Clone, Serialize)]
pub struct DiffEnvelope {
    pub schema_version: u32,
    pub data: Value,
}

/// Perform the actual JSON diff on two decoded response documents. The
/// responses are expected to follow the shape `{ "status": u16,
/// "headers": { k: v }, "body": <json> }`; fields may be missing — the
/// function degrades gracefully.
pub fn diff_responses(was: &Value, now: &Value) -> Value {
    let mut out = serde_json::Map::new();

    // Status
    let was_status = was.get("status").cloned();
    let now_status = now.get("status").cloned();
    if was_status != now_status {
        out.insert(
            "status".to_string(),
            json!({ "was": was_status.unwrap_or(Value::Null), "now": now_status.unwrap_or(Value::Null) }),
        );
    }

    // Headers (case-insensitive)
    let was_headers = headers_as_map(was.get("headers"));
    let now_headers = headers_as_map(now.get("headers"));
    let was_keys: BTreeSet<String> = was_headers.keys().cloned().collect();
    let now_keys: BTreeSet<String> = now_headers.keys().cloned().collect();
    let headers_added: Vec<String> = now_keys.difference(&was_keys).cloned().collect();
    let headers_removed: Vec<String> = was_keys.difference(&now_keys).cloned().collect();
    let mut headers_changed: Vec<Value> = Vec::new();
    for name in was_keys.intersection(&now_keys) {
        let w = was_headers.get(name);
        let n = now_headers.get(name);
        if w != n {
            headers_changed.push(json!({
                "name": name,
                "was": w.cloned().unwrap_or_default(),
                "now": n.cloned().unwrap_or_default(),
            }));
        }
    }
    if !headers_added.is_empty() {
        out.insert("headers_added".to_string(), json!(headers_added));
    }
    if !headers_removed.is_empty() {
        out.insert("headers_removed".to_string(), json!(headers_removed));
    }
    if !headers_changed.is_empty() {
        out.insert("headers_changed".to_string(), json!(headers_changed));
    }

    // Body (walk recursively).
    let body_was = was.get("body").cloned().unwrap_or(Value::Null);
    let body_now = now.get("body").cloned().unwrap_or(Value::Null);
    let mut body_added: Vec<String> = Vec::new();
    let mut body_removed: Vec<String> = Vec::new();
    let mut body_changed: Vec<Value> = Vec::new();
    walk_body(
        "$",
        &body_was,
        &body_now,
        &mut body_added,
        &mut body_removed,
        &mut body_changed,
    );
    if !body_added.is_empty() {
        out.insert("body_keys_added".to_string(), json!(body_added));
    }
    if !body_removed.is_empty() {
        out.insert("body_keys_removed".to_string(), json!(body_removed));
    }
    if !body_changed.is_empty() {
        out.insert("body_values_changed".to_string(), json!(body_changed));
    }

    Value::Object(out)
}

/// Normalise headers into a case-insensitive `BTreeMap<String, String>`.
/// Returns an empty map when the input is not a JSON object.
fn headers_as_map(value: Option<&Value>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(obj) = value.and_then(|v| v.as_object()) else {
        return out;
    };
    for (k, v) in obj {
        let key = k.to_ascii_lowercase();
        let val = match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        out.insert(key, val);
    }
    out
}

/// Recursive body walker. Records `added`/`removed` paths for keys that
/// appear on only one side and `changed` entries when the same path
/// carries different scalar (or structurally different) values.
fn walk_body(
    path: &str,
    was: &Value,
    now: &Value,
    added: &mut Vec<String>,
    removed: &mut Vec<String>,
    changed: &mut Vec<Value>,
) {
    match (was, now) {
        (Value::Object(a), Value::Object(b)) => {
            let was_keys: BTreeSet<&String> = a.keys().collect();
            let now_keys: BTreeSet<&String> = b.keys().collect();
            for k in now_keys.difference(&was_keys) {
                added.push(format!("{path}.{k}"));
            }
            for k in was_keys.difference(&now_keys) {
                removed.push(format!("{path}.{k}"));
            }
            for k in was_keys.intersection(&now_keys) {
                let child = format!("{path}.{k}");
                walk_body(
                    &child,
                    &a[k.as_str()],
                    &b[k.as_str()],
                    added,
                    removed,
                    changed,
                );
            }
        }
        (Value::Array(a), Value::Array(b)) => {
            let max_len = a.len().max(b.len());
            for i in 0..max_len {
                let child = format!("{path}[{i}]");
                match (a.get(i), b.get(i)) {
                    (Some(aw), Some(nw)) => {
                        walk_body(&child, aw, nw, added, removed, changed);
                    }
                    (None, Some(_)) => {
                        added.push(child);
                    }
                    (Some(_), None) => {
                        removed.push(child);
                    }
                    (None, None) => {}
                }
            }
        }
        (w, n) if w == n => {}
        (w, n) => {
            changed.push(json!({
                "path": path,
                "was": w,
                "now": n,
            }));
        }
    }
}

/// Execute `tarn.diffLastPassing`.
pub fn execute_diff_last_passing(
    args: &DiffLastPassingArgs,
    source: &dyn FixtureSource,
) -> Result<DiffEnvelope, ResponseError> {
    let file_path = parse_file_arg(&args.file)?;
    let was = match source.read_last_passing(&file_path, &args.test, args.step) {
        Some(v) => v,
        None => {
            return Ok(DiffEnvelope {
                schema_version: 1,
                data: json!({
                    "error": "no_baseline",
                    "message": "no passing run recorded for this step yet",
                }),
            });
        }
    };
    let now = source
        .read_current(&file_path, &args.test, args.step)
        .ok_or_else(|| {
            invalid_params(format!(
                "tarn.diffLastPassing: no current fixture for `{}::{}::{}` — run the step at least once",
                file_path.display(),
                args.test,
                args.step
            ))
        })?;
    let diff = diff_responses(&was, &now);
    Ok(DiffEnvelope {
        schema_version: 1,
        data: diff,
    })
}

/// Dispatcher hook for the server.
pub fn dispatch(command: &str, arguments: &[Value]) -> Option<Result<Value, ResponseError>> {
    if command != DIFF_LAST_PASSING_COMMAND {
        return None;
    }
    let arg = match arguments.first() {
        Some(v) => v.clone(),
        None => {
            return Some(Err(invalid_params(
                "tarn.diffLastPassing requires one argument object",
            )))
        }
    };
    let args: DiffLastPassingArgs = match serde_json::from_value(arg) {
        Ok(v) => v,
        Err(e) => return Some(Err(invalid_params(format!("tarn.diffLastPassing: {e}")))),
    };
    let source = DiskFixtureSource;
    Some(execute_diff_last_passing(&args, &source).and_then(|env| {
        serde_json::to_value(env).map_err(|e| ResponseError {
            code: ErrorCode::InternalError as i32,
            message: format!("serialize envelope: {e}"),
            data: None,
        })
    }))
}

fn invalid_params(msg: impl Into<String>) -> ResponseError {
    ResponseError {
        code: ErrorCode::InvalidParams as i32,
        message: msg.into(),
        data: None,
    }
}

/// In-memory fixture source for tests. `last_passing` may be `None` to
/// reproduce the "no baseline" path.
#[derive(Debug, Clone, Default)]
pub struct InMemoryFixtureSource {
    pub current: Option<Value>,
    pub last_passing: Option<Value>,
}

impl InMemoryFixtureSource {
    pub fn with(current: Value, last_passing: Option<Value>) -> Self {
        Self {
            current: Some(current),
            last_passing,
        }
    }
}

impl FixtureSource for InMemoryFixtureSource {
    fn read_current(&self, _file: &Path, _test: &str, _step: usize) -> Option<Value> {
        self.current.clone()
    }
    fn read_last_passing(&self, _file: &Path, _test: &str, _step: usize) -> Option<Value> {
        self.last_passing.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_status_shift() {
        let was = json!({ "status": 200 });
        let now = json!({ "status": 500 });
        let d = diff_responses(&was, &now);
        assert_eq!(d["status"], json!({ "was": 200, "now": 500 }));
    }

    #[test]
    fn diff_header_added_removed_changed() {
        let was = json!({
            "headers": { "Content-Type": "application/json", "X-Old": "1" }
        });
        let now = json!({
            "headers": { "content-type": "application/xml", "X-New": "2" }
        });
        let d = diff_responses(&was, &now);
        let added = d["headers_added"].as_array().unwrap();
        let removed = d["headers_removed"].as_array().unwrap();
        let changed = d["headers_changed"].as_array().unwrap();
        assert!(added.iter().any(|v| v == "x-new"));
        assert!(removed.iter().any(|v| v == "x-old"));
        assert!(changed.iter().any(|v| v["name"] == "content-type"));
    }

    #[test]
    fn diff_body_key_added_removed_changed() {
        let was = json!({ "body": { "user": { "id": 1, "name": "A" }, "drop": true }});
        let now = json!({ "body": { "user": { "id": 1, "name": "B" }, "new": 42 }});
        let d = diff_responses(&was, &now);
        assert_eq!(d["body_keys_added"], json!(["$.new"]));
        assert_eq!(d["body_keys_removed"], json!(["$.drop"]));
        let changed = d["body_values_changed"].as_array().unwrap();
        assert_eq!(changed[0]["path"], "$.user.name");
        assert_eq!(changed[0]["was"], "A");
        assert_eq!(changed[0]["now"], "B");
    }

    #[test]
    fn diff_unchanged_returns_empty_object() {
        let value = json!({ "status": 200, "headers": {}, "body": {"x": 1} });
        let d = diff_responses(&value, &value);
        let obj = d.as_object().unwrap();
        assert!(obj.is_empty());
    }

    #[test]
    fn execute_returns_no_baseline_when_fixture_missing() {
        let source = InMemoryFixtureSource {
            current: Some(json!({"status": 500})),
            last_passing: None,
        };
        let out = execute_diff_last_passing(
            &DiffLastPassingArgs {
                file: "/tmp/f.tarn.yaml".into(),
                test: "t".into(),
                step: 0,
            },
            &source,
        )
        .unwrap();
        assert_eq!(out.data["error"], "no_baseline");
    }

    #[test]
    fn execute_errors_when_current_missing() {
        let source = InMemoryFixtureSource {
            current: None,
            last_passing: Some(json!({"status": 200})),
        };
        let err = execute_diff_last_passing(
            &DiffLastPassingArgs {
                file: "/tmp/f.tarn.yaml".into(),
                test: "t".into(),
                step: 0,
            },
            &source,
        )
        .unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams as i32);
        assert!(err.message.contains("no current fixture"));
    }

    #[test]
    fn execute_produces_structured_diff_for_status_shift() {
        let source = InMemoryFixtureSource::with(
            json!({ "status": 500, "headers": {}, "body": {"error": "boom"} }),
            Some(json!({ "status": 200, "headers": {}, "body": {"ok": true} })),
        );
        let out = execute_diff_last_passing(
            &DiffLastPassingArgs {
                file: "/tmp/f.tarn.yaml".into(),
                test: "t".into(),
                step: 0,
            },
            &source,
        )
        .unwrap();
        assert_eq!(out.data["status"], json!({ "was": 200, "now": 500 }));
        assert_eq!(out.data["body_keys_added"], json!(["$.error"]));
        assert_eq!(out.data["body_keys_removed"], json!(["$.ok"]));
    }

    #[test]
    fn dispatch_returns_none_for_other_commands() {
        let res = dispatch("tarn.other", &[json!({})]);
        assert!(res.is_none());
    }

    #[test]
    fn dispatch_requires_arguments() {
        let res = dispatch(DIFF_LAST_PASSING_COMMAND, &[]);
        let err = res.unwrap().unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidParams as i32);
    }

    #[test]
    fn stable_hash_is_deterministic() {
        assert_eq!(stable_hash("abc"), stable_hash("abc"));
        assert_ne!(stable_hash("abc"), stable_hash("abd"));
    }
}
