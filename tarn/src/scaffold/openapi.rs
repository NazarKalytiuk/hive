//! `tarn scaffold --from-openapi <spec> --op-id <id>` — walk a
//! minimal OpenAPI 3.x document and produce a Tarn skeleton.
//!
//! We use `serde_json::Value` as the AST rather than pulling an
//! OpenAPI crate: the fields we need (paths, path params, request
//! body required fields, response schema top-level keys) are a tiny
//! subset and stability of the top-level shape matters more than
//! exhaustive spec support. If the spec uses `$ref`, we dereference
//! one level inline — good enough for the shape checks we do and
//! far shorter than implementing full JSON Reference resolution.

use super::{BodyShape, ScaffoldRequest, Todo, TodoCategory};
use crate::error::TarnError;
use std::collections::BTreeMap;
use std::path::Path;

pub fn scaffold_from_openapi(
    spec_path: &Path,
    op_id: &str,
) -> Result<(ScaffoldRequest, Vec<Todo>), TarnError> {
    let content = std::fs::read_to_string(spec_path).map_err(|e| {
        TarnError::Validation(format!(
            "tarn scaffold --from-openapi could not read {}: {e}",
            spec_path.display()
        ))
    })?;

    // Try JSON first (cheap if the file is already JSON), fall back
    // to YAML → serde_yaml::Value → serde_json::Value via round-trip
    // serialization. Using the serde_yaml/serde_json bridge keeps us
    // one parser.
    let spec: serde_json::Value = serde_json::from_str(&content).or_else(|_| {
        let y: serde_yaml::Value = serde_yaml::from_str(&content).map_err(|e| {
            TarnError::Validation(format!(
                "tarn scaffold --from-openapi failed to parse {}: {e}",
                spec_path.display()
            ))
        })?;
        serde_json::to_value(y).map_err(|e| {
            TarnError::Validation(format!(
                "tarn scaffold --from-openapi could not convert YAML to JSON for {}: {e}",
                spec_path.display()
            ))
        })
    })?;

    let (path, verb, op) = locate_operation(&spec, op_id)
        .ok_or_else(|| TarnError::Validation(format!("operation '{op_id}' not found in spec")))?;

    let method = verb.to_ascii_uppercase();
    let (templated_url, path_params) = template_path_params(&path);
    let full_url = format!("{{{{ env.base_url }}}}{templated_url}");

    // Headers: seed Content-Type when the operation declares a JSON
    // request body. We avoid other security headers here — they're
    // user policy, not spec data.
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    let body = op
        .get("requestBody")
        .and_then(|rb| rb.get("content"))
        .and_then(|c| c.as_object())
        .and_then(|media_types| {
            // Prefer `application/json`, fall back to the first entry.
            media_types
                .get("application/json")
                .or_else(|| media_types.values().next())
                .map(|mt| {
                    (
                        media_types.contains_key("application/json"),
                        mt.get("schema").cloned(),
                    )
                })
        })
        .and_then(|(is_json, schema)| {
            if is_json {
                headers.insert("Content-Type".into(), "application/json".into());
            }
            schema.and_then(|s| minimal_example_from_schema(&s, &spec))
        });
    let body_shape = body.map(BodyShape::Json);

    // Response captures + shape keys.
    let (captures, shape_keys) = infer_response_metadata(op, &spec);

    // Step + file names.
    let path_summary = summary_path(&templated_url);
    let step_name = format!("{method} {path_summary}");
    let file_name = op_id.to_string();

    let mut request = ScaffoldRequest::new(file_name, step_name);
    request.method = method;
    request.url = full_url;
    request.headers = headers;
    request.body = body_shape;
    request.captures = captures;
    request.path_params = path_params;
    request.response_shape_keys = shape_keys;

    let todos: Vec<Todo> = vec![Todo::new(
        TodoCategory::Body,
        "body was synthesized from the OpenAPI schema's `required` keys — fill in realistic values",
    )];
    Ok((request, todos))
}

/// Walk `paths:` and return the first `(path, verb, operation)` whose
/// `operationId` matches `op_id`. We iterate in insertion order so
/// determinism holds even when the spec has duplicate ids (we pick
/// the first, matching OpenAPI's "operationIds MUST be unique"
/// guidance — pointless to disambiguate in the happy case).
fn locate_operation<'a>(
    spec: &'a serde_json::Value,
    op_id: &str,
) -> Option<(String, String, &'a serde_json::Value)> {
    let paths = spec.get("paths")?.as_object()?;
    for (path, path_item) in paths {
        let Some(map) = path_item.as_object() else {
            continue;
        };
        for verb in &[
            "get", "put", "post", "delete", "options", "head", "patch", "trace",
        ] {
            if let Some(op) = map.get(*verb) {
                if op
                    .get("operationId")
                    .and_then(|v| v.as_str())
                    .map(|id| id == op_id)
                    .unwrap_or(false)
                {
                    return Some((path.clone(), (*verb).to_string(), op));
                }
            }
        }
    }
    None
}

