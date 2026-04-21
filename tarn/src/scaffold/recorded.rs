//! `tarn scaffold --from-recorded <path>` — rebuild a Tarn skeleton
//! from a previously-recorded fixture (NAZ-252 store under
//! `.tarn/fixtures/.../<N>/*.json`).
//!
//! Accepted inputs:
//! * a single JSON fixture file (e.g. `latest-passed.json` or
//!   `<millis>-<counter>.json`) in the unified [`Fixture`] shape —
//!   `{ recorded_at, request, response, captures, passed, ... }`.
//! * a step directory: we pick `latest-passed.json` when present,
//!   otherwise the newest history file per the on-disk `_index.json`.
//! * the legacy split form (`<dir>/request.json` +
//!   `<dir>/response.json`) — the ticket describes it and some older
//!   external callers may still produce it. We load both and
//!   synthesise a unified fixture.

use super::{BodyShape, ScaffoldRequest, Todo, TodoCategory};
use crate::error::TarnError;
use crate::fixtures::INDEX_FILENAME;
use crate::report::fixture_writer::{Fixture, FixtureRequest, FixtureResponse};
use std::collections::BTreeMap;
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

pub fn scaffold_from_recorded(input: &Path) -> Result<(ScaffoldRequest, Vec<Todo>), TarnError> {
    let fixture = load_fixture(input)?;
    let req = &fixture.request;
    let method = req.method.to_ascii_uppercase();
    let step_name = format!("{} {}", method, path_segment(&req.url));
    let file_name = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("recorded")
        .to_string();

    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for (k, v) in &req.headers {
        headers.insert(k.clone(), v.clone());
    }

    let body = req.body.as_ref().map(|v| {
        // Fixtures always store body as structured JSON; pass through.
        BodyShape::Json(v.clone())
    });

    let (captures, shape_keys, status_assertion) = derive_from_response(fixture.response.as_ref());

    let mut out = ScaffoldRequest::new(file_name, step_name);
    out.method = method;
    out.url = req.url.clone();
    out.headers = headers;
    out.body = body;
    out.captures = captures;
    out.response_shape_keys = shape_keys;
    out.status_assertion = status_assertion;

    // Flag common sensitive headers so the emitter attaches the Auth TODO.
    for name in ["Authorization", "Cookie", "X-Api-Key", "X-Auth-Token"] {
        if out.headers.keys().any(|k| k.eq_ignore_ascii_case(name)) {
            out.sensitive_headers.push(name.to_string());
        }
    }
    out.sensitive_headers.sort();
    out.sensitive_headers.dedup();

    let todos: Vec<Todo> = vec![Todo::new(
        TodoCategory::Body,
        "body is the literal recorded payload — replace identifiers and tokens with env/captures before rerunning",
    )];
    Ok((out, todos))
}

fn load_fixture(input: &Path) -> Result<Fixture, TarnError> {
    if input.is_file() {
        return load_single(input);
    }
    if !input.is_dir() {
        return Err(TarnError::Validation(format!(
            "tarn scaffold --from-recorded: {} does not exist",
            input.display()
        )));
    }
    // Directory mode — pick the best candidate.
    let latest = input.join("latest-passed.json");
    if latest.is_file() {
        return load_single(&latest);
    }
    // Try the rolling index first — it records chronological order
    // independent of filesystem mtimes.
    let index_path = input.join(INDEX_FILENAME);
    if index_path.is_file() {
        let content = std::fs::read_to_string(&index_path).map_err(to_val_err(&index_path))?;
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
            if let Some(arr) = parsed.get("history").and_then(|v| v.as_array()) {
                if let Some(last) = arr.iter().rev().find_map(|v| v.as_str()) {
                    return load_single(&input.join(last));
                }
            }
        }
    }
    // Legacy split form.
    let req_path = input.join("request.json");
    let resp_path = input.join("response.json");
    if req_path.is_file() {
        return load_split(
            &req_path,
            if resp_path.is_file() {
                Some(&resp_path)
            } else {
                None
            },
        );
    }

    Err(TarnError::Validation(format!(
        "tarn scaffold --from-recorded: no fixture found under {} — expected latest-passed.json, \
         a _index.json with history entries, or request.json",
        input.display()
    )))
}

fn load_single(path: &Path) -> Result<Fixture, TarnError> {
    let content = std::fs::read_to_string(path).map_err(to_val_err(path))?;
    serde_json::from_str::<Fixture>(&content).map_err(|e| {
        TarnError::Validation(format!(
            "tarn scaffold --from-recorded: failed to parse fixture {}: {e}",
            path.display()
        ))
    })
}

fn load_split(req_path: &Path, resp_path: Option<&Path>) -> Result<Fixture, TarnError> {
    let req_text = std::fs::read_to_string(req_path).map_err(to_val_err(req_path))?;
    let request: FixtureRequest = serde_json::from_str(&req_text).map_err(|e| {
        TarnError::Validation(format!(
            "tarn scaffold --from-recorded: failed to parse {}: {e}",
            req_path.display()
        ))
    })?;
    let response = if let Some(rp) = resp_path {
        let txt = std::fs::read_to_string(rp).map_err(to_val_err(rp))?;
        Some(serde_json::from_str::<FixtureResponse>(&txt).map_err(|e| {
            TarnError::Validation(format!(
                "tarn scaffold --from-recorded: failed to parse {}: {e}",
                rp.display()
            ))
        })?)
    } else {
        None
    };
    Ok(Fixture {
        // Empty recorded_at: the scaffold ignores it and emitting
        // something deterministic beats guessing.
        recorded_at: String::new(),
        request,
        response,
        captures: serde_json::Map::new(),
        passed: true,
        failure_message: None,
        duration_ms: 0,
    })
}

