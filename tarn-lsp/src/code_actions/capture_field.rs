//! `Capture as capture variable…` code action (NAZ-304, Phase L3.3).
//!
//! Triggers when the cursor sits on a JSONPath literal that serves as
//! the **key** of an entry inside an `assert.body:` mapping — for
//! example `$.data[0].id` in:
//!
//! ```yaml
//! assert:
//!   body:
//!     "$.data[0].id":
//!       eq: 5
//! ```
//!
//! The refactor inserts a fresh `capture:` entry into the enclosing
//! step so the author can re-use the same JSONPath across subsequent
//! steps without re-typing it. The generated entry uses the extended
//! `{ jsonpath: <path> }` shape so a later expansion into captured-
//! header form does not require re-writing the scaffolding.
//!
//! ## Leaf-name derivation
//!
//! The capture key is derived from the JSONPath via
//! [`crate::code_actions::jsonpath_name::leaf_name`]. See that
//! module for the full rule set. Collisions with existing captures
//! in the same step are resolved by counter-suffixing (`_2`, `_3`, …)
//! exactly the same way `extract_env` handles env key collisions.
//!
//! ## Merge vs create
//!
//! If the step already declares a `capture:` mapping, the new entry
//! is **appended** to it — duplicates are skipped by name, existing
//! entries are never overwritten. If the step has no `capture:`
//! block yet, a fresh one is inserted at the end of the step mapping
//! with the same indentation as the step's other top-level keys.
//!
//! ## URL-id trigger (deferred)
//!
//! The ticket also sketched a heuristic "cursor on a concrete ID
//! inside a request URL" trigger. That variant is **deferred** to a
//! follow-up — it adds a second pure-heuristic path that would need
//! its own collision-free ID detector, and shipping the simpler
//! assert-body trigger keeps NAZ-304 focused. The module layout
//! leaves room for the second provider when it does land.

use lsp_types::{CodeAction, CodeActionKind, Position, Range, TextEdit, Url, WorkspaceEdit};
use std::collections::{HashMap, HashSet};
use tarn::outline::{find_scalar_at_position, outline_from_str, PathSegment, StepOutline};

use crate::code_actions::jsonpath_name::leaf_name;
use crate::code_actions::CodeActionContext;
use crate::identifier::is_valid_identifier;

/// Stable title used in the LSP `CodeAction.title` field.
pub const CAPTURE_FIELD_TITLE: &str = "Capture as capture variable…";

/// Pure renderer for the **capture this field** code action.
///
/// Returns `None` for every soft-fail case: cursor not on a scalar,
/// scalar value is not a JSONPath literal, scalar's path is not
/// inside an `assert.body` mapping key, enclosing step cannot be
/// located, coined name is invalid. Every `None` flows out to the
/// client as "no action offered here".
pub fn capture_field_code_action(
    uri: &Url,
    source: &str,
    range: Range,
    _ctx: &CodeActionContext<'_>,
) -> Option<CodeAction> {
    // 1. Locate the scalar under the cursor / at the selection start.
    let line_one = (range.start.line as usize) + 1;
    let col_one = (range.start.character as usize) + 1;
    let scalar = find_scalar_at_position(source, line_one, col_one)?;

    // 2. The cursor must land on a JSONPath literal. Tarn JSONPath
    //    literals always start with `$.` or `$[`, and we further
    //    filter against the raw value so a stray scalar that happens
    //    to be spelled `$` is excluded.
    let jsonpath = scalar.value.trim();
    if !(jsonpath.starts_with("$.") || jsonpath.starts_with("$[")) {
        return None;
    }

    // 3. The scalar's path must describe an entry **inside** an
    //    assert.body mapping. The walker records the key scalar with
    //    the parent mapping's path, so we expect the last two
    //    segments to be `Key("assert"), Key("body")`.
    if !is_assert_body_key_position(&scalar.path) {
        return None;
    }

    // 4. Find the enclosing step so we know where to insert the
    //    `capture:` block. Every extractable assert.body sits under
    //    one of `steps[N]`, `setup[N]`, `teardown[N]`, or
    //    `tests.<name>.steps[N]`.
    let step_locator = locate_step_from_path(&scalar.path)?;
    let step = resolve_step_outline(source, &step_locator)?;

    // 5. Pick a unique capture name by walking existing captures in
    //    the step.
    let existing = collect_existing_captures_in_step(source, &step);
    let base = leaf_name(jsonpath);
    let chosen = pick_unique_capture_name(&base, &existing);
    if !is_valid_identifier(&chosen) {
        return None;
    }

    // Log to stderr for parity with the extract-env provider.
    eprintln!("tarn-lsp: capture field chose name `{chosen}`");

    // 6. Build the insertion edit. Two shapes:
    //    a) existing `capture:` mapping in the step → append a new
    //       entry at the end of it.
    //    b) no `capture:` mapping → insert a fresh block at the end
    //       of the step mapping with the right indentation.
    let edit = if let Some(block) = find_step_capture_block(source, &step) {
        render_capture_append(&chosen, jsonpath, &block)
    } else {
        render_capture_block_insert(&chosen, jsonpath, source, &step)?
    };

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    changes.insert(uri.clone(), vec![edit]);
    let workspace_edit = WorkspaceEdit {
        changes: Some(changes),
        ..WorkspaceEdit::default()
    };

    Some(CodeAction {
        title: CAPTURE_FIELD_TITLE.to_owned(),
        kind: Some(CodeActionKind::REFACTOR),
        edit: Some(workspace_edit),
        ..CodeAction::default()
    })
}

