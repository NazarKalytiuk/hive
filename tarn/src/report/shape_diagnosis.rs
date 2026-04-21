//! Response-shape drift diagnosis (NAZ-415).
//!
//! Given an asserted or captured JSONPath that produced no match and the
//! JSON response body that was observed, produce a structured diagnosis
//! that agents can act on: the observed top-level shape, and a list of
//! candidate JSONPaths that *would* have matched, ranked by confidence.
//!
//! The heuristic is intentionally cheap and conservative — it prefers
//! zero "confidently wrong" candidates over many speculative ones. See
//! [`diagnose`] for the algorithm.
//!
//! This module is pure: it only touches `serde_json::Value` and does no
//! I/O, so it can be unit-tested exhaustively without any HTTP or
//! runner state.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_json_path::JsonPath;

/// Ranking for a candidate fix path. Higher is better.
///
/// * `High` — the tail-segment search found the expected leaf under an
///   observed top-level object at depth 1. Very unlikely to be wrong.
/// * `Medium` — the match required dropping an intermediate segment of
///   the expected path, or the leaf was found at depth 2. Directional
///   but worth verifying.
/// * `Low` — reserved for future heuristics. Not emitted today.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShapeConfidence {
    High,
    Medium,
    Low,
}

impl ShapeConfidence {
    fn rank(self) -> u8 {
        match self {
            ShapeConfidence::High => 2,
            ShapeConfidence::Medium => 1,
            ShapeConfidence::Low => 0,
        }
    }
}

/// A single suggested replacement JSONPath.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateFix {
    pub path: String,
    pub confidence: ShapeConfidence,
    pub reason: String,
}

/// Structured shape-drift hint attached to a failing step whose
/// JSONPath miss was diagnosed as response-shape drift.
///
/// * `expected_path` — the failing JSONPath, verbatim.
/// * `observed_keys` — top-level keys of the response body (only
///   populated when the body is a JSON object).
/// * `observed_type` — type of the response body: `object`, `array`,
///   `string`, `number`, `boolean`, or `null`.
/// * `candidate_fixes` — ranked replacement paths.
/// * `high_confidence` — true when at least one candidate is `High`.
///   Callers use this to decide between the generic failure category
///   and the drift-specific one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapeMismatchDiagnosis {
    pub expected_path: String,
    pub observed_keys: Vec<String>,
    pub observed_type: String,
    pub candidate_fixes: Vec<CandidateFix>,
    pub high_confidence: bool,
}

/// Maximum number of candidates we surface per failure. Five is
/// enough to cover the realistic "wrap under one of N top-level keys"
/// shapes without turning the artifact into a suggestion wall.
const MAX_CANDIDATES: usize = 5;

/// Run the drift heuristic for a failing JSONPath against the observed
/// body. Never returns `None`: the caller receives a diagnosis even for
/// non-object bodies (with an empty `candidate_fixes` and
/// `high_confidence: false`) so downstream reporting has a single
/// shape to render.
///
/// The heuristic only fires when the expected path is a simple dotted
/// walk (no filter expressions, descendant-`..`, or wildcards). Complex
/// paths report `observed_type`/`observed_keys` but no candidates —
/// they are out of scope for drift detection.
pub fn diagnose(expected_path: &str, observed: &Value) -> ShapeMismatchDiagnosis {
    let observed_type = value_type(observed).to_string();
    let observed_keys = top_level_keys(observed);

    let candidate_fixes = if is_simple_path(expected_path) {
        let segments = parse_segments(expected_path);
        build_candidates(&segments, observed)
    } else {
        Vec::new()
    };

    let high_confidence = candidate_fixes
        .iter()
        .any(|c| c.confidence == ShapeConfidence::High);

    ShapeMismatchDiagnosis {
        expected_path: expected_path.to_string(),
        observed_keys,
        observed_type,
        candidate_fixes,
        high_confidence,
    }
}

