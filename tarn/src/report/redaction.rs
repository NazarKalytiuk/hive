use crate::assert::types::AssertionResult;
use crate::model::RedactionConfig;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

pub fn sanitize_assertion(
    assertion: &AssertionResult,
    redaction: &RedactionConfig,
    secret_values: &[String],
) -> AssertionResult {
    AssertionResult {
        assertion: assertion.assertion.clone(),
        passed: assertion.passed,
        expected: sanitize_string(&assertion.expected, &redaction.replacement, secret_values),
        actual: sanitize_string(&assertion.actual, &redaction.replacement, secret_values),
        message: sanitize_string(&assertion.message, &redaction.replacement, secret_values),
        diff: assertion
            .diff
            .as_ref()
            .map(|diff| sanitize_string(diff, &redaction.replacement, secret_values)),
        location: assertion.location.clone(),
        response_shape_mismatch: assertion.response_shape_mismatch.clone(),
    }
}

pub fn sanitize_string(input: &str, replacement: &str, secret_values: &[String]) -> String {
    let mut output = input.to_string();
    for secret in sorted_secret_values(secret_values) {
        output = output.replace(secret.as_str(), replacement);
    }
    output
}

pub fn sanitize_json(value: &Value, replacement: &str, secret_values: &[String]) -> Value {
    let secrets = sorted_secret_values(secret_values);
    sanitize_json_with_sorted(value, replacement, &secrets)
}

pub fn redact_headers(
    headers: &HashMap<String, String>,
    redaction: &RedactionConfig,
    secret_values: &[String],
) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(k, v)| {
            if redaction
                .headers
                .iter()
                .any(|header| header.eq_ignore_ascii_case(k))
            {
                (k.clone(), redaction.replacement.clone())
            } else {
                (
                    k.clone(),
                    sanitize_string(v, &redaction.replacement, secret_values),
                )
            }
        })
        .collect()
}

fn sanitize_json_with_sorted(value: &Value, replacement: &str, secret_values: &[String]) -> Value {
    match value {
        Value::String(s) => {
            Value::String(sanitize_string_with_sorted(s, replacement, secret_values))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| sanitize_json_with_sorted(item, replacement, secret_values))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    (
                        sanitize_string_with_sorted(k, replacement, secret_values),
                        sanitize_json_with_sorted(v, replacement, secret_values),
                    )
                })
                .collect(),
        ),
        other => {
            let rendered = other.to_string();
            if secret_values
                .iter()
                .any(|secret| secret.as_str() == rendered.as_str())
            {
                Value::String(replacement.to_string())
            } else {
                other.clone()
            }
        }
    }
}

fn sanitize_string_with_sorted(input: &str, replacement: &str, secret_values: &[String]) -> String {
    let mut output = input.to_string();
    for secret in secret_values {
        output = output.replace(secret.as_str(), replacement);
    }
    output
}

fn sorted_secret_values(secret_values: &[String]) -> Vec<String> {
    let mut values: Vec<String> = secret_values
        .iter()
        .filter(|value| !value.is_empty())
        .cloned()
        .collect();
    values.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    values.dedup();
    values
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sanitize_string_replaces_longest_match_first() {
        let secrets = vec!["abcd".into(), "abc".into()];
        let sanitized = sanitize_string("token=abcd", "***", &secrets);
        assert_eq!(sanitized, "token=***");
    }

    #[test]
    fn sanitize_json_replaces_nested_strings() {
        let secrets = vec!["secret-token".into()];
        let value = json!({
            "token": "secret-token",
            "nested": ["Bearer secret-token"],
        });

        let sanitized = sanitize_json(&value, "***", &secrets);
        assert_eq!(
            sanitized,
            json!({
                "token": "***",
                "nested": ["Bearer ***"],
            })
        );
    }

    #[test]
    fn redact_headers_applies_name_and_value_redaction() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".into(), "Bearer token".into());
        headers.insert("X-Trace".into(), "trace-secret".into());

        let redaction = RedactionConfig {
            headers: vec!["authorization".into()],
            replacement: "[hidden]".into(),
            env_vars: Vec::new(),
            captures: Vec::new(),
        };

        let sanitized = redact_headers(&headers, &redaction, &["trace-secret".into()]);
        assert_eq!(sanitized.get("Authorization").unwrap(), "[hidden]");
        assert_eq!(sanitized.get("X-Trace").unwrap(), "[hidden]");
    }

    #[test]
    fn sanitize_assertion_updates_all_text_fields() {
        let assertion = AssertionResult::fail_with_diff(
            "body $",
            "secret-token",
            "secret-token",
            "Expected secret-token",
            "--- secret-token",
        );
        let redaction = RedactionConfig {
            replacement: "***".into(),
            ..RedactionConfig::default()
        };

        let sanitized = sanitize_assertion(&assertion, &redaction, &["secret-token".into()]);
        assert_eq!(sanitized.expected, "***");
        assert_eq!(sanitized.actual, "***");
        assert_eq!(sanitized.message, "Expected ***");
        assert_eq!(sanitized.diff.as_deref(), Some("--- ***"));
    }
}
