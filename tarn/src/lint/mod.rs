//! Structural lint rules for Tarn test files.
//!
//! `tarn lint` is a companion to `tarn validate` that catches *reliability
//! and correctness smells* the schema validator cannot — things like
//! positional captures on shared list endpoints, polling loops with weak
//! stop conditions, and mutations without a status assertion. Validation
//! answers "will this parse and run?"; lint answers "is this a test that
//! won't fall over next month?".
//!
//! Each rule lives in its own module under `lint/` and implements a pure
//! `fn lint(file: &TestFile, path: &str) -> Vec<Finding>`. Keeping rules
//! independent and pure makes them trivially composable, individually
//! testable, and easy to add without touching the orchestrator.
//!
//! The orchestrator walks all rules, merges findings, sorts by (file,
//! line, rule_id), and leaves severity filtering + rendering to the CLI
//! layer.

use std::path::Path;

use crate::model::{CaptureSpec, Step, TestFile};
use crate::parser;

pub mod tl001_positional_capture;
pub mod tl002_shared_list_capture;
pub mod tl003_weak_polling;
pub mod tl004_missing_status_on_mutation;
pub mod tl005_weak_status_assertion;
pub mod tl006_capture_without_body_assertion;
pub mod tl007_duplicate_test_name;
pub mod tl008_hardcoded_absolute_url;

/// Severity bucket carried on every [`Finding`].
///
/// The three-way split mirrors the ticket's acceptance criteria:
/// `Error` is for things that are *wrong* and break test semantics
/// (e.g. duplicate test names that confuse the selector), `Warning` is
/// for reliability hazards that will probably break in a month, and
/// `Info` is for style/diagnostic suggestions that never need to fail
/// CI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        }
    }

    /// Parse a CLI-provided severity threshold. Case-insensitive so
    /// `--severity WARNING` behaves the same as `--severity warning`.
    pub fn parse_threshold(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "error" => Some(Severity::Error),
            "warning" | "warn" => Some(Severity::Warning),
            "info" => Some(Severity::Info),
            _ => None,
        }
    }
}

/// One structured lint finding, ready for human or JSON rendering.
///
/// `line` / `column` are optional because not every rule has a node
/// location to point at — TL007 (duplicate test name) anchors at the
/// *second* occurrence's step, which may be `None` if the file parsed
/// without location metadata (e.g. the rule was exercised in a unit
/// test with a hand-built `TestFile`). Callers that want a guaranteed
/// pointer should fall back to the file path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule_id: &'static str,
    pub severity: Severity,
    pub file: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    /// `FILE::TEST::STEP` (or `FILE::STEP` for flat-step files). Lets
    /// editors and agents navigate without re-parsing the YAML.
    pub step_path: Option<String>,
    pub message: String,
    pub hint: Option<String>,
}

/// Options that influence rule behavior. Populated from CLI flags so
/// rules don't have to know about `clap`.
#[derive(Debug, Clone, Default)]
pub struct LintOptions {
    /// Suppress TL008 entirely. Useful for projects that deliberately
    /// test third-party services by absolute URL.
    pub allow_absolute_urls: bool,
}

/// Iterator-friendly view of every step in a [`TestFile`] paired with a
/// synthetic `step_path`. Setup and teardown steps are included so
/// rules see the whole surface; per-rule logic can filter further.
///
/// `file_path` is the *file-system* path the file was loaded from,
/// used as the stable prefix in `FILE::TEST::STEP` selectors. Tarn's
/// `TestFile.name` is the human-readable header inside the YAML, not
/// the file path — we keep that distinction explicit so selector
/// output matches what `tarn run --select` expects.
pub(crate) fn walk_steps<'a>(file: &'a TestFile, file_path: &str) -> Vec<(String, &'a Step)> {
    let mut out = Vec::new();
    for step in &file.setup {
        out.push((format!("{}::setup::{}", file_path, step.name), step));
    }
    for step in &file.steps {
        out.push((format!("{}::{}", file_path, step.name), step));
    }
    for (test_name, group) in &file.tests {
        for step in &group.steps {
            out.push((format!("{}::{}::{}", file_path, test_name, step.name), step));
        }
    }
    for step in &file.teardown {
        out.push((format!("{}::teardown::{}", file_path, step.name), step));
    }
    out
}