/// True when the last two segments of `path` are `assert`, `body`.
/// The walker stores the key scalar with the parent mapping's path —
/// so a cursor on `"$.id"` inside `assert.body."$.id"` yields a scalar
/// whose path ends `..., Key("assert"), Key("body")`.
fn is_assert_body_key_position(path: &[PathSegment]) -> bool {
    if path.len() < 2 {
        return false;
    }
    let last = &path[path.len() - 1];
    let prev = &path[path.len() - 2];
    matches!(last, PathSegment::Key(k) if k == "body")
        && matches!(prev, PathSegment::Key(k) if k == "assert")
}

/// Where the enclosing step lives in the outline tree.
#[derive(Debug, Clone)]
enum StepLocator {
    Setup(usize),
    Teardown(usize),
    FlatSteps(usize),
    Test { name: String, index: usize },
}

/// Walk the scalar's path to locate the step that contains it.
///
/// Every valid assert.body path looks like one of:
///
///   * `[setup, N, ...]`
///   * `[teardown, N, ...]`
///   * `[steps, N, ...]`
///   * `[tests, <name>, steps, N, ...]`
///
/// Anything else yields `None`.
fn locate_step_from_path(path: &[PathSegment]) -> Option<StepLocator> {
    match path.first()? {
        PathSegment::Key(k) if k == "setup" => match path.get(1)? {
            PathSegment::Index(i) => Some(StepLocator::Setup(*i)),
            _ => None,
        },
        PathSegment::Key(k) if k == "teardown" => match path.get(1)? {
            PathSegment::Index(i) => Some(StepLocator::Teardown(*i)),
            _ => None,
        },
        PathSegment::Key(k) if k == "steps" => match path.get(1)? {
            PathSegment::Index(i) => Some(StepLocator::FlatSteps(*i)),
            _ => None,
        },
        PathSegment::Key(k) if k == "tests" => {
            let PathSegment::Key(name) = path.get(1)? else {
                return None;
            };
            let PathSegment::Key(steps_key) = path.get(2)? else {
                return None;
            };
            if steps_key != "steps" {
                return None;
            }
            let PathSegment::Index(i) = path.get(3)? else {
                return None;
            };
            Some(StepLocator::Test {
                name: name.clone(),
                index: *i,
            })
        }
        _ => None,
    }
}

/// Look up the step outline entry that corresponds to `loc`. Walks
/// the outline produced by `outline_from_str` and returns the
/// matching [`StepOutline`] clone.
fn resolve_step_outline(source: &str, loc: &StepLocator) -> Option<StepOutline> {
    let outline = outline_from_str("<buf>", source)?;
    match loc {
        StepLocator::Setup(i) => outline.setup.get(*i).cloned(),
        StepLocator::Teardown(i) => outline.teardown.get(*i).cloned(),
        StepLocator::FlatSteps(i) => outline.flat_steps.get(*i).cloned(),
        StepLocator::Test { name, index } => outline
            .tests
            .iter()
            .find(|t| &t.name == name)
            .and_then(|t| t.steps.get(*index))
            .cloned(),
    }
}

