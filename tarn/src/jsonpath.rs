//! JSONPath evaluation — thin public wrapper over `serde_json_path`
//! that [`crate::assert::body`] and [`crate::capture`] already use
//! internally.
//!
//! Shipped as part of L3.6 (NAZ-307) so the `tarn-lsp` hover provider
//! and `workspace/executeCommand` handler have one canonical library
//! entry point for JSONPath evaluation. The assertion and capture
//! modules continue to call into `serde_json_path` directly because
//! their public surface pre-dates this wrapper — the wrapper exists
//! so new call sites (hover, LSP commands, the upcoming VS Code
//! extension migration under Phase V) don't re-invent the parse /
//! error plumbing.
//!
//! ## Error taxonomy
//!
//! There is only one failure mode at library level: the path fails to
//! parse. "Zero matches" is **not** an error — an empty vector is the
//! correct answer for a path that was well-formed but matched nothing,
//! and the caller decides whether that counts as a failure in its own
//! context. This matches the behaviour of [`crate::assert::body`],
//! which produces a `<path not found>` assertion failure rather than
//! aborting the run.
//!
//! ```
//! use serde_json::json;
//! use tarn::jsonpath::evaluate_path;
//!
//! let value = json!({"items": [{"id": 1}, {"id": 2}]});
//! let matches = evaluate_path("$.items[*].id", &value).unwrap();
//! assert_eq!(matches, vec![json!(1), json!(2)]);
//! ```

use serde_json::Value;
use serde_json_path::JsonPath;
use thiserror::Error;

/// Failure modes for [`evaluate_path`].
///
/// Only `Parse` is emitted today. The enum is kept non-exhaustive in
/// spirit — future expansions (filter evaluation errors, overflow on
/// numeric conversions) can land without a source-breaking change as
/// long as callers handle `Parse` plus a wildcard fallback.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum JsonPathError {
    /// The JSONPath expression could not be parsed by `serde_json_path`.
    /// The string is the underlying library's error message (lossy but
    /// stable enough for LSP "InvalidParams" surfacing).
    #[error("JSONPath parse error: {0}")]
    Parse(String),
}

