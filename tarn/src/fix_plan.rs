//! Shared "fix plan" surface backing both the `tarn-mcp` `tarn_fix_plan`
//! tool and the `tarn-lsp` **Quick Fix** code action (NAZ-305, Phase L3.4).
//!
//! Two distinct inputs, one output shape:
//!
//!   * [`generate_fix_plan_from_report`] consumes the JSON [`RunResult`]
//!     that `tarn run --format json` emits and summarises every failing
//!     step into an advice-only [`FixPlan`] — no structured edits, just
//!     a human-readable description + actionable hints. This is the path
//!     the MCP tool has always used. The former inline implementation
//!     inside `tarn-mcp/src/tools.rs` now calls this function directly so
//!     the MCP surface and any future consumer share one source of truth.
//!
//!   * [`generate_fix_plan`] consumes an in-memory source buffer and a
//!     slice of [`ValidationMessage`]s (the same type `tarn-lsp` reads
//!     out of `validate_document`) and emits [`FixPlan`]s **with
//!     concrete edits** whenever a message carries a mechanically-
//!     fixable suggestion — specifically, the `Unknown field 'X' …
//!     Did you mean 'Y'?` pattern that the parser already emits. These
//!     plans drive the LSP Quick Fix code action: the `edits` vector is
//!     handed straight to `WorkspaceEdit.changes` and the editor applies
//!     them on click.
//!
//! The two functions live side-by-side because both produce a `FixPlan`
//! — same struct, same serialization — but they operate on different
//! inputs and have different guarantees about which fields are populated.
//! Splitting them into two modules would force every caller to learn two
//! APIs for one concept.
//!
//! # Why only the "Did you mean" pattern carries edits
//!
//! Tarn's validator emits many shape errors ("Step must have a URL",
//! "Body assertion format mismatch", …) but only the unknown-field typo
//! path can be mechanically healed without asking the user for more
//! input. Non-fix-plan validation messages flow through the LSP
//! diagnostics pipeline as-is and simply do not get a Quick Fix offered.
//! Declining to offer a fix is **not** an error — see the acceptance
//! criteria in NAZ-305.
//!
//! # Stability
//!
//! The JSON shape emitted by [`generate_fix_plan_from_report`] is pinned
//! by `tarn-mcp/tests/golden/fix-plan.json.golden`. Any change to the
//! MCP-facing output must be reflected there. The LSP-facing
//! [`generate_fix_plan`] is not serialized to disk — callers use the
//! struct directly — so its shape can evolve more freely, but changes
//! must still keep existing callers compiling.

use serde_json::Value;
use std::collections::VecDeque;
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser};
use yaml_rust2::scanner::Marker;

use crate::model::Location;
use crate::validation::{ValidationCode, ValidationMessage};

/// A single fix plan entry.
///
/// A plan is either **actionable** (`edits` non-empty) or **advisory**
/// (`edits` empty + `description` populated). The MCP path always
/// produces advisory plans; the LSP path always produces actionable
/// ones. Downstream consumers can check `edits.is_empty()` to decide
/// whether to render a Quick Fix button or a read-only explanation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixPlan {
    /// Stable validation code this plan addresses, matching the
    /// `ValidationMessage.code` that produced it (for the LSP path)
    /// or a failure-category string (for the MCP path). Downstream
    /// code uses this to correlate plans with diagnostics.
    pub diagnostic_code: String,
    /// Short user-facing title. For the LSP path this is the text the
    /// editor shows in the Quick Fix menu; for the MCP path it is the
    /// human-readable summary the MCP tool emits as `summary`.
    pub title: String,
    /// Concrete, pre-computed edits the client can apply without any
    /// further interaction. Empty for advice-only plans.
    pub edits: Vec<FixEdit>,
    /// `true` when the plan is the obvious, unambiguous fix for the
    /// diagnostic. LSP clients that pay attention to
    /// `CodeAction.isPreferred` will auto-select plans with this flag
    /// set.
    pub preferred: bool,
    /// Optional prose that accompanies the plan. The MCP path always
    /// populates this with its summary-text machinery; the LSP path
    /// leaves it `None` because the title already carries the
    /// user-facing copy.
    pub description: Option<String>,
}