/// Information about an existing `capture:` mapping in a step.
#[derive(Debug, Clone)]
struct CaptureBlock {
    /// 0-based line on which to insert a new entry (line immediately
    /// after the last entry in the block).
    insertion_line: u32,
    /// Column width used for child entries of the block.
    child_indent: usize,
    /// Names already declared in the block — used for collision
    /// detection.
    existing_names: HashSet<String>,
}

/// Scan the step body for a `capture:` mapping. Returns `None` when
/// the step has no capture block yet.
fn find_step_capture_block(source: &str, step: &StepOutline) -> Option<CaptureBlock> {
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    // Step.range is 1-based inclusive; convert to 0-based line
    // indices for slicing.
    let start_line = step.range.start_line.saturating_sub(1);
    let end_line = step
        .range
        .end_line
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1));

    // The step's top-level keys sit at the same indent as `name:`;
    // find it so we know what "top-level key line" looks like.
    let top_indent = step_top_level_indent(source, step)?;

    // Find `capture:` at exactly `top_indent` within the step's
    // range.
    let capture_line_idx = (start_line..=end_line).find(|idx| {
        let line = lines.get(*idx).map(|l| l.trim_end_matches(['\n', '\r']));
        if let Some(line) = line {
            let indent = line.len() - line.trim_start().len();
            indent == top_indent && line.trim_start().starts_with("capture:")
        } else {
            false
        }
    })?;

    // Walk forward from the capture line to find the block's child
    // indent and the final line that still belongs to the block.
    // The insertion point must sit **after** the whole subtree of
    // the last entry (e.g. after the `jsonpath:` sub-key under
    // `token:`), not merely after the key line itself — otherwise
    // the new entry would land inside the previous entry's value.
    let mut last_block_idx = capture_line_idx;
    let mut child_indent: Option<usize> = None;
    let mut existing: HashSet<String> = HashSet::new();
    for idx in (capture_line_idx + 1)..=end_line {
        let Some(line) = lines.get(idx) else {
            break;
        };
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.trim().is_empty() {
            continue;
        }
        let indent = trimmed.len() - trimmed.trim_start().len();
        if indent <= top_indent {
            break;
        }
        if child_indent.is_none() {
            child_indent = Some(indent);
        }
        // Every non-blank line that is still deeper than
        // `top_indent` belongs to the capture block — either as a
        // direct child at `child_indent` or as a nested value
        // deeper than that. Track the deepest line index so the
        // insertion lands after the complete block.
        last_block_idx = idx;
        if Some(indent) == child_indent {
            if let Some(name) = parse_leading_key_name(trimmed.trim_start()) {
                existing.insert(name);
            }
        }
    }
    let child_indent = child_indent?;
    let last_entry_idx = last_block_idx;
    Some(CaptureBlock {
        insertion_line: (last_entry_idx as u32) + 1,
        child_indent,
        existing_names: existing,
    })
}

/// Parse the leading key name from a single YAML line like
/// `token: $.id` or `token:\n  jsonpath: $.id`. Returns `None` for
/// sequence entries, comments, and everything else that cannot start
/// with a key name.
fn parse_leading_key_name(line: &str) -> Option<String> {
    if line.starts_with('#') || line.starts_with('-') {
        return None;
    }
    let colon = line.find(':')?;
    let key = line[..colon].trim();
    if key.is_empty() {
        return None;
    }
    // Strip quotes around the key if present — YAML allows quoted
    // keys (`"weird-key":`), but capture keys should be identifier-
    // shaped in practice.
    let stripped = key
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| key.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(key);
    Some(stripped.to_owned())
}

/// Collect the set of already-declared capture names for collision
/// detection. Returns an empty set when the step has no capture block.
fn collect_existing_captures_in_step(source: &str, step: &StepOutline) -> HashSet<String> {
    find_step_capture_block(source, step)
        .map(|b| b.existing_names)
        .unwrap_or_default()
}

