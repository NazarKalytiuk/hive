//! `Scaffold assert.body from last response` code action
//! (NAZ-304, Phase L3.3).
//!
//! Triggers when the cursor sits inside a `request:` block of a
//! named step **and** the `CodeActionContext` carries a
//! [`RecordedResponseSource`] that returns a JSON object for that
//! step. The refactor walks the top-level fields of the recorded
//! response and inserts an `assert.body` block pre-populated with
//! type assertions, one entry per field.
//!
//! The action is deliberately conservative:
//!
//!   * Only top-level fields are emitted. Nested `$.user.email` or
//!     array element paths are outside scope — the user can expand
//!     deeper manually. That matches the ticket's scope guidance and
//!     keeps the generated YAML readable.
//!   * Only JSON **objects** are scaffolded. A recorded response
//!     whose root is an array, a scalar, or `null` offers no
//!     top-level fields and the action declines.
//!   * Existing `assert.body` entries are merged — the renderer
//!     never overwrites a path the user already asserted against
//!     and never emits an edit for a step whose recorded fields
//!     are already all covered.
//!
//! ## Sidecar convention
//!
//! See [`crate::code_actions::response_source`] for the on-disk
//! layout. Nothing writes these files yet — the renderer ships the
//! read side so the refactor is ready as soon as the writer lands
//! in a separate ticket. Until then the disk reader always returns
//! `None` and the action simply does not trigger, which is the
//! documented graceful degradation.

use lsp_types::{CodeAction, CodeActionKind, Position, Range, TextEdit, Url, WorkspaceEdit};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use tarn::outline::{find_scalar_at_position, outline_from_str, PathSegment, StepOutline};

use crate::code_actions::jsonpath_name::infer_type;
use crate::code_actions::CodeActionContext;

/// Stable title used in the LSP `CodeAction.title` field.
pub const SCAFFOLD_ASSERT_TITLE: &str = "Scaffold assert.body from last response";

/// Pure renderer for the **scaffold assert from last response** code
/// action.
///
/// Returns `None` for every soft-fail case: no reader wired, reader
/// returns `None`, step unnamed, recorded response is not an object,
/// every top-level field is already asserted, or the buffer cannot
/// be parsed far enough to locate an enclosing step.
pub fn scaffold_assert_code_action(
    uri: &Url,
    source: &str,
    range: Range,
    ctx: &CodeActionContext<'_>,
) -> Option<CodeAction> {
    // 1. The action only fires when the LSP was handed a reader.
    //    Without one there is no way to look up recorded data, so
    //    the action gracefully declines.
    let reader = ctx.recorded_response_reader.as_ref()?;

    // 2. Find the scalar the cursor is on. We only need its path —
    //    not its value — to decide whether the cursor is inside a
    //    `request:` block.
    let line_one = (range.start.line as usize) + 1;
    let col_one = (range.start.character as usize) + 1;
    let scalar = find_scalar_at_position(source, line_one, col_one)?;
    if !path_is_inside_request(&scalar.path) {
        return None;
    }

    // 3. Locate the enclosing step in the outline so we know where
    //    to insert the `assert.body` block and can pass the step's
    //    display name to the reader.
    let step_loc = locate_step_from_path(&scalar.path)?;
    let outline = outline_from_str("<buf>", source)?;
    let (step, test_name) = resolve_step_and_test(&outline, &step_loc)?;

    // 4. The reader needs a real step name so it can point at the
    //    right sidecar file. An unnamed step would force a
    //    synthetic `<step N>` slug that the writer cannot
    //    reproduce, so the action declines for unnamed steps.
    if step.name.starts_with("<step ") {
        return None;
    }

    // 5. Ask the reader. A `None` return means "no recording
    //    available", which is a documented no-op branch.
    let file_path = uri
        .to_file_path()
        .ok()
        .unwrap_or_else(|| PathBuf::from(uri.path()));
    let value = reader.read(&file_path, &test_name, &step.name)?;

    // 6. Only JSON objects are in scope — top-level fields only.
    let obj = match value {
        serde_json::Value::Object(map) => map,
        _ => return None,
    };
    if obj.is_empty() {
        return None;
    }

    // 7. Compute existing assert.body entries for merge. If every
    //    top-level field is already asserted there is nothing to
    //    emit, so the action declines.
    let existing_entries = collect_existing_assert_body_entries(source, &step);
    let mut new_entries: BTreeMap<String, String> = BTreeMap::new();
    for (key, val) in obj.iter() {
        let path = format!("$.{key}");
        if existing_entries.contains(&path) {
            continue;
        }
        new_entries.insert(path, infer_type(val).to_owned());
    }
    if new_entries.is_empty() {
        return None;
    }

    // 8. Build the edit: merge into an existing assert.body block or
    //    insert a fresh `assert.body` section at the end of the
    //    step.
    let edit = if let Some(block) = find_assert_body_block(source, &step) {
        render_assert_body_append(&new_entries, &block)
    } else {
        render_assert_body_insert(&new_entries, source, &step)?
    };

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    changes.insert(uri.clone(), vec![edit]);
    let workspace_edit = WorkspaceEdit {
        changes: Some(changes),
        ..WorkspaceEdit::default()
    };

    eprintln!(
        "tarn-lsp: scaffold-assert populated {n} assert.body entries for step `{name}`",
        n = new_entries.len(),
        name = step.name,
    );

    Some(CodeAction {
        title: SCAFFOLD_ASSERT_TITLE.to_owned(),
        kind: Some(CodeActionKind::REFACTOR),
        edit: Some(workspace_edit),
        ..CodeAction::default()
    })
}