/// Rewrite OpenAPI-style `{id}` segments into Tarn `{{ test.id }}`
/// templates and return the parameter names in the order they appear.
/// We use the `test.` scope rather than `env.` so different tests in
/// the same file can bind different values without stomping the
/// project-wide env, matching the convention in the ticket brief.
fn template_path_params(path: &str) -> (String, Vec<String>) {
    let mut out = String::with_capacity(path.len());
    let mut params = Vec::new();
    let mut rest = path;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('}') {
            let name = &after[..end];
            if !name.is_empty() {
                out.push_str(&format!("{{{{ test.{name} }}}}"));
                params.push(name.to_string());
            } else {
                out.push_str("{}");
            }
            rest = &after[end + 1..];
        } else {
            // Unterminated — keep literal so we don't silently drop chars.
            out.push_str(&rest[start..]);
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    (out, params)
}

/// Dereference `{ "$ref": "#/components/schemas/Foo" }` in-place one
/// level. Returns the resolved value or the original on lookup
/// failure. Only local refs (`#/...`) are supported; remote refs
/// remain as-is (the scaffold just emits them as TODOs elsewhere).
fn deref_local<'a>(
    value: &'a serde_json::Value,
    spec: &'a serde_json::Value,
) -> &'a serde_json::Value {
    let Some(reference) = value.get("$ref").and_then(|v| v.as_str()) else {
        return value;
    };
    let Some(rest) = reference.strip_prefix("#/") else {
        return value;
    };
    let mut cur = spec;
    for seg in rest.split('/') {
        let Some(next) = cur.get(seg) else {
            return value;
        };
        cur = next;
    }
    cur
}

/// Build a minimal example from a JSON Schema fragment. Only the
/// `required` fields are emitted; their values default to `null`
/// (scaffold-quality, the user will overwrite). Unknown shapes
/// return `None` so the caller can decide to skip the body entirely.
fn minimal_example_from_schema(
    schema: &serde_json::Value,
    spec: &serde_json::Value,
) -> Option<serde_json::Value> {
    let resolved = deref_local(schema, spec);
    let ty = resolved.get("type").and_then(|v| v.as_str());
    match ty {
        Some("object") | None => {
            let required: Vec<String> = resolved
                .get("required")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let props = resolved.get("properties").and_then(|v| v.as_object());
            let mut out = serde_json::Map::new();
            // Sort required so output is deterministic regardless of
            // input order.
            let mut required_sorted = required.clone();
            required_sorted.sort();
            for key in &required_sorted {
                let prop_schema = props
                    .and_then(|p| p.get(key))
                    .map(|v| deref_local(v, spec))
                    .cloned();
                let value = prop_schema
                    .as_ref()
                    .and_then(schema_default_value)
                    .unwrap_or(serde_json::Value::Null);
                out.insert(key.clone(), value);
            }
            if out.is_empty() {
                // No required fields — nothing useful to seed. Return
                // an explicit empty object so the emitter still renders
                // `body: {}` and the user gets the scaffold hint.
                Some(serde_json::Value::Object(out))
            } else {
                Some(serde_json::Value::Object(out))
            }
        }
        Some("array") => Some(serde_json::Value::Array(Vec::new())),
        _ => schema_default_value(resolved),
    }
}

/// Pick a shape-matching default for a leaf schema. `null` is a
/// deliberate placeholder — the TODO already says "fill this in" and
/// a typed zero (`0`, `""`) would masquerade as a real value in
/// review tools.
fn schema_default_value(schema: &serde_json::Value) -> Option<serde_json::Value> {
    let ty = schema.get("type").and_then(|v| v.as_str())?;
    Some(match ty {
        "string" => serde_json::Value::Null,
        "integer" | "number" => serde_json::Value::Null,
        "boolean" => serde_json::Value::Null,
        "array" => serde_json::Value::Array(Vec::new()),
        "object" => serde_json::Value::Object(serde_json::Map::new()),
        _ => serde_json::Value::Null,
    })
}

/// Inspect the operation's successful response (200/201 preferred,
/// first 2xx fallback) and return (capture map, shape keys). Capture
/// names mirror the top-level id-shaped fields; shape keys list
/// every top-level property name for the `--format json` metadata.
fn infer_response_metadata(
    op: &serde_json::Value,
    spec: &serde_json::Value,
) -> (BTreeMap<String, String>, Vec<String>) {
    let Some(responses) = op.get("responses").and_then(|v| v.as_object()) else {
        return (BTreeMap::new(), Vec::new());
    };
    // Prefer 201, then 200, then first 2xx.
    let preferred_keys = ["201", "200"];
    let chosen = preferred_keys
        .iter()
        .find_map(|k| responses.get(*k))
        .or_else(|| {
            responses
                .iter()
                .find(|(k, _)| k.starts_with('2'))
                .map(|(_, v)| v)
        });
    let Some(resp) = chosen else {
        return (BTreeMap::new(), Vec::new());
    };
    let schema = resp
        .get("content")
        .and_then(|c| c.get("application/json"))
        .and_then(|mt| mt.get("schema"));
    let Some(schema) = schema else {
        return (BTreeMap::new(), Vec::new());
    };
    let resolved = deref_local(schema, spec);
    let props = resolved.get("properties").and_then(|v| v.as_object());
    let Some(props) = props else {
        return (BTreeMap::new(), Vec::new());
    };

    let mut keys: Vec<String> = props.keys().cloned().collect();
    keys.sort();
    let mut captures: BTreeMap<String, String> = BTreeMap::new();
    for k in &keys {
        let lower = k.to_ascii_lowercase();
        if matches!(lower.as_str(), "id" | "uuid" | "name" | "slug" | "token")
            || lower.ends_with("_id")
        {
            captures.insert(k.clone(), format!("$.{k}"));
        }
    }
    (captures, keys)
}