fn value_type(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn top_level_keys(value: &Value) -> Vec<String> {
    match value {
        Value::Object(map) => map.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

/// A path is "simple" if it's anchored at `$`, uses only dotted key
/// access and numeric array indices, and contains none of the
/// expressive JSONPath features (`..`, `*`, `[?…]`, unions, slices).
fn is_simple_path(path: &str) -> bool {
    if !path.starts_with('$') {
        return false;
    }
    if path.contains("..") || path.contains('*') || path.contains('?') {
        return false;
    }
    // Bracket contents must be pure digits (single numeric index).
    // Anything else (commas, colons, quoted strings) means we bail.
    let mut chars = path.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '[' {
            let mut inner = String::new();
            for d in chars.by_ref() {
                if d == ']' {
                    break;
                }
                inner.push(d);
            }
            if inner.is_empty() || !inner.chars().all(|c| c.is_ascii_digit()) {
                return false;
            }
        }
    }
    true
}

/// Split an expected path into logical segments *after* the `$` root.
/// Array indices stay attached to their key (`items[0]`) because that's
/// how we re-emit them into candidate suggestions — we never manipulate
/// numeric indices structurally.
fn parse_segments(path: &str) -> Vec<String> {
    let stripped = path.strip_prefix('$').unwrap_or(path);
    let stripped = stripped.trim_start_matches('.');
    if stripped.is_empty() {
        return Vec::new();
    }
    stripped.split('.').map(|s| s.to_string()).collect()
}

fn segments_to_path(segments: &[&str]) -> String {
    if segments.is_empty() {
        "$".to_string()
    } else {
        format!("$.{}", segments.join("."))
    }
}

/// Check whether a candidate JSONPath resolves to at least one node in
/// the observed body. Parse errors count as "no match" — a candidate we
/// couldn't even construct as valid JSONPath is worse than no candidate.
fn path_exists(candidate: &str, observed: &Value) -> bool {
    match JsonPath::parse(candidate) {
        Ok(jp) => !jp.query(observed).all().is_empty(),
        Err(_) => false,
    }
}

fn build_candidates(segments: &[String], observed: &Value) -> Vec<CandidateFix> {
    if segments.is_empty() {
        return Vec::new();
    }
    let Some(obj) = observed.as_object() else {
        return Vec::new();
    };

    // Collect candidates with their depth so we can sort deterministically.
    // The i32 score stores `(confidence_rank, -depth)`; we convert to
    // sort order at the end.
    let mut raw: Vec<(CandidateFix, u32)> = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();

    let full_suffix: Vec<&str> = segments.iter().map(String::as_str).collect();

    // (a) Depth-1 wrap: prefix the entire expected suffix with one
    // observed top-level object key. This is the "server wrapped the
    // response in an envelope" case — the classic `$.request.uuid`
    // drift.
    for (key, val) in obj {
        if !val.is_object() {
            continue;
        }
        let mut wrapped = vec![key.as_str()];
        wrapped.extend(full_suffix.iter().copied());
        let candidate = segments_to_path(&wrapped);
        if !seen_paths.insert(candidate.clone()) {
            continue;
        }
        if path_exists(&candidate, observed) {
            raw.push((
                CandidateFix {
                    path: candidate,
                    confidence: ShapeConfidence::High,
                    reason: format!(
                        "expected path is present under observed top-level key `{}`",
                        key
                    ),
                },
                1,
            ));
        }
    }

    // (b) Depth-2 wrap: two intermediate keys. Only fires when depth-1
    // found nothing — real APIs rarely nest two layers deep for the
    // same primary resource, so being conservative keeps the signal
    // clean.
    if raw.is_empty() {
        for (key1, val1) in obj {
            let Some(inner) = val1.as_object() else {
                continue;
            };
            for (key2, val2) in inner {
                if !val2.is_object() {
                    continue;
                }
                let mut wrapped = vec![key1.as_str(), key2.as_str()];
                wrapped.extend(full_suffix.iter().copied());
                let candidate = segments_to_path(&wrapped);
                if !seen_paths.insert(candidate.clone()) {
                    continue;
                }
                if path_exists(&candidate, observed) {
                    raw.push((
                        CandidateFix {
                            path: candidate,
                            confidence: ShapeConfidence::Medium,
                            reason: format!(
                                "expected path is present under observed keys `{}.{}`",
                                key1, key2
                            ),
                        },
                        2,
                    ));
                }
            }
        }
    }

    // (c) Prefix-drop: the expected path was over-specified — drop one
    // or more leading segments and see whether the tail exists against
    // the observed root. This catches `$.data.items[0].id` when the
    // server now returns `{items: [{id: ...}]}` at the root.
    for drop_count in 1..segments.len() {
        let tail: Vec<&str> = segments[drop_count..].iter().map(String::as_str).collect();
        if tail.is_empty() {
            continue;
        }
        let candidate = segments_to_path(&tail);
        if !seen_paths.insert(candidate.clone()) {
            continue;
        }
        if path_exists(&candidate, observed) {
            let dropped: Vec<&str> = segments[..drop_count].iter().map(String::as_str).collect();
            raw.push((
                CandidateFix {
                    path: candidate,
                    confidence: ShapeConfidence::Medium,
                    reason: format!(
                        "observed body matches path when prefix `{}` is dropped",
                        dropped.join(".")
                    ),
                },
                // Sort prefix-drops after wraps of the same confidence
                // by giving them a larger effective depth.
                (drop_count as u32) + 10,
            ));
        }
    }

    // Sort by confidence desc, then depth asc, then path asc for
    // stability in the artifact.
    raw.sort_by(|a, b| {
        b.0.confidence
            .rank()
            .cmp(&a.0.confidence.rank())
            .then(a.1.cmp(&b.1))
            .then(a.0.path.cmp(&b.0.path))
    });

    raw.into_iter()
        .map(|(c, _)| c)
        .take(MAX_CANDIDATES)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn leaf_under_top_level_object_is_high_confidence() {
        let body = json!({"request": {"uuid": "abc"}, "stageStatus": "pending"});
        let d = diagnose("$.uuid", &body);
        assert_eq!(d.observed_type, "object");
        assert!(d.high_confidence);
        let top = &d.candidate_fixes[0];
        assert_eq!(top.path, "$.request.uuid");
        assert_eq!(top.confidence, ShapeConfidence::High);
        assert!(d.observed_keys.contains(&"request".to_string()));
        assert!(d.observed_keys.contains(&"stageStatus".to_string()));
    }

    #[test]
    fn no_match_anywhere_returns_no_candidates() {
        let body = json!({"foo": 1, "bar": "baz"});
        let d = diagnose("$.uuid", &body);
        assert!(d.candidate_fixes.is_empty());
        assert!(!d.high_confidence);
    }

    #[test]
    fn prefix_drop_when_parent_segment_is_missing() {
        let body = json!({"items": [{"id": "x"}]});
        let d = diagnose("$.data.items[0].id", &body);
        assert!(!d.candidate_fixes.is_empty());
        let top = &d.candidate_fixes[0];
        assert_eq!(top.path, "$.items[0].id");
        assert_eq!(top.confidence, ShapeConfidence::Medium);
    }

    #[test]
    fn depth_two_wrap_is_medium_confidence() {
        let body = json!({"envelope": {"data": {"uuid": "u"}}});
        let d = diagnose("$.uuid", &body);
        assert!(!d.candidate_fixes.is_empty());
        let top = &d.candidate_fixes[0];
        assert_eq!(top.path, "$.envelope.data.uuid");
        assert_eq!(top.confidence, ShapeConfidence::Medium);
    }

    #[test]
    fn array_observed_body_yields_no_candidates() {
        let body = json!([{"uuid": "x"}]);
        let d = diagnose("$.uuid", &body);
        assert_eq!(d.observed_type, "array");
        assert!(d.observed_keys.is_empty());
        assert!(d.candidate_fixes.is_empty());
    }

    #[test]
    fn scalar_observed_body_yields_no_candidates() {
        let body = json!("hello");
        let d = diagnose("$.uuid", &body);
        assert_eq!(d.observed_type, "string");
        assert!(d.candidate_fixes.is_empty());
    }

    #[test]
    fn null_body_reports_null_type_and_no_candidates() {
        let body = json!(null);
        let d = diagnose("$.uuid", &body);
        assert_eq!(d.observed_type, "null");
        assert!(d.candidate_fixes.is_empty());
        assert!(!d.high_confidence);
    }

    #[test]
    fn filter_expression_paths_are_out_of_scope() {
        let body = json!({"items": [{"id": 1, "ok": true}]});
        let d = diagnose("$.items[?(@.ok)].id", &body);
        // Shape of the body is still reported…
        assert_eq!(d.observed_type, "object");
        assert!(d.observed_keys.contains(&"items".to_string()));
        // …but we do not synthesize candidates for expressive paths.
        assert!(d.candidate_fixes.is_empty());
    }

    #[test]
    fn candidates_are_capped_at_five() {
        let mut pairs = serde_json::Map::new();
        // Six separate top-level envelopes that each contain `uuid`.
        for i in 0..6 {
            pairs.insert(format!("env{}", i), json!({ "uuid": "x" }));
        }
        let body = Value::Object(pairs);
        let d = diagnose("$.uuid", &body);
        assert_eq!(d.candidate_fixes.len(), MAX_CANDIDATES);
        // All surviving candidates stay high confidence.
        assert!(d
            .candidate_fixes
            .iter()
            .all(|c| c.confidence == ShapeConfidence::High));
    }

    #[test]
    fn candidate_reason_names_the_wrapping_key() {
        let body = json!({"request": {"uuid": "x"}});
        let d = diagnose("$.uuid", &body);
        let top = &d.candidate_fixes[0];
        assert!(
            top.reason.contains("request"),
            "reason should name the wrapping key, got: {}",
            top.reason
        );
    }

    #[test]
    fn path_that_does_not_start_with_dollar_is_not_simple() {
        assert!(!is_simple_path("foo.bar"));
        assert!(!is_simple_path(".uuid"));
    }

    #[test]
    fn bracket_filters_mark_path_as_non_simple() {
        assert!(!is_simple_path("$.items[?(@.ok)]"));
        assert!(!is_simple_path("$..uuid"));
        assert!(!is_simple_path("$.items[*]"));
        assert!(is_simple_path("$.items[0].id"));
        assert!(is_simple_path("$.uuid"));
    }
}