/// A single text edit inside a [`FixPlan`]. Carries a [`Location`] so
/// consumers do not depend on LSP types at the crate level — both the
/// `tarn-lsp` code-action path and future non-LSP callers convert the
/// 1-based point into whatever range shape they need.
///
/// Ranges here are **half-open**: `range` is the starting position and
/// `length` is the number of characters (on the same line) to replace.
/// Every fix this module currently produces is a single-line replacement
/// of a YAML mapping key, so a point + length is both sufficient and
/// cheaper than carrying full `(start, end)` pairs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixEdit {
    /// 1-based start position of the slice to replace.
    pub range: Location,
    /// Number of characters on `range.line` to replace, starting at
    /// `range.column`. Always `>= 1` for the edits this module produces.
    pub length: usize,
    /// Replacement text. Never contains newlines in the current
    /// implementation.
    pub new_text: String,
}

// ---------------------------------------------------------------------
// Report-driven path (MCP tool)
// ---------------------------------------------------------------------

/// Convert a `tarn run` JSON report into a list of advisory fix plans,
/// one per failed step. Used by `tarn-mcp`'s `tarn_fix_plan` tool.
///
/// Returns a list ordered by priority (parse/connection errors first,
/// then timeouts, capture errors, and assertion failures). Each plan
/// carries the step's failure category, a short summary line, and the
/// remediation hints the runner already emitted into the report. Plans
/// from this path always have `edits: Vec::new()` — fixing a failed
/// test run inherently requires semantic judgement the library cannot
/// automate.
///
/// `max_items` caps the returned vector so callers do not have to
/// re-sort/truncate themselves.
pub fn generate_fix_plan_from_report(report: &Value, max_items: usize) -> Vec<ReportFixItem> {
    let Some(files) = report.get("files").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut items: Vec<ReportFixItem> = Vec::new();

    for file in files {
        let file_name = file
            .get("file")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        for step in file
            .get("setup")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(item) = report_item(&file_name, "setup", step) {
                items.push(item);
            }
        }

        for test in file
            .get("tests")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let test_name = test.get("name").and_then(Value::as_str).unwrap_or("test");
            for step in test
                .get("steps")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(item) = report_item(&file_name, test_name, step) {
                    items.push(item);
                }
            }
        }

        for step in file
            .get("teardown")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(item) = report_item(&file_name, "teardown", step) {
                items.push(item);
            }
        }
    }

    items.sort_by(|a, b| {
        a.priority_rank
            .cmp(&b.priority_rank)
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.step.cmp(&b.step))
    });
    items.truncate(max_items);
    items
}

/// Report-driven fix item. Kept as a dedicated type so the MCP surface
/// can serialize it to the exact JSON shape its golden contract pins
/// without forcing the `FixPlan` struct to grow report-only fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportFixItem {
    pub file: String,
    pub scope: String,
    pub step: String,
    pub failure_category: String,
    pub error_code: String,
    pub priority: &'static str,
    pub priority_rank: u64,
    pub summary: String,
    pub actions: Vec<Value>,
    pub request_url: Value,
    pub response_status: Value,
    pub failed_assertions: Vec<Value>,
}

impl ReportFixItem {
    /// Serialize the item to the JSON shape pinned by
    /// `tarn-mcp/tests/golden/fix-plan.json.golden`. Kept here — not on
    /// the MCP side — so every fix-plan consumer that emits the same
    /// shape uses the same serializer.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "file": self.file,
            "scope": self.scope,
            "step": self.step,
            "failure_category": self.failure_category,
            "error_code": self.error_code,
            "priority": self.priority,
            "priority_rank": self.priority_rank,
            "summary": self.summary,
            "actions": self.actions,
            "evidence": {
                "request_url": self.request_url,
                "response_status": self.response_status,
                "failed_assertions": self.failed_assertions,
            }
        })
    }
}

