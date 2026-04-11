//! JSONPath leaf-name derivation and JSON type inference helpers
//! (NAZ-304, Phase L3.3).
//!
//! Shared between the **capture-this-field** and **scaffold-assert**
//! code actions. Both providers need to turn a JSONPath literal into
//! a valid Tarn identifier (for a `capture:` key name) and to map
//! JSON runtime types onto Tarn's `type:` assertion vocabulary.
//!
//! The helpers are deliberately pure and free of any LSP type so the
//! unit tests can exercise every branch without constructing a fake
//! `CodeActionContext`.
//!
//! ## Leaf-name rules
//!
//! The leaf name is derived from the **last non-wildcard segment** of
//! a JSONPath. Segments may be:
//!
//!   * `.identifier` (dot notation, ASCII letters + digits + `_`)
//!   * `["quoted key"]` (bracket notation with double or single quotes)
//!   * `[N]` (integer index — attached to the preceding key as
//!     `key_N`, or reported as `index_N` when the path is only an
//!     index)
//!   * `[*]` or `.*` (wildcard — inherits the previous key; if no
//!     previous key, falls back to `field`)
//!
//! Non-identifier characters (hyphen, space, dot inside quotes) are
//! replaced with `_`. A leading digit after sanitisation is prefixed
//! with `_`. The result is always a valid Tarn identifier in the
//! sense of [`crate::identifier::is_valid_identifier`]; an empty or
//! all-garbage input yields `field`.

use crate::identifier::is_valid_identifier;

/// Derive a capture identifier name from a JSONPath literal.
///
/// See the module documentation for the full rule set. Always returns
/// a non-empty string that passes [`is_valid_identifier`]; the catch-
/// all fallback is `"field"`.
pub fn leaf_name(path: &str) -> String {
    let segments = parse_segments(path);
    if segments.is_empty() {
        return "field".to_owned();
    }

    // Walk from the tail backwards. The last *named* segment wins; if
    // the very last segment is an index and there is a named segment
    // before it, we combine them as `<key>_<index>`. A pure-wildcard
    // tail inherits the previous named segment. A pure-index path
    // becomes `index_N`.
    let mut last_key: Option<String> = None;
    let mut trailing_index: Option<String> = None;

    for seg in &segments {
        match seg {
            Segment::Key(k) => {
                if trailing_index.is_some() {
                    // A new key after an index resets the pair.
                    trailing_index = None;
                }
                last_key = Some(k.clone());
            }
            Segment::Index(i) => {
                trailing_index = Some(i.clone());
            }
            Segment::Wildcard => {
                // Wildcard inherits whatever came before it. Do not
                // overwrite `last_key` — it is exactly the behaviour
                // we want for `$.tags[*]` → `tags`.
            }
        }
    }

    let raw = match (last_key, trailing_index) {
        (Some(k), Some(i)) => format!("{k}_{i}"),
        (Some(k), None) => k,
        (None, Some(i)) => format!("index_{i}"),
        (None, None) => "field".to_owned(),
    };

    sanitize(&raw)
}

/// Map a JSON value onto the Tarn `type:` assertion vocabulary.
///
/// Covers the six primitives Tarn's `assert.body` accepts:
/// `number`, `string`, `boolean`, `array`, `object`, `null`. Integers
/// and floats both fold to `number` — Tarn does not distinguish them
/// in assertion land.
pub fn infer_type(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

// --- internal parser ---------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Key(String),
    Index(String),
    Wildcard,
}

fn parse_segments(path: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let bytes = path.as_bytes();
    let mut i = 0usize;

    // Skip the leading `$`; callers pass `$.data[0]` etc.
    if i < bytes.len() && bytes[i] == b'$' {
        i += 1;
    }

    while i < bytes.len() {
        match bytes[i] {
            b'.' => {
                i += 1;
                if i < bytes.len() && bytes[i] == b'*' {
                    out.push(Segment::Wildcard);
                    i += 1;
                    continue;
                }
                // Read an identifier-ish run.
                let start = i;
                while i < bytes.len() && !matches!(bytes[i], b'.' | b'[' | b' ') {
                    i += 1;
                }
                if start == i {
                    continue;
                }
                let key = path[start..i].to_owned();
                out.push(Segment::Key(key));
            }
            b'[' => {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                match bytes[i] {
                    b'*' => {
                        out.push(Segment::Wildcard);
                        // Skip until closing `]`.
                        while i < bytes.len() && bytes[i] != b']' {
                            i += 1;
                        }
                        if i < bytes.len() {
                            i += 1;
                        }
                    }
                    b'"' | b'\'' => {
                        let quote = bytes[i];
                        i += 1;
                        let start = i;
                        while i < bytes.len() && bytes[i] != quote {
                            i += 1;
                        }
                        let key = path[start..i].to_owned();
                        out.push(Segment::Key(key));
                        if i < bytes.len() {
                            i += 1; // closing quote
                        }
                        while i < bytes.len() && bytes[i] != b']' {
                            i += 1;
                        }
                        if i < bytes.len() {
                            i += 1; // closing bracket
                        }
                    }
                    _ => {
                        // Numeric (or arbitrary filter) index. Collect
                        // run of digits; give up on anything else so a
                        // filter expression does not corrupt the name.
                        let start = i;
                        while i < bytes.len() && bytes[i].is_ascii_digit() {
                            i += 1;
                        }
                        if start == i {
                            // Unknown bracket content — skip until `]`
                            // without emitting a segment.
                            while i < bytes.len() && bytes[i] != b']' {
                                i += 1;
                            }
                            if i < bytes.len() {
                                i += 1;
                            }
                            continue;
                        }
                        let idx = path[start..i].to_owned();
                        out.push(Segment::Index(idx));
                        while i < bytes.len() && bytes[i] != b']' {
                            i += 1;
                        }
                        if i < bytes.len() {
                            i += 1;
                        }
                    }
                }
            }
            _ => {
                // Skip anything we don't recognise. Protects against
                // garbage tails without infinite-looping.
                i += 1;
            }
        }
    }

    out
}