/// True when `path` contains a `request` segment anywhere after the
/// step index. The cursor only has to be inside the request block —
/// it does not need to point at a specific leaf.
fn path_is_inside_request(path: &[PathSegment]) -> bool {
    path.iter()
        .any(|seg| matches!(seg, PathSegment::Key(k) if k == "request"))
        && locate_step_from_path(path).is_some()
}

/// Shared step-locator copied structurally from
/// [`crate::code_actions::capture_field`]. The two providers deal
/// with different trigger conditions but both need exactly the same
/// "which step am I in" answer, so the logic is duplicated rather
/// than shared via a third module — it is three match arms and
/// hiding them would obscure the caller's intent.
#[derive(Debug, Clone)]
enum StepLocator {
    Setup(usize),
    Teardown(usize),
    FlatSteps(usize),
    Test { name: String, index: usize },
}

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

/// Resolve a [`StepLocator`] to the matching [`StepOutline`] plus
/// the "test name" string the reader expects. Setup / teardown /
/// flat steps use the sentinel strings `"setup"` / `"teardown"` /
/// `"<flat>"` so the sidecar path can still be constructed in a
/// deterministic way.
fn resolve_step_and_test(
    outline: &tarn::outline::Outline,
    loc: &StepLocator,
) -> Option<(StepOutline, String)> {
    match loc {
        StepLocator::Setup(i) => outline
            .setup
            .get(*i)
            .cloned()
            .map(|s| (s, "setup".to_owned())),
        StepLocator::Teardown(i) => outline
            .teardown
            .get(*i)
            .cloned()
            .map(|s| (s, "teardown".to_owned())),
        StepLocator::FlatSteps(i) => outline
            .flat_steps
            .get(*i)
            .cloned()
            .map(|s| (s, "<flat>".to_owned())),
        StepLocator::Test { name, index } => outline
            .tests
            .iter()
            .find(|t| &t.name == name)
            .and_then(|t| t.steps.get(*index).cloned().map(|s| (s, name.clone()))),
    }
}

/// Information about an existing `assert.body:` mapping inside a
/// step.
#[derive(Debug, Clone)]
struct AssertBodyBlock {
    /// 0-based line on which to insert a new entry (line after the
    /// last existing entry).
    insertion_line: u32,
    /// Column width used for child entries of the `body:` mapping.
    child_indent: usize,
}

/// Collect every JSONPath key already declared under the step's
/// `assert.body:` mapping. Returns an empty set when the step has
/// no assert block yet.
fn collect_existing_assert_body_entries(source: &str, step: &StepOutline) -> HashSet<String> {
    let Some(block_lines) = find_assert_body_key_lines(source, step) else {
        return HashSet::new();
    };
    let mut out = HashSet::new();
    for line in block_lines {
        // Parse the key portion. Quoted keys are common for JSONPaths
        // (brackets / dots trip up plain YAML).
        if let Some(key) = parse_leading_key_name(line.trim_start()) {
            out.insert(key);
        }
    }
    out
}