fn report_item(file_name: &str, scope: &str, step: &Value) -> Option<ReportFixItem> {
    if step.get("status")?.as_str()? != "FAILED" {
        return None;
    }

    let failure_category = step
        .get("failure_category")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let error_code = step
        .get("error_code")
        .and_then(Value::as_str)
        .unwrap_or(&failure_category)
        .to_string();
    let failed_assertions = step
        .get("assertions")
        .and_then(|value| value.get("failures"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let actions = step
        .get("remediation_hints")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    Some(ReportFixItem {
        file: file_name.to_string(),
        scope: scope.to_string(),
        step: step
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        failure_category: failure_category.clone(),
        error_code: error_code.clone(),
        priority: priority_label(&failure_category),
        priority_rank: priority_rank(&failure_category),
        summary: summary_text(&failure_category, &error_code, &failed_assertions),
        actions,
        request_url: step
            .get("request")
            .and_then(|request| request.get("url"))
            .cloned()
            .unwrap_or(Value::Null),
        response_status: step
            .get("response")
            .and_then(|response| response.get("status"))
            .cloned()
            .unwrap_or(Value::Null),
        failed_assertions,
    })
}

fn priority_rank(category: &str) -> u64 {
    match category {
        "parse_error" => 1,
        "connection_error" => 2,
        "timeout" => 3,
        "capture_error" => 4,
        "assertion_failed" => 5,
        _ => 9,
    }
}

fn priority_label(category: &str) -> &'static str {
    match priority_rank(category) {
        1 | 2 => "high",
        3 | 4 => "medium",
        _ => "normal",
    }
}

fn summary_text(category: &str, error_code: &str, failed_assertions: &[Value]) -> String {
    if let Some(message) = failed_assertions
        .first()
        .and_then(|failure| failure.get("message"))
        .and_then(Value::as_str)
    {
        return message.to_string();
    }

    match category {
        "connection_error" => format!("Connectivity issue detected ({error_code})."),
        "timeout" => format!("Operation timed out ({error_code})."),
        "capture_error" => format!("Capture extraction failed ({error_code})."),
        "parse_error" => format!("Test definition or interpolation issue detected ({error_code})."),
        _ => format!("Test step failed ({error_code})."),
    }
}

// ---------------------------------------------------------------------
// Diagnostic-driven path (LSP Quick Fix)
// ---------------------------------------------------------------------

/// Produce a list of [`FixPlan`]s — one per diagnostic whose message
/// matches a fix-plan pattern — for the LSP Quick Fix code action.
///
/// The function is pure: it reads `source` and `diagnostics`, returns
/// a vector, and talks to nothing else. The returned vector is always
/// ordered by diagnostic index (plan *i* addresses `diagnostics[i]`
/// when present; diagnostics without a matching pattern are silently
/// dropped) so callers can zip plans back against the original list
/// if they need to.
///
/// Each returned plan contains exactly the edits needed to apply the
/// suggestion. The [`FixPlan::preferred`] flag is always `true` for
/// plans produced by this function — the "Did you mean X?" suggestion
/// is by construction the single unambiguous fix the parser could
/// compute, so there is never more than one candidate per diagnostic.
pub fn generate_fix_plan(source: &str, diagnostics: &[ValidationMessage]) -> Vec<FixPlan> {
    let mut out: Vec<FixPlan> = Vec::new();
    // Lazily build the YAML key span index on first use — walking the
    // buffer is O(n), cheap, but no point paying for it when no
    // diagnostic carries a typo pattern.
    let mut index: Option<KeySpanIndex> = None;

    for diag in diagnostics {
        if !matches!(
            diag.code,
            ValidationCode::TarnValidation | ValidationCode::TarnParse
        ) {
            continue;
        }
        let Some(parsed) = parse_unknown_field_message(&diag.message) else {
            continue;
        };
        let idx = index.get_or_insert_with(|| KeySpanIndex::build(source));
        let Some(span) = idx.find_key(&parsed.context_path, &parsed.unknown) else {
            continue;
        };
        out.push(FixPlan {
            diagnostic_code: diag.code.as_str().to_string(),
            title: format!("Change '{}' to '{}'", parsed.unknown, parsed.suggestion),
            edits: vec![FixEdit {
                range: Location {
                    file: diag
                        .location
                        .as_ref()
                        .map(|loc| loc.file.clone())
                        .unwrap_or_default(),
                    line: span.line,
                    column: span.column,
                },
                length: parsed.unknown.chars().count(),
                new_text: parsed.suggestion.clone(),
            }],
            preferred: true,
            description: None,
        });
    }
    out
}

/// Return `true` when `diag` has a mechanically-applicable fix plan in
/// `source`. Thin wrapper over [`generate_fix_plan`] that avoids
/// building a whole `Vec` when callers only need the boolean.
pub fn has_fix_plan(source: &str, diag: &ValidationMessage) -> bool {
    !generate_fix_plan(source, std::slice::from_ref(diag)).is_empty()
}

/// Parsed form of a `Unknown field 'X' at root.path. Did you mean 'Y'?`
/// validation message. Returned by [`parse_unknown_field_message`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct UnknownFieldSuggestion {
    unknown: String,
    suggestion: String,
    context_path: Vec<ContextSegment>,
}