/// Coerce a raw segment string into a valid Tarn identifier:
///
///   * Replace every non-`[A-Za-z0-9_]` character with `_`.
///   * Collapse runs of `_`.
///   * Prefix with `_` when the first character after coercion is a
///     digit or when the result is empty.
///   * Fall back to `field` when there is nothing to salvage.
fn sanitize(raw: &str) -> String {
    let mut buf = String::with_capacity(raw.len());
    let mut prev_underscore = false;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            if c == '_' {
                if prev_underscore {
                    continue;
                }
                prev_underscore = true;
            } else {
                prev_underscore = false;
            }
            buf.push(c);
        } else if !prev_underscore && !buf.is_empty() {
            buf.push('_');
            prev_underscore = true;
        }
    }
    // Strip trailing underscore, it's never meaningful.
    while buf.ends_with('_') {
        buf.pop();
    }
    // Empty / still-invalid → fall back.
    if buf.is_empty() {
        return "field".to_owned();
    }
    if let Some(first) = buf.chars().next() {
        if first.is_ascii_digit() {
            buf.insert(0, '_');
        }
    }
    if !is_valid_identifier(&buf) {
        return "field".to_owned();
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- leaf_name ----------

    #[test]
    fn leaf_name_for_simple_dot_key() {
        assert_eq!(leaf_name("$.id"), "id");
    }

    #[test]
    fn leaf_name_for_trailing_index_combines_with_previous_key() {
        assert_eq!(leaf_name("$.data[0]"), "data_0");
    }

    #[test]
    fn leaf_name_for_index_then_key_returns_last_key() {
        assert_eq!(leaf_name("$.data[0].id"), "id");
    }

    #[test]
    fn leaf_name_for_nested_dot_key() {
        assert_eq!(leaf_name("$.user.email"), "email");
    }

    #[test]
    fn leaf_name_for_wildcard_inherits_previous_key() {
        assert_eq!(leaf_name("$.tags[*]"), "tags");
    }

    #[test]
    fn leaf_name_for_double_quoted_bracket_key() {
        assert_eq!(leaf_name("$[\"weird-key\"]"), "weird_key");
    }

    #[test]
    fn leaf_name_for_single_quoted_bracket_key() {
        assert_eq!(leaf_name("$['user.email']"), "user_email");
    }

    #[test]
    fn leaf_name_for_pure_index_falls_back_to_index_marker() {
        assert_eq!(leaf_name("$[5]"), "index_5");
    }

    #[test]
    fn leaf_name_for_empty_path_falls_back_to_field() {
        assert_eq!(leaf_name(""), "field");
    }

    #[test]
    fn leaf_name_for_root_alone_falls_back_to_field() {
        assert_eq!(leaf_name("$"), "field");
    }

    #[test]
    fn leaf_name_sanitises_leading_digit() {
        // `.123key` is weird but possible in quoted form; we still
        // want a valid identifier on the way out.
        assert_eq!(leaf_name("$[\"123key\"]"), "_123key");
    }

    #[test]
    fn leaf_name_wildcard_without_previous_key_falls_back_to_field() {
        assert_eq!(leaf_name("$[*]"), "field");
    }

    // ---------- infer_type ----------

    #[test]
    fn infer_type_covers_number_string_boolean_array_object_null() {
        use serde_json::json;
        assert_eq!(infer_type(&json!(42)), "number");
        // Using an arbitrary float value — 2.5 avoids clippy's
        // `approx_constant` lint that fires on 3.14 (close to PI).
        assert_eq!(infer_type(&json!(2.5)), "number");
        assert_eq!(infer_type(&json!("hello")), "string");
        assert_eq!(infer_type(&json!(true)), "boolean");
        assert_eq!(infer_type(&json!(false)), "boolean");
        assert_eq!(infer_type(&json!([1, 2, 3])), "array");
        assert_eq!(infer_type(&json!({"k": "v"})), "object");
        assert_eq!(infer_type(&serde_json::Value::Null), "null");
    }

    #[test]
    fn infer_type_distinguishes_empty_array_and_object_from_null() {
        use serde_json::json;
        assert_eq!(infer_type(&json!([])), "array");
        assert_eq!(infer_type(&json!({})), "object");
    }
}