fn to_val_err(path: &Path) -> impl FnOnce(std::io::Error) -> TarnError + '_ {
    move |e| {
        TarnError::Validation(format!(
            "tarn scaffold --from-recorded: I/O error for {}: {e}",
            path.display()
        ))
    }
}

/// Given the recorded response, derive the scaffold captures, the
/// response-shape key list, and an exact-status assertion — all three
/// are far tighter than what the other modes can infer, which is the
/// whole point of recording before scaffolding.
fn derive_from_response(
    response: Option<&FixtureResponse>,
) -> (BTreeMap<String, String>, Vec<String>, Option<String>) {
    let Some(resp) = response else {
        return (BTreeMap::new(), Vec::new(), None);
    };
    let status = Some(resp.status.to_string());

    let mut keys: Vec<String> = Vec::new();
    let mut captures: BTreeMap<String, String> = BTreeMap::new();
    if let Some(serde_json::Value::Object(map)) = &resp.body {
        for k in map.keys() {
            keys.push(k.clone());
        }
        keys.sort();
        for k in &keys {
            let lower = k.to_ascii_lowercase();
            if matches!(lower.as_str(), "id" | "uuid" | "name" | "slug" | "token")
                || lower.ends_with("_id")
            {
                captures.insert(k.clone(), format!("$.{k}"));
            }
        }
    }
    (captures, keys, status)
}

fn path_segment(url: &str) -> String {
    if let Some(idx) = url.find("://") {
        let rest = &url[idx + 3..];
        if let Some(slash) = rest.find('/') {
            return rest[slash..].to_string();
        }
    }
    url.to_string()
}

/// Convenience for integration tests that want to build a fixture
/// file on disk.
#[cfg(test)]
pub(crate) fn write_fixture_for_test(dir: &Path, fixture: &Fixture) -> PathBuf {
    let p = dir.join("fixture.json");
    let serialised = serde_json::to_string_pretty(fixture).unwrap();
    std::fs::write(&p, serialised).unwrap();
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap as StdBTreeMap;

    fn sample_fixture() -> Fixture {
        let mut headers = StdBTreeMap::new();
        headers.insert("Authorization".to_string(), "Bearer abc".to_string());
        headers.insert("Content-Type".to_string(), "application/json".to_string());
        let mut body = serde_json::Map::new();
        body.insert("name".into(), serde_json::Value::String("Jane".into()));
        Fixture {
            recorded_at: "2024-01-01T00:00:00Z".to_string(),
            request: FixtureRequest {
                method: "POST".to_string(),
                url: "http://api/users".to_string(),
                headers,
                body: Some(serde_json::Value::Object(body)),
            },
            response: Some(FixtureResponse {
                status: 201,
                headers: StdBTreeMap::new(),
                body: Some(serde_json::json!({
                    "id": "u_123",
                    "name": "Jane",
                    "created_at": "now"
                })),
            }),
            captures: serde_json::Map::new(),
            passed: true,
            failure_message: None,
            duration_ms: 42,
        }
    }

    #[test]
    fn recorded_reconstructs_method_url_and_body() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture_for_test(dir.path(), &sample_fixture());
        let (req, _) = scaffold_from_recorded(&path).unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "http://api/users");
        match req.body {
            Some(BodyShape::Json(v)) => assert_eq!(v["name"], "Jane"),
            other => panic!("expected JSON body, got {:?}", other),
        }
        assert!(req
            .sensitive_headers
            .iter()
            .any(|h| h.eq_ignore_ascii_case("Authorization")));
    }

    #[test]
    fn recorded_derives_status_and_id_capture_from_response() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture_for_test(dir.path(), &sample_fixture());
        let (req, _) = scaffold_from_recorded(&path).unwrap();
        assert_eq!(req.status_assertion.as_deref(), Some("201"));
        assert_eq!(req.captures.get("id").map(String::as_str), Some("$.id"));
    }

    #[test]
    fn recorded_directory_prefers_latest_passed() {
        let dir = tempfile::tempdir().unwrap();
        let fixture = sample_fixture();
        let latest = dir.path().join("latest-passed.json");
        std::fs::write(&latest, serde_json::to_string_pretty(&fixture).unwrap()).unwrap();
        // Also drop a noisy history entry to prove we don't pick it.
        let history = dir.path().join("0001.json");
        let mut alt = fixture.clone();
        alt.request.url = "http://wrong".into();
        std::fs::write(&history, serde_json::to_string_pretty(&alt).unwrap()).unwrap();

        let (req, _) = scaffold_from_recorded(dir.path()).unwrap();
        assert_eq!(req.url, "http://api/users");
    }

    #[test]
    fn recorded_missing_path_is_validation_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = scaffold_from_recorded(&dir.path().join("nope.json")).unwrap_err();
        assert!(err.to_string().contains("does not exist"));
    }

    #[test]
    fn recorded_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_fixture_for_test(dir.path(), &sample_fixture());
        let a = scaffold_from_recorded(&path).unwrap().0;
        let b = scaffold_from_recorded(&path).unwrap().0;
        assert_eq!(a.headers, b.headers);
        assert_eq!(a.captures, b.captures);
        assert_eq!(a.response_shape_keys, b.response_shape_keys);
    }
}