fn summary_path(templated: &str) -> String {
    // We currently use the raw templated path as-is; the function
    // exists as a central place to adjust naming conventions later
    // (strip leading `/`, collapse `/{{ test.id }}` → `/:id`, etc.)
    // without touching each call site.
    templated.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn write_temp_spec(json: &str) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(json.as_bytes()).unwrap();
        f.flush().unwrap();
        f
    }

    fn sample_spec() -> &'static str {
        r#"{
          "openapi": "3.0.0",
          "info": {"title": "T","version":"1"},
          "paths": {
            "/users/{id}": {
              "get": {
                "operationId": "getUser",
                "responses": {
                  "200": {
                    "content": {
                      "application/json": {
                        "schema": {
                          "type": "object",
                          "properties": {
                            "id": {"type": "string"},
                            "name": {"type": "string"},
                            "email": {"type": "string"}
                          }
                        }
                      }
                    }
                  }
                }
              }
            },
            "/users": {
              "post": {
                "operationId": "createUser",
                "requestBody": {
                  "content": {
                    "application/json": {
                      "schema": {
                        "type": "object",
                        "required": ["name", "email"],
                        "properties": {
                          "name": {"type": "string"},
                          "email": {"type": "string"}
                        }
                      }
                    }
                  }
                },
                "responses": {
                  "201": {
                    "content": {
                      "application/json": {
                        "schema": {
                          "type": "object",
                          "properties": {
                            "id": {"type": "string"},
                            "name": {"type": "string"}
                          }
                        }
                      }
                    }
                  }
                }
              }
            }
          }
        }"#
    }

    #[test]
    fn openapi_get_with_path_param_templates_id() {
        let f = write_temp_spec(sample_spec());
        let (req, _) = scaffold_from_openapi(f.path(), "getUser").unwrap();
        assert_eq!(req.method, "GET");
        assert!(req.url.contains("/users/{{ test.id }}"));
        assert_eq!(req.path_params, vec!["id".to_string()]);
        // Response captures should include `id` and `name`.
        assert_eq!(req.captures.get("id").map(String::as_str), Some("$.id"));
        assert_eq!(req.captures.get("name").map(String::as_str), Some("$.name"));
        // `email` is not id-shaped → no capture.
        assert!(!req.captures.contains_key("email"));
    }

    #[test]
    fn openapi_post_seeds_body_from_required_fields() {
        let f = write_temp_spec(sample_spec());
        let (req, _) = scaffold_from_openapi(f.path(), "createUser").unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(
            req.headers.get("Content-Type").map(String::as_str),
            Some("application/json")
        );
        match req.body {
            Some(BodyShape::Json(v)) => {
                let obj = v.as_object().unwrap();
                assert!(obj.contains_key("name"));
                assert!(obj.contains_key("email"));
            }
            other => panic!("expected structured body, got {:?}", other),
        }
    }

    #[test]
    fn openapi_unknown_op_id_is_validation_error() {
        let f = write_temp_spec(sample_spec());
        let err = scaffold_from_openapi(f.path(), "doesNotExist").unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn openapi_supports_yaml_input() {
        let yaml = r#"
openapi: 3.0.0
info:
  title: T
  version: '1'
paths:
  /health:
    get:
      operationId: getHealth
      responses:
        '200':
          content:
            application/json:
              schema:
                type: object
                properties:
                  status:
                    type: string
"#;
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(yaml.as_bytes()).unwrap();
        f.flush().unwrap();
        let (req, _) = scaffold_from_openapi(f.path(), "getHealth").unwrap();
        assert_eq!(req.method, "GET");
        assert!(req.url.ends_with("/health"));
    }

    #[test]
    fn openapi_is_deterministic() {
        let f = write_temp_spec(sample_spec());
        let a = scaffold_from_openapi(f.path(), "createUser").unwrap().0;
        let b = scaffold_from_openapi(f.path(), "createUser").unwrap().0;
        assert_eq!(a.headers, b.headers);
        assert_eq!(a.path_params, b.path_params);
        assert_eq!(a.response_shape_keys, b.response_shape_keys);
        if let (Some(BodyShape::Json(va)), Some(BodyShape::Json(vb))) = (a.body, b.body) {
            assert_eq!(va, vb);
        } else {
            panic!("both bodies should be JSON");
        }
    }
}