/// Locate an existing `assert.body:` mapping inside the step.
///
/// Returns an [`AssertBodyBlock`] describing where to insert new
/// entries, the indent used by the existing children, and the
/// indent of the `body:` key itself. Returns `None` only when the
/// step has no `assert.body:` at all — an `assert.body:` with zero
/// children still yields a usable block (the child indent is
/// synthesised from `body_indent + 2`).
fn find_assert_body_block(source: &str, step: &StepOutline) -> Option<AssertBodyBlock> {
    let (body_line_idx, body_indent, _top_indent) = find_body_line_idx(source, step)?;
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let end_line = step
        .range
        .end_line
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1));

    let mut last_block_idx = body_line_idx;
    let mut child_indent: Option<usize> = None;
    for idx in (body_line_idx + 1)..=end_line {
        let Some(line) = lines.get(idx) else {
            break;
        };
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.trim().is_empty() {
            continue;
        }
        let indent = trimmed.len() - trimmed.trim_start().len();
        if indent <= body_indent {
            break;
        }
        if child_indent.is_none() {
            child_indent = Some(indent);
        }
        // Track the last line that still belongs to the block so
        // the insertion point lands after the full subtree of the
        // previous entry, not between its key and its own children.
        last_block_idx = idx;
    }
    let child_indent = child_indent.unwrap_or(body_indent + 2);
    Some(AssertBodyBlock {
        insertion_line: (last_block_idx as u32) + 1,
        child_indent,
    })
}

/// Collect the raw "key line" text of every top-level entry under
/// `assert.body:` — used by [`collect_existing_assert_body_entries`].
fn find_assert_body_key_lines(source: &str, step: &StepOutline) -> Option<Vec<String>> {
    let (body_line_idx, body_indent, _top_indent) = find_body_line_idx(source, step)?;
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let end_line = step
        .range
        .end_line
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1));

    let mut out = Vec::new();
    let mut child_indent: Option<usize> = None;
    for idx in (body_line_idx + 1)..=end_line {
        let Some(line) = lines.get(idx) else {
            break;
        };
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.trim().is_empty() {
            continue;
        }
        let indent = trimmed.len() - trimmed.trim_start().len();
        if indent <= body_indent {
            break;
        }
        if child_indent.is_none() {
            child_indent = Some(indent);
        }
        if Some(indent) == child_indent {
            out.push(trimmed.to_owned());
        }
    }
    Some(out)
}

/// Walk the step body for the `body:` key inside an `assert:` block.
/// Returns `(line_idx, body_indent, step_top_indent)` when found.
fn find_body_line_idx(source: &str, step: &StepOutline) -> Option<(usize, usize, usize)> {
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let start_line = step.range.start_line.saturating_sub(1);
    let end_line = step
        .range
        .end_line
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1));
    let top_indent = step_top_level_indent(source, step)?;

    // Find `assert:` at exactly `top_indent`.
    let assert_line_idx = (start_line..=end_line).find(|idx| {
        let line = lines.get(*idx).map(|l| l.trim_end_matches(['\n', '\r']));
        if let Some(line) = line {
            let indent = line.len() - line.trim_start().len();
            indent == top_indent && line.trim_start().starts_with("assert:")
        } else {
            false
        }
    })?;

    // Walk forward to find `body:` at `top_indent + 2` (or whatever
    // the first child indent of the assert block turns out to be).
    let mut assert_child_indent: Option<usize> = None;
    for idx in (assert_line_idx + 1)..=end_line {
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
        if assert_child_indent.is_none() {
            assert_child_indent = Some(indent);
        }
        if Some(indent) == assert_child_indent && trimmed.trim_start().starts_with("body:") {
            return Some((idx, indent, top_indent));
        }
    }
    None
}

fn parse_leading_key_name(line: &str) -> Option<String> {
    if line.starts_with('#') || line.starts_with('-') {
        return None;
    }
    let colon = line.find(':')?;
    let key = line[..colon].trim();
    if key.is_empty() {
        return None;
    }
    let stripped = key
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| key.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(key);
    Some(stripped.to_owned())
}