/// Evaluate a JSONPath expression against a JSON document and return
/// every match as owned [`Value`]s.
///
/// * An empty string (`""`) or any other unparseable expression
///   returns [`JsonPathError::Parse`]. The library never returns
///   `Ok(vec![])` for a parse failure — "zero matches" and "bad
///   path" are always distinguishable by the caller.
/// * A valid expression that matches nothing returns `Ok(vec![])`.
/// * A valid expression that matches exactly one node returns a
///   one-element vector — no flattening, no auto-unwrap, so callers
///   that need "single value or none" can check `matches.len()`
///   themselves.
/// * A valid expression that matches multiple nodes returns the
///   matches in document order, each cloned from the source document.
///
/// ## Why owned values
///
/// The LSP hover provider and `workspace/executeCommand` handler both
/// serialise the result straight to JSON, so borrowed references would
/// force every caller to either clone at the call site or hold the
/// source document alive across an async boundary. Cloning in one
/// place keeps the API simple at the cost of a few extra heap
/// allocations per hover — cheap relative to the YAML reparse the
/// hover handler already does.
pub fn evaluate_path(path: &str, value: &Value) -> Result<Vec<Value>, JsonPathError> {
    let parsed = JsonPath::parse(path).map_err(|e| JsonPathError::Parse(e.to_string()))?;
    let node_list = parsed.query(value);
    Ok(node_list.all().into_iter().cloned().collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn root_path_returns_whole_value() {
        let value = json!({"a": 1, "b": 2});
        let got = evaluate_path("$", &value).expect("root is always valid");
        assert_eq!(got, vec![value.clone()]);
    }

    #[test]
    fn simple_key_access_returns_matching_scalar() {
        let value = json!({"name": "alice"});
        let got = evaluate_path("$.name", &value).unwrap();
        assert_eq!(got, vec![json!("alice")]);
    }

    #[test]
    fn nested_key_access_returns_leaf_scalar() {
        let value = json!({"user": {"profile": {"email": "a@b.com"}}});
        let got = evaluate_path("$.user.profile.email", &value).unwrap();
        assert_eq!(got, vec![json!("a@b.com")]);
    }

    #[test]
    fn array_index_returns_single_element() {
        let value = json!({"items": [10, 20, 30]});
        let got = evaluate_path("$.items[1]", &value).unwrap();
        assert_eq!(got, vec![json!(20)]);
    }

    #[test]
    fn array_wildcard_returns_every_element_in_order() {
        let value = json!({"items": [1, 2, 3]});
        let got = evaluate_path("$.items[*]", &value).unwrap();
        assert_eq!(got, vec![json!(1), json!(2), json!(3)]);
    }

    #[test]
    fn projected_field_over_array_preserves_order() {
        let value = json!({"items": [{"id": 1}, {"id": 2}, {"id": 3}]});
        let got = evaluate_path("$.items[*].id", &value).unwrap();
        assert_eq!(got, vec![json!(1), json!(2), json!(3)]);
    }

    #[test]
    fn filter_expression_narrows_array() {
        let value = json!({"items": [{"id": 1, "ok": true}, {"id": 2, "ok": false}, {"id": 3, "ok": true}]});
        let got = evaluate_path("$.items[?(@.ok == true)].id", &value).unwrap();
        assert_eq!(got, vec![json!(1), json!(3)]);
    }

    #[test]
    fn not_found_returns_empty_vec_not_error() {
        let value = json!({"present": 1});
        let got = evaluate_path("$.missing", &value).unwrap();
        assert_eq!(got, Vec::<Value>::new());
    }

    #[test]
    fn invalid_path_returns_parse_error() {
        let value = json!({});
        let err = evaluate_path("$.[not valid json path]", &value).unwrap_err();
        assert!(
            matches!(err, JsonPathError::Parse(_)),
            "expected Parse error, got {err:?}"
        );
    }

    #[test]
    fn empty_string_path_returns_parse_error() {
        let value = json!({});
        let err = evaluate_path("", &value).unwrap_err();
        assert!(matches!(err, JsonPathError::Parse(_)));
    }

    #[test]
    fn non_root_anchored_path_returns_parse_error() {
        // `serde_json_path` enforces the `$` anchor; a bare key like
        // `.foo` is rejected at parse time.
        let value = json!({"foo": 1});
        let err = evaluate_path("foo", &value).unwrap_err();
        assert!(matches!(err, JsonPathError::Parse(_)));
    }

    #[test]
    fn preserves_json_types_across_matches() {
        let value = json!({"mix": [1, "two", true, null, 3.5]});
        let got = evaluate_path("$.mix[*]", &value).unwrap();
        assert_eq!(
            got,
            vec![json!(1), json!("two"), json!(true), json!(null), json!(3.5)]
        );
    }

    #[test]
    fn returned_values_are_owned_not_aliased() {
        let value = json!({"a": {"b": [1, 2]}});
        let got = evaluate_path("$.a.b", &value).unwrap();
        // Mutate via re-parse wouldn't affect returned clones.
        assert_eq!(got, vec![json!([1, 2])]);
        // Re-evaluating against a modified root picks up the change.
        let value2 = json!({"a": {"b": [99]}});
        let got2 = evaluate_path("$.a.b", &value2).unwrap();
        assert_eq!(got2, vec![json!([99])]);
        // Original result unchanged.
        assert_eq!(got, vec![json!([1, 2])]);
    }

    #[test]
    fn parse_error_message_is_not_empty() {
        let err = evaluate_path("$.[", &json!({})).unwrap_err();
        let JsonPathError::Parse(msg) = err;
        assert!(!msg.is_empty(), "Parse error message must be non-empty");
    }
}
