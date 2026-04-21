//! TL003 — polling step with a weak stop condition.
//!
//! `poll:` loops re-execute a step until the `until:` assertions pass.
//! If `until` has no body-shape assertion and only a broad status
//! range (or nothing narrower than "2xx"), a server that returns 200
//! during a "still pending" state will immediately satisfy the stop
//! condition and the poll terminates prematurely — or, conversely,
//! a server that *never* transitions state will loop to `max_attempts`
//! without ever telling the user *why*. Either way the developer's
//! intent — "wait until the resource is ready" — isn't encoded.
//!
//! Rule fires when:
//! - the step has `poll:` configured
//! - AND `until` has no `body` assertions
//! - AND the status assertion is missing, `"2xx"`/`"3xx"`/`"4xx"`,
//!   or a range that spans more than one class
//!
//! We stay quiet when `until.body` is present (even one entry), when
//! `until.headers` is present (a Retry-After-style header can be a
//! legitimate terminator), or when `until.status` is an exact code —
//! those are all strong enough signals.

use crate::lint::{finding_from_step, walk_steps, Finding, Severity};
use crate::model::{Assertion, StatusAssertion, TestFile};

pub fn lint(file: &TestFile, path: &str) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (step_path, step) in walk_steps(file, path) {
        let Some(poll) = &step.poll else { continue };
        if !is_weak_stop_condition(&poll.until) {
            continue;
        }
        findings.push(finding_from_step(
            "TL003",
            Severity::Warning,
            path,
            Some(step_path.clone()),
            step,
            format!(
                "Polling step `{}` has a weak stop condition — no body assertion and status is missing or a broad range.",
                step.name
            ),
            Some(
                "Polling without a strict success criterion can loop until timeout or terminate too early. Add a body assertion (e.g. `$.status: ready`) that distinguishes terminal from transient states.".to_string(),
            ),
        ));
    }
    findings
}

fn is_weak_stop_condition(until: &Assertion) -> bool {
    // Any body assertion at all is a strong signal — the user has
    // encoded "the response looks like X when I'm done".
    if until.body.as_ref().is_some_and(|m| !m.is_empty()) {
        return false;
    }
    if until.headers.as_ref().is_some_and(|m| !m.is_empty()) {
        return false;
    }
    match &until.status {
        None => true,
        Some(StatusAssertion::Exact(_)) => false,
        Some(StatusAssertion::Shorthand(s)) => is_broad_status_shorthand(s),
        Some(StatusAssertion::Complex(_)) => true,
    }
}

fn is_broad_status_shorthand(s: &str) -> bool {
    // `"2xx"`, `"3xx"`, `"4xx"`, `"5xx"` — all broad. Exact strings
    // like `"201"` (unusual but legal) are narrow and must not fire.
    let lower = s.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "1xx" | "2xx" | "3xx" | "4xx" | "5xx" | "xxx"
    )
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
    fn fires_on_poll_with_only_2xx_status_and_no_body() {
        let file = parse(
            r#"
name: polling
steps:
  - name: wait for ready
    request:
      method: GET
      url: "http://example.com/jobs/1"
    poll:
      interval: "1s"
      max_attempts: 10
      until:
        status: "2xx"
"#,
        );
        let findings = lint(&file, "t.tarn.yaml");
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].rule_id, "TL003");
    }

    #[test]
    fn quiet_when_body_assertion_is_present() {
        let file = parse(
            r#"
name: polling-ok
steps:
  - name: wait for ready
    request:
      method: GET
      url: "http://example.com/jobs/1"
    poll:
      interval: "1s"
      max_attempts: 10
      until:
        status: 200
        body:
          "$.status": "ready"
"#,
        );
        let findings = lint(&file, "t.tarn.yaml");
        assert!(findings.is_empty());
    }

    #[test]
    fn quiet_when_status_is_exact_even_without_body() {
        // An exact status — e.g. "poll until we see 204 No Content" —
        // is a narrow, specific terminator. Not flagged.
        let file = parse(
            r#"
name: exact
steps:
  - name: wait for drain
    request:
      method: GET
      url: "http://example.com/queue"
    poll:
      interval: "500ms"
      max_attempts: 20
      until:
        status: 204
"#,
        );
        let findings = lint(&file, "t.tarn.yaml");
        assert!(findings.is_empty(), "got {:?}", findings);
    }
}