/// Extract the JSONPath portion of a capture (if any). `Header` / cookie
/// captures have no JSONPath to analyze and return `None`.
pub(crate) fn capture_jsonpath(spec: &CaptureSpec) -> Option<&str> {
    match spec {
        CaptureSpec::JsonPath(s) => Some(s.as_str()),
        CaptureSpec::Extended(ext) => ext.jsonpath.as_deref(),
    }
}

/// Attach the file path and step location to a partial finding.
pub(crate) fn finding_from_step(
    rule_id: &'static str,
    severity: Severity,
    file: &str,
    step_path: Option<String>,
    step: &Step,
    message: String,
    hint: Option<String>,
) -> Finding {
    Finding {
        rule_id,
        severity,
        file: file.to_string(),
        line: step.location.as_ref().map(|l| l.line),
        column: step.location.as_ref().map(|l| l.column),
        step_path,
        message,
        hint,
    }
}

/// Run every lint rule against a parsed test file and return merged,
/// sorted findings. Rules are invoked in declaration order; the final
/// sort is by `(line, rule_id)` so human output reads top-to-bottom.
pub fn lint_file(file: &TestFile, path: &str, opts: &LintOptions) -> Vec<Finding> {
    let mut findings = Vec::new();
    findings.extend(tl001_positional_capture::lint(file, path));
    findings.extend(tl002_shared_list_capture::lint(file, path));
    findings.extend(tl003_weak_polling::lint(file, path));
    findings.extend(tl004_missing_status_on_mutation::lint(file, path));
    findings.extend(tl005_weak_status_assertion::lint(file, path));
    findings.extend(tl006_capture_without_body_assertion::lint(file, path));
    findings.extend(tl007_duplicate_test_name::lint(file, path));
    findings.extend(tl008_hardcoded_absolute_url::lint(file, path, opts));
    findings.sort_by(|a, b| {
        a.line
            .unwrap_or(0)
            .cmp(&b.line.unwrap_or(0))
            .then_with(|| a.rule_id.cmp(b.rule_id))
    });
    findings
}

/// Read a file from disk and run every rule. Returns `Err` with an
/// exit-code-2 friendly message when the file cannot be read or parsed.
///
/// This is the surface `tarn lint` calls per discovered file; it's
/// public because `tarn-lsp` (future ticket) will want the same
/// entry point for in-editor diagnostics.
pub fn lint_path(path: &Path, opts: &LintOptions) -> Result<Vec<Finding>, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    // Lint requires a fully-parsed tree (we walk typed captures, polls,
    // etc.). If parsing fails, we surface the parser error verbatim so
    // the user gets the same diagnostic `tarn validate` would print and
    // the command exits 2. We deliberately do NOT fall back to partial
    // linting — a malformed file is validate's job, not lint's.
    let file = parser::parse_str(&source, path).map_err(|e| e.to_string())?;
    Ok(lint_file(&file, &path.display().to_string(), opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_ordering_lets_threshold_filter_correctly() {
        assert!(Severity::Error > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
    }

    #[test]
    fn parse_threshold_accepts_canonical_forms() {
        assert_eq!(Severity::parse_threshold("error"), Some(Severity::Error));
        assert_eq!(
            Severity::parse_threshold("WARNING"),
            Some(Severity::Warning)
        );
        assert_eq!(Severity::parse_threshold("warn"), Some(Severity::Warning));
        assert_eq!(Severity::parse_threshold("info"), Some(Severity::Info));
        assert_eq!(Severity::parse_threshold("nonsense"), None);
    }
}