/// Copy of the step-top-indent helper from `capture_field`. The two
/// providers share a trivial algorithm; duplicating it keeps the
/// two modules independently auditable.
fn step_top_level_indent(source: &str, step: &StepOutline) -> Option<usize> {
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let start_line = step.range.start_line.saturating_sub(1);
    let end_line = step
        .range
        .end_line
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1));
    let first_line = lines.get(start_line)?.trim_end_matches(['\n', '\r']);
    let stripped = first_line.trim_start();
    if let Some(after_dash) = stripped.strip_prefix('-') {
        let dash_col = first_line.len() - stripped.len();
        let leading_ws = after_dash.len() - after_dash.trim_start().len();
        return Some(dash_col + 1 + leading_ws);
    }
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

/// Render an append-style edit that slots new entries into an
/// existing `assert.body:` mapping.
fn render_assert_body_append(
    entries: &BTreeMap<String, String>,
    block: &AssertBodyBlock,
) -> TextEdit {
    let mut body = String::new();
    for (path, ty) in entries {
        body.push_str(&format!(
            "{indent}{key}:\n{inner}type: {ty}\n",
            indent = " ".repeat(block.child_indent),
            inner = " ".repeat(block.child_indent + 2),
            key = double_quote_jsonpath(path),
        ));
    }
    TextEdit {
        range: Range::new(
            Position::new(block.insertion_line, 0),
            Position::new(block.insertion_line, 0),
        ),
        new_text: body,
    }
}

/// Render a fresh `assert.body:` block at the end of the step.
///
/// Handles three cases:
///
///   * step has `assert:` but no `body:` child → append `body:` to
///     the existing assert block.
///   * step has neither `assert:` nor `body:` → append a brand new
///     `assert:\n  body:\n    ...` block at the end of the step.
fn render_assert_body_insert(
    entries: &BTreeMap<String, String>,
    source: &str,
    step: &StepOutline,
) -> Option<TextEdit> {
    let top_indent = step_top_level_indent(source, step)?;
    let insertion_line = step.range.end_line as u32;

    // Detect whether an `assert:` block already exists.
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let start_line = step.range.start_line.saturating_sub(1);
    let end_line = step
        .range
        .end_line
        .saturating_sub(1)
        .min(lines.len().saturating_sub(1));
    let mut assert_line_idx: Option<usize> = None;
    for idx in start_line..=end_line {
        let Some(line) = lines.get(idx) else {
            break;
        };
        let trimmed = line.trim_end_matches(['\n', '\r']);
        let indent = trimmed.len() - trimmed.trim_start().len();
        if indent == top_indent && trimmed.trim_start().starts_with("assert:") {
            assert_line_idx = Some(idx);
            break;
        }
    }

    if let Some(assert_idx) = assert_line_idx {
        // Step has assert but no body. Append a `body:` sub-block at
        // the end of the assert mapping. Walk forward to find the
        // last line that belongs to the assert block.
        let mut last_assert_line = assert_idx;
        let mut assert_child_indent: Option<usize> = None;
        for idx in (assert_idx + 1)..=end_line {
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
            if assert_child_indent.is_none() {
                assert_child_indent = Some(indent);
            }
            last_assert_line = idx;
        }
        let body_indent = assert_child_indent.unwrap_or(top_indent + 2);
        let child_indent = body_indent + 2;
        let inner_indent = child_indent + 2;
        let mut text = format!("{pad}body:\n", pad = " ".repeat(body_indent));
        for (path, ty) in entries {
            text.push_str(&format!(
                "{outer}{key}:\n{inner}type: {ty}\n",
                outer = " ".repeat(child_indent),
                inner = " ".repeat(inner_indent),
                key = double_quote_jsonpath(path),
            ));
        }
        return Some(TextEdit {
            range: Range::new(
                Position::new((last_assert_line as u32) + 1, 0),
                Position::new((last_assert_line as u32) + 1, 0),
            ),
            new_text: text,
        });
    }

    // Step has neither assert: nor body:. Insert a fresh
    // `assert:\n  body:\n    ...` block at the end of the step.
    let body_indent = top_indent + 2;
    let child_indent = body_indent + 2;
    let inner_indent = child_indent + 2;
    let mut text = format!(
        "{outer}assert:\n{body}body:\n",
        outer = " ".repeat(top_indent),
        body = " ".repeat(body_indent),
    );
    for (path, ty) in entries {
        text.push_str(&format!(
            "{outer}{key}:\n{inner}type: {ty}\n",
            outer = " ".repeat(child_indent),
            inner = " ".repeat(inner_indent),
            key = double_quote_jsonpath(path),
        ));
    }
    Some(TextEdit {
        range: Range::new(
            Position::new(insertion_line, 0),
            Position::new(insertion_line, 0),
        ),
        new_text: text,
    })
}