/// One segment of the context path the validator emits in an unknown-
/// field error message. `Key("root")`, `Key("steps")`, `Index(0)`,
/// `Key("request")`, etc.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ContextSegment {
    Key(String),
    Index(usize),
}

/// Parse a `Unknown field 'X' at <path>. Did you mean 'Y'?` message
/// into its three components.
///
/// Returns `None` for every message that does not match the pattern —
/// including messages that are of the same family but are missing a
/// suggestion (e.g. a typo too far from any known key). Being strict
/// here keeps the LSP Quick Fix surface small and predictable: we
/// either know the one right answer or decline to offer a fix at all.
fn parse_unknown_field_message(message: &str) -> Option<UnknownFieldSuggestion> {
    // Locate the three anchors. Anything before `Unknown field` is
    // already stripped by `validation::validate_document`, but allow a
    // prefix in case a future caller passes a raw message through.
    let anchor = message.find("Unknown field '")?;
    let rest = &message[anchor + "Unknown field '".len()..];
    let end_unknown = rest.find('\'')?;
    let unknown = rest[..end_unknown].to_string();
    if unknown.is_empty() {
        return None;
    }
    let after_unknown = &rest[end_unknown + 1..];

    // Optional " at <context>. Did you mean 'Y'?" tail.
    let at_marker = " at ";
    let at_pos = after_unknown.find(at_marker)?;
    let after_at = &after_unknown[at_pos + at_marker.len()..];
    // Context runs until the next `. ` — the `Did you mean` clause.
    let did_marker = ". Did you mean '";
    let did_pos = after_at.find(did_marker)?;
    let context_raw = after_at[..did_pos].trim();
    let after_did = &after_at[did_pos + did_marker.len()..];
    let end_sugg = after_did.find('\'')?;
    let suggestion = after_did[..end_sugg].to_string();
    if suggestion.is_empty() {
        return None;
    }

    let context_path = parse_context_path(context_raw)?;
    Some(UnknownFieldSuggestion {
        unknown,
        suggestion,
        context_path,
    })
}

