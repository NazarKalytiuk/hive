//! `tarn scaffold --method <M> --url <U>` — the minimal-input mode.
//!
//! We intentionally infer nothing beyond "POST/PUT/PATCH probably
//! want a JSON body placeholder" — anything more would cross the
//! non-goal line ("magical inference of business assertions from
//! thin input"). The scaffold's value here is consistency: the user
//! writes the same YAML shape every time, with a TODO per field that
//! needs a human decision.

use super::{BodyShape, ScaffoldRequest, Todo};
use crate::error::TarnError;
use std::collections::BTreeMap;

pub fn scaffold_from_explicit(
    method: &str,
    url: &str,
) -> Result<(ScaffoldRequest, Vec<Todo>), TarnError> {
    let method_upper = method.trim().to_ascii_uppercase();
    if method_upper.is_empty() {
        return Err(TarnError::Validation(
            "tarn scaffold --method requires a non-empty value".into(),
        ));
    }
    if !method_upper
        .chars()
        .all(|c| c.is_ascii_alphabetic() || c == '-' || c == '_')
    {
        return Err(TarnError::Validation(format!(
            "tarn scaffold --method received '{}' which is not a valid HTTP method",
            method
        )));
    }
    let trimmed_url = url.trim();
    if trimmed_url.is_empty() {
        return Err(TarnError::Validation(
            "tarn scaffold --url requires a non-empty value".into(),
        ));
    }

    let step_name = format!("{} {}", method_upper, path_or_url(trimmed_url));
    let file_name = default_file_name(&method_upper, trimmed_url);

    let mut request = ScaffoldRequest::new(file_name, step_name);
    request.method = method_upper.clone();
    request.url = trimmed_url.to_string();

    // For mutating requests, pre-seed Content-Type: application/json
    // and an empty body placeholder so the scaffold can be filled in
    // without the user remembering to add the header.
    let mutating = matches!(method_upper.as_str(), "POST" | "PUT" | "PATCH" | "DELETE");
    if mutating && !matches!(method_upper.as_str(), "DELETE") {
        let mut headers = BTreeMap::new();
        headers.insert("Content-Type".into(), "application/json".into());
        request.headers = headers;
        let mut placeholder = serde_json::Map::new();
        placeholder.insert("field".into(), serde_json::Value::Null);
        request.body = Some(BodyShape::Json(serde_json::Value::Object(placeholder)));
    }

    Ok((request, Vec::new()))
}

fn path_or_url(url: &str) -> String {
    if let Some(idx) = url.find("://") {
        let rest = &url[idx + 3..];
        if let Some(slash) = rest.find('/') {
            let path = &rest[slash..];
            if !path.is_empty() {
                return path.to_string();
            }
        }
    }
    url.to_string()
}

fn default_file_name(method: &str, url: &str) -> String {
    format!("{} {}", method, path_or_url(url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_post_seeds_content_type_and_body_placeholder() {
        let (req, _) = scaffold_from_explicit("POST", "http://example.com/users").unwrap();
        assert_eq!(req.method, "POST");
        assert_eq!(req.url, "http://example.com/users");
        assert_eq!(
            req.headers.get("Content-Type").map(String::as_str),
            Some("application/json")
        );
        assert!(matches!(req.body, Some(BodyShape::Json(_))));
    }

    #[test]
    fn explicit_get_has_no_body_or_content_type() {
        let (req, _) = scaffold_from_explicit("GET", "http://example.com/health").unwrap();
        assert_eq!(req.method, "GET");
        assert!(req.headers.is_empty());
        assert!(req.body.is_none());
    }

    #[test]
    fn explicit_delete_has_no_body_but_no_content_type() {
        let (req, _) = scaffold_from_explicit("DELETE", "http://example.com/users/1").unwrap();
        assert_eq!(req.method, "DELETE");
        // DELETE bodies are legal but atypical — scaffold stays quiet.
        assert!(req.body.is_none());
        assert!(req.headers.is_empty());
    }

    #[test]
    fn explicit_normalizes_method_case() {
        let (req, _) = scaffold_from_explicit("post", "http://example.com/x").unwrap();
        assert_eq!(req.method, "POST");
    }

    #[test]
    fn explicit_rejects_empty_method_and_url() {
        assert!(scaffold_from_explicit("", "http://x/y").is_err());
        assert!(scaffold_from_explicit("GET", "   ").is_err());
    }

    #[test]
    fn explicit_rejects_garbage_method() {
        assert!(scaffold_from_explicit("GET /users", "http://x/y").is_err());
    }

    #[test]
    fn explicit_step_name_prefers_path_over_full_url() {
        let (req, _) = scaffold_from_explicit("GET", "http://example.com/api/v1/widgets").unwrap();
        assert_eq!(req.step_name, "GET /api/v1/widgets");
    }
}