/// JSONPath keys almost always need double quoting because they
/// contain `.`, `[`, and `*` — YAML would mis-parse several of those
/// as flow-style delimiters. We quote unconditionally for the
/// scaffold so the output never surprises the user.
fn double_quote_jsonpath(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_actions::response_source::InMemoryResponseSource;
    use crate::code_actions::CodeActionContext;
    use lsp_types::CodeActionContext as LspCodeActionContext;
    use std::collections::BTreeMap;
    use std::sync::Arc;

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

    fn ctx_with_reader<'a>(
        uri: &'a Url,
        source: &'a str,
        env: &'a BTreeMap<String, tarn::env::EnvEntry>,
        lsp_ctx: &'a LspCodeActionContext,
        value: serde_json::Value,
    ) -> CodeActionContext<'a> {
        CodeActionContext {
            uri,
            source,
            env,
            lsp_ctx,
            recorded_response_reader: Some(Arc::new(InMemoryResponseSource::new(value))),
        }
    }

    fn ctx_without_reader<'a>(
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

    fn ctx_with_empty_reader<'a>(
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
            recorded_response_reader: Some(Arc::new(InMemoryResponseSource::empty())),
        }
    }

    fn cursor(line: u32, col: u32) -> Range {
        Range::new(Position::new(line, col), Position::new(line, col))
    }

    // ---------- scaffold_assert_code_action ----------

    #[test]
    fn scaffold_assert_happy_path_three_fields_creates_block() {
        let source = "steps:\n  - name: get_user\n    request:\n      method: GET\n      url: http://example.com/user\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let value = serde_json::json!({
            "id": 1,
            "name": "Alice",
            "tags": ["admin"]
        });
        let ctx = ctx_with_reader(&uri, source, &env, &lsp_ctx, value);
        // Cursor inside the `url:` value on line 5 (0-based 4).
        let range = cursor(4, 15);
        let action = scaffold_assert_code_action(&uri, source, range, &ctx).expect("action");
        assert_eq!(action.title, SCAFFOLD_ASSERT_TITLE);
        assert_eq!(action.kind, Some(CodeActionKind::REFACTOR));
        let edit = action.edit.expect("edit");
        let changes = edit.changes.expect("changes");
        let edits = changes.get(&uri).expect("edits");
        let text = &edits[0].new_text;
        assert!(text.contains("assert:"), "got: {text}");
        assert!(text.contains("body:"), "got: {text}");
        assert!(text.contains("\"$.id\":"), "got: {text}");
        assert!(text.contains("type: number"), "got: {text}");
        assert!(text.contains("\"$.name\":"), "got: {text}");
        assert!(text.contains("type: string"), "got: {text}");
        assert!(text.contains("\"$.tags\":"), "got: {text}");
        assert!(text.contains("type: array"), "got: {text}");
    }

    #[test]
    fn scaffold_assert_handles_mixed_types_number_string_array_bool_null() {
        let source = "steps:\n  - name: s\n    request:\n      method: GET\n      url: http://x/\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let value = serde_json::json!({
            "count": 42,
            "title": "hello",
            "items": [],
            "active": true,
            "deleted_at": serde_json::Value::Null,
        });
        let ctx = ctx_with_reader(&uri, source, &env, &lsp_ctx, value);
        let range = cursor(4, 15);
        let action = scaffold_assert_code_action(&uri, source, range, &ctx).expect("action");
        let edit = action.edit.expect("edit");
        let changes = edit.changes.expect("changes");
        let edits = changes.get(&uri).expect("edits");
        let text = &edits[0].new_text;
        assert!(text.contains("type: number"));
        assert!(text.contains("type: string"));
        assert!(text.contains("type: array"));
        assert!(text.contains("type: boolean"));
        assert!(text.contains("type: null"));
    }

    #[test]
    fn scaffold_assert_declines_without_reader() {
        let source = "steps:\n  - name: s\n    request:\n      method: GET\n      url: http://x/\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let ctx = ctx_without_reader(&uri, source, &env, &lsp_ctx);
        let range = cursor(4, 15);
        assert!(scaffold_assert_code_action(&uri, source, range, &ctx).is_none());
    }

    #[test]
    fn scaffold_assert_declines_when_reader_returns_none() {
        let source = "steps:\n  - name: s\n    request:\n      method: GET\n      url: http://x/\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let ctx = ctx_with_empty_reader(&uri, source, &env, &lsp_ctx);
        let range = cursor(4, 15);
        assert!(scaffold_assert_code_action(&uri, source, range, &ctx).is_none());
    }

    #[test]
    fn scaffold_assert_declines_outside_request_block() {
        let source = "steps:\n  - name: s\n    request:\n      method: GET\n      url: http://x/\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let value = serde_json::json!({"id": 1});
        let ctx = ctx_with_reader(&uri, source, &env, &lsp_ctx, value);
        // Cursor on the step `name:` value `s` — not inside
        // `request:`.
        let range = cursor(1, 12);
        assert!(scaffold_assert_code_action(&uri, source, range, &ctx).is_none());
    }

    #[test]
    fn scaffold_assert_merges_into_existing_assert_body_entries() {
        let source = "steps:\n  - name: s\n    request:\n      method: GET\n      url: http://x/\n    assert:\n      body:\n        \"$.id\":\n          type: number\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        // Response has `id` (already covered) and `name` (new).
        let value = serde_json::json!({"id": 1, "name": "A"});
        let ctx = ctx_with_reader(&uri, source, &env, &lsp_ctx, value);
        let range = cursor(4, 15);
        let action = scaffold_assert_code_action(&uri, source, range, &ctx).expect("action");
        let edit = action.edit.expect("edit");
        let changes = edit.changes.expect("changes");
        let edits = changes.get(&uri).expect("edits");
        let text = &edits[0].new_text;
        assert!(
            text.contains("\"$.name\":"),
            "expected new $.name entry, got: {text}"
        );
        assert!(
            !text.contains("\"$.id\":"),
            "existing $.id must not be re-emitted: {text}"
        );
        assert!(
            !text.contains("assert:"),
            "expected append, not a fresh assert block: {text}"
        );
    }

    #[test]
    fn scaffold_assert_declines_on_empty_object() {
        let source = "steps:\n  - name: s\n    request:\n      method: GET\n      url: http://x/\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let value = serde_json::json!({});
        let ctx = ctx_with_reader(&uri, source, &env, &lsp_ctx, value);
        let range = cursor(4, 15);
        assert!(scaffold_assert_code_action(&uri, source, range, &ctx).is_none());
    }

    #[test]
    fn scaffold_assert_declines_when_response_is_not_object() {
        let source = "steps:\n  - name: s\n    request:\n      method: GET\n      url: http://x/\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let value = serde_json::json!([1, 2, 3]);
        let ctx = ctx_with_reader(&uri, source, &env, &lsp_ctx, value);
        let range = cursor(4, 15);
        assert!(scaffold_assert_code_action(&uri, source, range, &ctx).is_none());
    }

    #[test]
    fn scaffold_assert_declines_when_every_field_already_asserted() {
        let source = "steps:\n  - name: s\n    request:\n      method: GET\n      url: http://x/\n    assert:\n      body:\n        \"$.id\":\n          type: number\n";
        let uri = uri();
        let env = empty_env();
        let lsp_ctx = empty_lsp_ctx();
        let value = serde_json::json!({"id": 1});
        let ctx = ctx_with_reader(&uri, source, &env, &lsp_ctx, value);
        let range = cursor(4, 15);
        assert!(scaffold_assert_code_action(&uri, source, range, &ctx).is_none());
    }
}