/// Coin a unique capture name from `base`. Returns `base` when it is
/// free; otherwise counter-suffixes with `_2`, `_3`, … exactly the
/// same shape the env-key collision resolver uses.
pub fn pick_unique_capture_name(base: &str, existing: &HashSet<String>) -> String {
    if !existing.contains(base) {
        return base.to_owned();
    }
    for n in 2..1000u32 {
        let candidate = format!("{base}_{n}");
        if !existing.contains(&candidate) {
            return candidate;
        }
    }
    base.to_owned()
}

/// Determine the column width used for the step mapping's top-level
/// keys (`name:`, `request:`, `assert:`, etc.).
///
/// Walks forward from the step's first line and returns the indent
/// of the first non-blank line whose column is greater than the
/// step's opening column. That line is always a sibling key of
/// `name:` in well-formed YAML.
fn step_top_level_indent(source: &str, step: &StepOutline) -> Option<usize> {
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let start_line = step.range.start_line.saturating_sub(1);
    let end_line = step
        .range
        .end_line
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1));
    // The step's first line may start with `- name:`; its top-level
    // key indent is the column of the `n` in `name`. Subsequent
    // sibling keys use the same indent.
    let first_line = lines.get(start_line)?.trim_end_matches(['\n', '\r']);
    let stripped = first_line.trim_start();
    if let Some(after_dash) = stripped.strip_prefix('-') {
        let dash_col = first_line.len() - stripped.len();
        // After the dash there is whitespace then the key; its
        // column is `dash_col + 1 + leading_ws`.
        let leading_ws = after_dash.len() - after_dash.trim_start().len();
        return Some(dash_col + 1 + leading_ws);
    }
    // Fallback: scan for the first non-blank line after the start
    // and use its indent. Covers the rare case where the step's
    // first line is blank or a comment.
    for idx in start_line..=end_line {
        let line = lines.get(idx)?.trim_end_matches(['\n', '\r']);
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        return Some(line.len() - trimmed.len());
    }
    None
}

/// Build a [`TextEdit`] that appends a new entry to an existing
/// `capture:` block at `insertion_line`.
fn render_capture_append(chosen: &str, jsonpath: &str, block: &CaptureBlock) -> TextEdit {
    TextEdit {
        range: Range::new(
            Position::new(block.insertion_line, 0),
            Position::new(block.insertion_line, 0),
        ),
        new_text: format!(
            "{indent}{chosen}:\n{inner_indent}jsonpath: {value}\n",
            indent = " ".repeat(block.child_indent),
            inner_indent = " ".repeat(block.child_indent + 2),
            chosen = chosen,
            value = double_quote_if_needed(jsonpath),
        ),
    }
}

/// Build a [`TextEdit`] that inserts a brand new `capture:` block at
/// the very end of the step mapping.
fn render_capture_block_insert(
    chosen: &str,
    jsonpath: &str,
    source: &str,
    step: &StepOutline,
) -> Option<TextEdit> {
    let top_indent = step_top_level_indent(source, step)?;
    // Insert right after the step's last line so the block lands as
    // a new sibling key inside the step mapping.
    let insertion_line = step.range.end_line as u32;
    let child_indent = top_indent + 2;
    let grand_indent = child_indent + 2;
    let text = format!(
        "{outer}capture:\n{inner}{chosen}:\n{grand}jsonpath: {value}\n",
        outer = " ".repeat(top_indent),
        inner = " ".repeat(child_indent),
        grand = " ".repeat(grand_indent),
        chosen = chosen,
        value = double_quote_if_needed(jsonpath),
    );
    Some(TextEdit {
        range: Range::new(
            Position::new(insertion_line, 0),
            Position::new(insertion_line, 0),
        ),
        new_text: text,
    })
}