/// Parse a dotted context string like `root.steps[0].request` into a
/// sequence of [`ContextSegment`]s. The leading `root` is preserved so
/// the key-span index can match on a `StreamStart → MappingStart` root
/// without ambiguity. Returns `None` on malformed input.
fn parse_context_path(raw: &str) -> Option<Vec<ContextSegment>> {
    let mut out: Vec<ContextSegment> = Vec::new();
    for part in raw.split('.') {
        if part.is_empty() {
            return None;
        }
        // Split off any trailing `[N]` indices. A single key can carry
        // multiple indices back-to-back (`x[0][1]`) so loop until we
        // consume everything.
        let mut head = part;
        let key_end = head.find('[').unwrap_or(head.len());
        let key = &head[..key_end];
        if key.is_empty() {
            return None;
        }
        out.push(ContextSegment::Key(key.to_string()));
        head = &head[key_end..];
        while let Some(stripped) = head.strip_prefix('[') {
            let close = stripped.find(']')?;
            let num: usize = stripped[..close].parse().ok()?;
            out.push(ContextSegment::Index(num));
            head = &stripped[close + 1..];
        }
        if !head.is_empty() {
            return None;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------
// YAML key span index
// ---------------------------------------------------------------------

/// A single key span the index records. 1-based line / column, mirroring
/// [`crate::model::Location`] everywhere else in the crate.
#[derive(Debug, Clone, Copy)]
struct KeySpan {
    line: usize,
    column: usize,
}

/// Index from a flattened context path — `["root", "steps", "[0]",
/// "request", "header"]` — to the marker of the `header:` key in the
/// source. Built on demand when [`generate_fix_plan`] sees an
/// unknown-field diagnostic.
///
/// The walker uses `yaml-rust2`'s marked event stream — the same
/// primitive [`crate::outline`] relies on — so the key-position logic
/// matches the rest of the crate's span machinery. Invalid YAML is
/// handled by returning an empty index; the caller then quietly skips
/// every diagnostic because no key can be found.
struct KeySpanIndex {
    entries: Vec<(Vec<ContextSegment>, KeySpan)>,
}

impl KeySpanIndex {
    fn build(source: &str) -> Self {
        let mut sink = EventSink { events: Vec::new() };
        let mut parser = Parser::new_from_str(source);
        if parser.load(&mut sink, true).is_err() {
            return Self {
                entries: Vec::new(),
            };
        }
        let mut walker = Walker {
            events: &sink.events,
            pos: 0,
            path: vec![ContextSegment::Key("root".to_string())],
            entries: Vec::new(),
        };
        walker.walk();
        Self {
            entries: walker.entries,
        }
    }

    /// Look up the line/column of the `key:` mapping entry at
    /// `context_path` whose local name is `key`. Returns `None` when
    /// the key is absent — the diagnostic is then silently dropped.
    fn find_key(&self, context_path: &[ContextSegment], key: &str) -> Option<KeySpan> {
        let mut target = context_path.to_vec();
        target.push(ContextSegment::Key(key.to_string()));
        self.entries
            .iter()
            .find(|(p, _)| p == &target)
            .map(|(_, span)| *span)
    }
}

/// Stash every `(Event, Marker)` emitted by `yaml-rust2` so the walker
/// can process them with lookahead. Tarn test files are small and
/// re-walking events is cheap compared to re-parsing the buffer.
struct EventSink {
    events: Vec<(Event, Marker)>,
}

impl MarkedEventReceiver for EventSink {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        self.events.push((ev, mark));
    }
}

/// Event-stream walker that records a `KeySpan` every time it enters a
/// mapping key at any nesting level. The `path` stack grows and
/// shrinks as the walker descends into mapping values and sequence
/// entries.
struct Walker<'a> {
    events: &'a [(Event, Marker)],
    pos: usize,
    path: Vec<ContextSegment>,
    entries: Vec<(Vec<ContextSegment>, KeySpan)>,
}

impl<'a> Walker<'a> {
    fn peek(&self) -> Option<&'a (Event, Marker)> {
        self.events.get(self.pos)
    }

    fn advance(&mut self) -> Option<&'a (Event, Marker)> {
        let event = self.events.get(self.pos);
        if event.is_some() {
            self.pos += 1;
        }
        event
    }

    /// Entry point. Advances through `StreamStart` / `DocumentStart`
    /// and recursively walks the root node.
    fn walk(&mut self) {
        let mut queue: VecDeque<()> = VecDeque::new();
        queue.push_back(());
        // Skip StreamStart / DocumentStart.
        while let Some((ev, _)) = self.peek() {
            if matches!(ev, Event::StreamStart | Event::DocumentStart) {
                self.advance();
                continue;
            }
            break;
        }
        self.walk_node();
    }

    /// Walk a single node of any kind, consuming balanced events. For
    /// mappings, every key adds a [`KeySpan`] entry and recurses into
    /// the value; for sequences, every item advances the trailing
    /// `Index(_)` segment.
    fn walk_node(&mut self) {
        let Some((event, _)) = self.advance() else {
            return;
        };
        match event {
            Event::MappingStart(_, _) => self.walk_mapping(),
            Event::SequenceStart(_, _) => self.walk_sequence(),
            Event::Scalar(_, _, _, _) | Event::Alias(_) => {
                // Leaf — nothing to index.
            }
            _ => {}
        }
    }

    fn walk_mapping(&mut self) {
        loop {
            match self.peek() {
                Some((Event::MappingEnd, _)) => {
                    self.advance();
                    return;
                }
                Some((Event::Scalar(key, _, _, _), mark)) => {
                    let key = key.clone();
                    let span = KeySpan {
                        // `yaml-rust2` lines are 1-based; cols are
                        // 0-based. Bump col to 1-based so everything
                        // downstream matches `crate::model::Location`.
                        line: mark.line(),
                        column: mark.col() + 1,
                    };
                    self.advance();
                    self.path.push(ContextSegment::Key(key));
                    self.entries.push((self.path.clone(), span));
                    self.walk_node();
                    self.path.pop();
                }
                Some(_) => {
                    // Non-scalar key (flow-style mapping with complex
                    // key). Not supported for indexing; skip balanced
                    // so the walker stays synchronised.
                    self.walk_node(); // key
                    self.walk_node(); // value
                }
                None => return,
            }
        }
    }

    fn walk_sequence(&mut self) {
        let mut index: usize = 0;
        loop {
            match self.peek() {
                Some((Event::SequenceEnd, _)) => {
                    self.advance();
                    return;
                }
                Some(_) => {
                    self.path.push(ContextSegment::Index(index));
                    self.walk_node();
                    self.path.pop();
                    index += 1;
                }
                None => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::{Severity, ValidationCode};

    // ---- report-driven path ----

    #[test]
    fn report_fix_plan_returns_empty_when_no_files() {
        let report = serde_json::json!({});
        let items = generate_fix_plan_from_report(&report, 10);
        assert!(items.is_empty());
    }

    #[test]
    fn report_fix_plan_matches_golden_shape_for_single_failure() {
        let report = serde_json::json!({
            "files": [{
                "file": "tests/users.tarn.yaml",
                "tests": [{
                    "name": "smoke",
                    "steps": [{
                        "name": "Create user",
                        "status": "FAILED",
                        "failure_category": "assertion_failed",
                        "error_code": "assertion_mismatch",
                        "remediation_hints": ["hint"],
                        "assertions": {"failures": [{"message": "Expected HTTP 201, got 400"}]},
                        "request": {"url": "https://example.test/users"},
                        "response": {"status": 400}
                    }]
                }]
            }]
        });
        let items = generate_fix_plan_from_report(&report, 10);
        assert_eq!(items.len(), 1);
        let item = &items[0];
        assert_eq!(item.file, "tests/users.tarn.yaml");
        assert_eq!(item.scope, "smoke");
        assert_eq!(item.step, "Create user");
        assert_eq!(item.failure_category, "assertion_failed");
        assert_eq!(item.error_code, "assertion_mismatch");
        assert_eq!(item.priority, "normal");
        assert_eq!(item.priority_rank, 5);
        assert_eq!(item.summary, "Expected HTTP 201, got 400");
    }

    #[test]
    fn report_fix_plan_orders_by_priority_then_file_then_step() {
        let report = serde_json::json!({
            "files": [{
                "file": "a.tarn.yaml",
                "steps": [],
                "tests": [{
                    "name": "t",
                    "steps": [
                        {"name": "z_asserting", "status": "FAILED", "failure_category": "assertion_failed"},
                        {"name": "a_parse", "status": "FAILED", "failure_category": "parse_error"}
                    ]
                }]
            }]
        });
        let items = generate_fix_plan_from_report(&report, 10);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].failure_category, "parse_error");
        assert_eq!(items[0].priority, "high");
        assert_eq!(items[1].failure_category, "assertion_failed");
    }

    #[test]
    fn report_fix_plan_truncates_to_max_items() {
        let mut steps = Vec::new();
        for i in 0..5 {
            steps.push(serde_json::json!({
                "name": format!("step_{i}"),
                "status": "FAILED",
                "failure_category": "assertion_failed"
            }));
        }
        let report = serde_json::json!({
            "files": [{
                "file": "f.tarn.yaml",
                "tests": [{"name": "t", "steps": steps}]
            }]
        });
        let items = generate_fix_plan_from_report(&report, 3);
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn report_fix_plan_skips_passing_steps() {
        let report = serde_json::json!({
            "files": [{
                "file": "f.tarn.yaml",
                "tests": [{
                    "name": "t",
                    "steps": [
                        {"name": "ok", "status": "PASSED"},
                        {"name": "bad", "status": "FAILED", "failure_category": "timeout"}
                    ]
                }]
            }]
        });
        let items = generate_fix_plan_from_report(&report, 10);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].step, "bad");
    }

    // ---- diagnostic-driven path ----

    fn msg(message: &str, code: ValidationCode) -> ValidationMessage {
        ValidationMessage {
            severity: Severity::Error,
            code,
            message: message.to_string(),
            location: Some(Location {
                file: "t.tarn.yaml".into(),
                line: 1,
                column: 1,
            }),
        }
    }

    #[test]
    fn parse_unknown_field_message_extracts_three_parts() {
        let parsed =
            parse_unknown_field_message("Unknown field 'step' at root. Did you mean 'steps'?")
                .unwrap();
        assert_eq!(parsed.unknown, "step");
        assert_eq!(parsed.suggestion, "steps");
        assert_eq!(
            parsed.context_path,
            vec![ContextSegment::Key("root".into())]
        );
    }

    #[test]
    fn parse_unknown_field_message_handles_index_and_nested_context() {
        let parsed = parse_unknown_field_message(
            "Unknown field 'header' at root.steps[0].request. Did you mean 'headers'?",
        )
        .unwrap();
        assert_eq!(parsed.unknown, "header");
        assert_eq!(parsed.suggestion, "headers");
        assert_eq!(
            parsed.context_path,
            vec![
                ContextSegment::Key("root".into()),
                ContextSegment::Key("steps".into()),
                ContextSegment::Index(0),
                ContextSegment::Key("request".into()),
            ]
        );
    }

    #[test]
    fn parse_unknown_field_message_rejects_messages_without_suggestion() {
        assert!(parse_unknown_field_message("Unknown field 'step' at root.").is_none());
        assert!(parse_unknown_field_message("Step 'x' has empty URL").is_none());
        assert!(parse_unknown_field_message("").is_none());
    }

    #[test]
    fn generate_fix_plan_finds_typo_at_root() {
        let source = "name: x\nstep: []\n";
        let d = msg(
            "Unknown field 'step' at root. Did you mean 'steps'?",
            ValidationCode::TarnValidation,
        );
        let plans = generate_fix_plan(source, &[d]);
        assert_eq!(plans.len(), 1);
        let plan = &plans[0];
        assert_eq!(plan.title, "Change 'step' to 'steps'");
        assert!(plan.preferred);
        assert_eq!(plan.edits.len(), 1);
        let edit = &plan.edits[0];
        // `step:` begins at line 2 column 1 in 1-based coordinates.
        assert_eq!(edit.range.line, 2);
        assert_eq!(edit.range.column, 1);
        assert_eq!(edit.length, 4);
        assert_eq!(edit.new_text, "steps");
    }

    #[test]
    fn generate_fix_plan_finds_typo_in_nested_context() {
        let source = "name: x\nsteps:\n  - name: s1\n    request:\n      method: GET\n      url: u\n      header:\n        a: b\n    assert:\n      status: 200\n";
        let d = msg(
            "Unknown field 'header' at root.steps[0].request. Did you mean 'headers'?",
            ValidationCode::TarnValidation,
        );
        let plans = generate_fix_plan(source, &[d]);
        assert_eq!(plans.len(), 1);
        let edit = &plans[0].edits[0];
        // `header:` sits on line 7 at column 7 (six spaces of indent).
        assert_eq!(edit.range.line, 7);
        assert_eq!(edit.range.column, 7);
        assert_eq!(edit.length, 6);
        assert_eq!(edit.new_text, "headers");
    }

    #[test]
    fn generate_fix_plan_empty_diagnostics_returns_empty() {
        let source = "name: x\nsteps: []\n";
        let plans = generate_fix_plan(source, &[]);
        assert!(plans.is_empty());
    }

    #[test]
    fn generate_fix_plan_skips_yaml_syntax_diagnostics() {
        let source = "name: x\nsteps: [\n";
        let d = ValidationMessage {
            severity: Severity::Error,
            code: ValidationCode::YamlSyntax,
            message: "did not find expected ',' or ']'".to_string(),
            location: Some(Location {
                file: "t.tarn.yaml".into(),
                line: 1,
                column: 1,
            }),
        };
        let plans = generate_fix_plan(source, &[d]);
        assert!(plans.is_empty());
    }

    #[test]
    fn generate_fix_plan_skips_messages_without_did_you_mean() {
        let source = "name: x\nsteps: []\n";
        let d = msg("Step 'x' has empty URL", ValidationCode::TarnParse);
        let plans = generate_fix_plan(source, &[d]);
        assert!(plans.is_empty());
    }

    #[test]
    fn generate_fix_plan_declines_when_key_not_found_in_source() {
        // Message says `root.steps[2].request.header`, but the source
        // only has one step — the walker cannot locate the key and
        // returns no plan rather than guessing.
        let source =
            "name: x\nsteps:\n  - name: s1\n    request:\n      method: GET\n      url: u\n";
        let d = msg(
            "Unknown field 'header' at root.steps[2].request. Did you mean 'headers'?",
            ValidationCode::TarnValidation,
        );
        let plans = generate_fix_plan(source, &[d]);
        assert!(plans.is_empty());
    }

    #[test]
    fn generate_fix_plan_handles_multiple_diagnostics_same_file() {
        let source = "name: x\nstep: []\nteardowns: []\n";
        let d1 = msg(
            "Unknown field 'step' at root. Did you mean 'steps'?",
            ValidationCode::TarnValidation,
        );
        let d2 = msg(
            "Unknown field 'teardowns' at root. Did you mean 'teardown'?",
            ValidationCode::TarnValidation,
        );
        let plans = generate_fix_plan(source, &[d1, d2]);
        assert_eq!(plans.len(), 2);
        assert_eq!(plans[0].title, "Change 'step' to 'steps'");
        assert_eq!(plans[1].title, "Change 'teardowns' to 'teardown'");
    }

    #[test]
    fn generate_fix_plan_never_reports_advice_only_plans() {
        // Quick-fix path guarantees every emitted plan carries edits.
        let source = "name: x\nstep: []\n";
        let d = msg(
            "Unknown field 'step' at root. Did you mean 'steps'?",
            ValidationCode::TarnValidation,
        );
        let plans = generate_fix_plan(source, &[d]);
        for plan in &plans {
            assert!(!plan.edits.is_empty());
            assert!(plan.description.is_none());
        }
    }

    #[test]
    fn has_fix_plan_mirrors_generate_fix_plan() {
        let source = "name: x\nstep: []\n";
        let with_fix = msg(
            "Unknown field 'step' at root. Did you mean 'steps'?",
            ValidationCode::TarnValidation,
        );
        let without_fix = msg("Step 'x' has empty URL", ValidationCode::TarnParse);
        assert!(has_fix_plan(source, &with_fix));
        assert!(!has_fix_plan(source, &without_fix));
    }

    #[test]
    fn parse_context_path_splits_dotted_and_indexed_segments() {
        let path = parse_context_path("root.steps[0].request").unwrap();
        assert_eq!(
            path,
            vec![
                ContextSegment::Key("root".into()),
                ContextSegment::Key("steps".into()),
                ContextSegment::Index(0),
                ContextSegment::Key("request".into()),
            ]
        );
    }

    #[test]
    fn parse_context_path_rejects_malformed_input() {
        assert!(parse_context_path("").is_none());
        assert!(parse_context_path("root..steps").is_none());
        assert!(parse_context_path("root[abc]").is_none());
    }
}
