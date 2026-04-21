//! TL005 — shorthand status assertion where a specific code would be
//! more diagnostic.
//!
//! `status: "2xx"` is a useful smoke assertion, but when the step
//! name implies a specific outcome (e.g. `"creates user"`, `"deletes
//! user"`, `"returns 201"`) the user almost certainly *knows* the
//! expected code. Using the shorthand there hides the signal: a
//! mutation drifting from 201 to 200 slips through, and later schema
//! drift detection (NAZ-415) won't know what normal looks like.
//!
//! The rule is deliberately noisy-only-when-the-name-tells-us: we
//! match simple verb heuristics in the step/test name. If the
//! heuristic is unsure, we stay silent.

use crate::lint::{finding_from_step, walk_steps, Finding, Severity};
use crate::model::{StatusAssertion, TestFile};

pub fn lint(file: &TestFile, path: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (step_path, step) in walk_steps(file, path) {
        let Some(assertion) = &step.assertions else {
            continue;
        };
        let Some(status) = &assertion.status else {
            continue;
        };
        let StatusAssertion::Shorthand(s) = status else {
            continue;
        };
        if !is_broad_shorthand(s) {
            continue;
        }
        let expected = expected_code_from_name(&step.name);
        if expected.is_none() {
            continue;
        }
        let expected = expected.unwrap();
        findings.push(finding_from_step(
            "TL005",
            Severity::Info,
            path,
            Some(step_path.clone()),
            step,
            format!(
                "Step `{}` asserts `status: \"{}\"` but its name suggests an expected code of {}.",
                step.name, s, expected
            ),
            Some(
                "A literal status value narrows diagnosis when an endpoint drifts from 201 to 200 (or vice versa).".to_string(),
            ),
        ));
    }
    findings
}

fn is_broad_shorthand(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "1xx" | "2xx" | "3xx" | "4xx" | "5xx" | "xxx"
    )
}

/// Very conservative verb heuristic — only returns `Some` when the
/// name pattern is unambiguous. False positives here are worse than
/// false negatives, because lint must not be noisy.
fn expected_code_from_name(name: &str) -> Option<u16> {
    let lower = name.to_ascii_lowercase();
    // "creates X" / "create X" / "POST X" → 201
    if lower.starts_with("create") || lower.contains(" creates ") || lower.starts_with("post ") {
        return Some(201);
    }
    // "deletes X" / "delete X" → 204 (most common) — we don't guess
    // 200 here; `204` is the API-design default and if the service
    // uses 200 the user can make the assertion literal.
    if lower.starts_with("delete") || lower.contains(" deletes ") {
        return Some(204);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::parse_str;
    use std::path::Path;

    fn parse(source: &str) -> TestFile {
        parse_str(source, Path::new("t.tarn.yaml")).expect("parse")
    }

    #[test]
    fn fires_on_create_with_2xx_shorthand() {
        let file = parse(
            r#"
name: weak-status
steps:
  - name: create user
    request:
      method: POST
      url: "http://example.com/users"
      body: { name: x }
    assert:
      status: "2xx"
"#,
        );
        let findings = lint(&file, "t.tarn.yaml");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule_id, "TL005");
        assert_eq!(findings[0].severity, Severity::Info);
    }

    #[test]
    fn quiet_when_status_is_literal_201() {
        let file = parse(
            r#"
name: literal
steps:
  - name: create user
    request:
      method: POST
      url: "http://example.com/users"
      body: { name: x }
    assert:
      status: 201
"#,
        );
        let findings = lint(&file, "t.tarn.yaml");
        assert!(findings.is_empty());
    }

    #[test]
    fn quiet_when_step_name_gives_no_signal() {
        // Non-CRUD step name → heuristic returns None → no fire.
        let file = parse(
            r#"
name: opaque-name
steps:
  - name: poke endpoint
    request:
      method: GET
      url: "http://example.com/health"
    assert:
      status: "2xx"
"#,
        );
        let findings = lint(&file, "t.tarn.yaml");
        assert!(findings.is_empty(), "got {:?}", findings);
    }
}