/// Quote a JSONPath literal when it contains characters YAML would
/// swallow in a plain scalar (brackets, stars, spaces). Bracket
/// notation JSONPaths always need quoting; dot notation usually does
/// not.
fn double_quote_if_needed(s: &str) -> String {
    let needs = s
        .chars()
        .any(|c| matches!(c, '[' | ']' | '*' | ' ' | '{' | '}' | ',' | '&' | '!'));
    if needs {
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        s.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_actions::CodeActionContext;
    use lsp_types::CodeActionContext as LspCodeActionContext;
    use std::collections::BTreeMap;

    fn uri() -> Url {
        Url::parse("file:///tmp/fixture.tarn.yaml").unwrap()
    }

    fn empty_env() -> BTreeMap<String, tarn::env::EnvEntry> {
        BTreeMap::new()
    }

    fn empty_lsp_ctx() -> LspCodeActionContext {
        LspCodeActionContext {
            diagnostics: Vec::new(),
            only: None,
            trigger_kind: None,
        }
    }

    fn ctx_for<'a>(
        uri: &'a Url,
        source: &'a str,
        env: &'a BTreeMap<String, tarn::env::EnvEntry>,
        lsp_ctx: &'a LspCodeActionContext,
    ) -> CodeActionContext<'a> {
        CodeActionContext {
            uri,
            source,
            env,
            lsp_ctx,
            recorded_response_reader: None,
        }
    }

    fn cursor(line: u32, col: u32) -> Range {
        Range::new(Position::new(line, col), Position::new(line, col))
    }

    // ---------- capture_field_code_action ----------

    #[test]
    fn capture_field_happy_path_on_assert_body_key_creates_block() {
        let source = "name: fixture\nsteps:\n  - name: s1\n    request:\n      method: GET\n      url: http://x/\n    assert:\n      body:\n        \"$.data[0].id\":\n          eq: 5\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let ctx = ctx_for(&uri, source, &env, &lsp_ctx);
        // Cursor on the JSONPath key on line 9 (0-based 8).
        let range = cursor(8, 12);
        let action = capture_field_code_action(&uri, source, range, &ctx).expect("action");
        assert_eq!(action.title, CAPTURE_FIELD_TITLE);
        assert_eq!(action.kind, Some(CodeActionKind::REFACTOR));
        let edit = action.edit.expect("edit");
        let changes = edit.changes.expect("changes");
        let edits = changes.get(&uri).expect("edits");
        assert_eq!(edits.len(), 1);
        let text = &edits[0].new_text;
        assert!(
            text.contains("capture:") && text.contains("id:") && text.contains("jsonpath:"),
            "expected fresh capture block, got: {text}"
        );
        assert!(
            text.contains("\"$.data[0].id\""),
            "expected quoted JSONPath value in {text}"
        );
    }

    #[test]
    fn capture_field_leaf_name_derivation_matches_last_segment() {
        let source = "steps:\n  - name: s1\n    request:\n      method: GET\n      url: http://x/\n    assert:\n      body:\n        $.user.email:\n          type: string\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let ctx = ctx_for(&uri, source, &env, &lsp_ctx);
        let range = cursor(7, 10);
        let action = capture_field_code_action(&uri, source, range, &ctx).expect("action");
        let edit = action.edit.expect("edit");
        let changes = edit.changes.expect("changes");
        let edits = changes.get(&uri).expect("edits");
        let text = &edits[0].new_text;
        assert!(
            text.contains("email:"),
            "expected derived leaf name `email`, got: {text}"
        );
    }

    #[test]
    fn capture_field_collision_suffixes_with_2() {
        // Existing `capture: { id: $.old }` in the step means the
        // new `$.id` key must pick `id_2`.
        let source = "steps:\n  - name: s1\n    request:\n      method: GET\n      url: http://x/\n    capture:\n      id:\n        jsonpath: $.old\n    assert:\n      body:\n        $.id:\n          eq: 5\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let ctx = ctx_for(&uri, source, &env, &lsp_ctx);
        // Cursor on `$.id` on line 11 (0-based 10).
        let range = cursor(10, 10);
        let action = capture_field_code_action(&uri, source, range, &ctx).expect("action");
        let edit = action.edit.expect("edit");
        let changes = edit.changes.expect("changes");
        let edits = changes.get(&uri).expect("edits");
        let text = &edits[0].new_text;
        assert!(
            text.contains("id_2:"),
            "expected collision suffix id_2, got: {text}"
        );
    }

    #[test]
    fn capture_field_merges_into_existing_capture_block() {
        let source = "steps:\n  - name: s1\n    request:\n      method: GET\n      url: http://x/\n    capture:\n      token:\n        jsonpath: $.token\n    assert:\n      body:\n        $.user.email:\n          type: string\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let ctx = ctx_for(&uri, source, &env, &lsp_ctx);
        let range = cursor(10, 10);
        let action = capture_field_code_action(&uri, source, range, &ctx).expect("action");
        let edit = action.edit.expect("edit");
        let changes = edit.changes.expect("changes");
        let edits = changes.get(&uri).expect("edits");
        assert_eq!(edits.len(), 1);
        let text = &edits[0].new_text;
        // Must be an append (no new `capture:` key)
        assert!(
            !text.contains("capture:"),
            "expected merge into existing block, got a fresh block: {text}"
        );
        assert!(text.contains("email:"));
        assert!(text.contains("jsonpath: $.user.email"));
        // The edit should be inserted right after the last capture
        // entry (line 8, 0-based) — before the `assert:` sibling.
        let edit_line = edits[0].range.start.line;
        assert_eq!(edit_line, 8, "expected insertion after existing capture");
    }

    #[test]
    fn capture_field_declines_when_cursor_not_on_jsonpath() {
        let source = "steps:\n  - name: s1\n    request:\n      method: GET\n      url: http://x/\n    assert:\n      body:\n        $.id:\n          eq: 5\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let ctx = ctx_for(&uri, source, &env, &lsp_ctx);
        // Cursor on the step `name` value `s1` — definitely not a
        // JSONPath literal.
        let range = cursor(1, 12);
        assert!(capture_field_code_action(&uri, source, range, &ctx).is_none());
    }

    #[test]
    fn capture_field_declines_on_status_assertion() {
        // `assert.status` is a number, not a JSONPath. The scalar
        // value `200` is not a JSONPath literal so the renderer
        // must decline.
        let source = "steps:\n  - name: s1\n    request:\n      method: GET\n      url: http://x/\n    assert:\n      status: 200\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let ctx = ctx_for(&uri, source, &env, &lsp_ctx);
        let range = cursor(6, 15);
        assert!(capture_field_code_action(&uri, source, range, &ctx).is_none());
    }

    #[test]
    fn capture_field_declines_on_capture_block_key() {
        // Cursor on an existing capture key. Its parent path is
        // `[steps, 0, capture]`, not `[..., assert, body]`, so the
        // renderer must decline — "capture an existing capture"
        // would be an identity op.
        let source =
            "steps:\n  - name: s1\n    capture:\n      token:\n        jsonpath: $.token\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let ctx = ctx_for(&uri, source, &env, &lsp_ctx);
        // Cursor on the jsonpath value `$.token` under a capture
        // mapping — path ends `..., capture, token, jsonpath`, not
        // `..., assert, body`.
        let range = cursor(4, 18);
        assert!(capture_field_code_action(&uri, source, range, &ctx).is_none());
    }

    #[test]
    fn capture_field_triggers_under_named_test_group() {
        let source = "tests:\n  main:\n    steps:\n      - name: s1\n        request:\n          method: GET\n          url: http://x/\n        assert:\n          body:\n            $.id:\n              eq: 5\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let ctx = ctx_for(&uri, source, &env, &lsp_ctx);
        let range = cursor(9, 14);
        let action = capture_field_code_action(&uri, source, range, &ctx).expect("action");
        let edit = action.edit.expect("edit");
        let changes = edit.changes.expect("changes");
        let edits = changes.get(&uri).expect("edits");
        let text = &edits[0].new_text;
        assert!(text.contains("capture:"));
        assert!(text.contains("id:"));
    }

    // ---------- pick_unique_capture_name ----------

    #[test]
    fn pick_unique_capture_name_returns_base_when_free() {
        let existing: HashSet<String> = HashSet::new();
        assert_eq!(pick_unique_capture_name("id", &existing), "id");
    }

    #[test]
    fn pick_unique_capture_name_suffixes_with_2_on_collision() {
        let existing: HashSet<String> = ["id".to_owned()].into_iter().collect();
        assert_eq!(pick_unique_capture_name("id", &existing), "id_2");
    }

    #[test]
    fn pick_unique_capture_name_walks_chain() {
        let existing: HashSet<String> = ["id".to_owned(), "id_2".to_owned()].into_iter().collect();
        assert_eq!(pick_unique_capture_name("id", &existing), "id_3");
    }
}
