//! TL006 — capture path with no corresponding body type assertion.
//!
//! When a step captures a value but never asserts anything about the
//! response body, a silent shape change (field renamed, type flipped
//! from string to object) slips past and cascades into later steps
//! that reference the captured variable. Pairing a capture with even
//! a minimal body assertion lets response-shape drift (NAZ-415)
//! surface at the capture site instead of at a mysterious downstream
//! interpolation failure.

use crate::lint::{capture_jsonpath, finding_from_step, walk_steps, Finding, Severity};
use crate::model::TestFile;

pub fn lint(file: &TestFile, path: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (step_path, step) in walk_steps(file, path) {
        // Need at least one JSONPath-backed capture. Header / cookie /
        // status captures are structurally safer (header names don't
        // drift silently), so they don't trigger this rule.
        let has_jsonpath_capture = step.capture.values().any(|c| capture_jsonpath(c).is_some());
        if !has_jsonpath_capture {
            continue;
        }
        // If the step has *any* body assertion we consider the shape
        // pinned enough — the lint's goal is surfacing, not
        // micromanaging what the user pins.
        let body_asserted = step
            .assertions
            .as_ref()
            .and_then(|a| a.body.as_ref())
            .is_some_and(|m| !m.is_empty());
        if body_asserted {
            continue;
        }
        findings.push(finding_from_step(
            "TL006",
            Severity::Info,
            path,
            Some(step_path.clone()),
            step,
            format!(
                "Step `{}` captures from the response body but asserts nothing about its shape.",
                step.name
            ),
            Some(
                "Pair captures with a body type assertion (e.g. `is_object`, `is_uuid`) so shape drift surfaces immediately instead of cascading into skipped steps.".to_string(),
            ),
        ));
    }
    findings
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
    fn fires_on_capture_without_body_assertion() {
        let file = parse(
            r#"
name: unchecked-capture
steps:
  - name: login
    request:
      method: POST
      url: "http://example.com/login"
      body: { user: x }
    capture:
      token: "$.token"
    assert:
      status: 200
"#,
        );
        let findings = lint(&file, "t.tarn.yaml");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule_id, "TL006");
        assert_eq!(findings[0].severity, Severity::Info);
    }

    #[test]
    fn quiet_when_body_has_any_assertion() {
        let file = parse(
            r#"
name: ok
steps:
  - name: login
    request:
      method: POST
      url: "http://example.com/login"
      body: { user: x }
    capture:
      token: "$.token"
    assert:
      status: 200
      body:
        "$.token": { exists: true }
"#,
        );
        let findings = lint(&file, "t.tarn.yaml");
        assert!(findings.is_empty());
    }

    #[test]
    fn quiet_when_only_captures_are_headers() {
        // A header capture doesn't carry response-shape risk.
        let file = parse(
            r#"
name: headers-only
steps:
  - name: login
    request:
      method: POST
      url: "http://example.com/login"
      body: { user: x }
    capture:
      cookie:
        header: "Set-Cookie"
    assert:
      status: 200
"#,
        );
        let findings = lint(&file, "t.tarn.yaml");
        assert!(findings.is_empty(), "got {:?}", findings);
    }
}
