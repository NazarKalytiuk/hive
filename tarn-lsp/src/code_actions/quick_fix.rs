//! `Apply fix: …` **Quick Fix** code action (NAZ-305, Phase L3.4).
//!
//! Bridges the shared `tarn::fix_plan::generate_fix_plan` library into
//! the LSP dispatcher. For every diagnostic the client sends along
//! with a `textDocument/codeAction` request, the provider asks the
//! library whether a mechanical fix exists; if one does, it converts
//! the library's [`FixPlan`] into a fully-resolved [`CodeAction`] with
//! kind [`CodeActionKind::QUICKFIX`].
//!
//! # Inputs
//!
//! LSP clients forward the set of diagnostics overlapping the current
//! selection in `CodeActionParams.context.diagnostics`. The provider
//! reads that list rather than re-running the validator — diagnostics
//! and code actions are separate request paths in LSP and the client
//! has already paid for the validation work once per keystroke via
//! `textDocument/publishDiagnostics`.
//!
//! # Filtering rules
//!
//! Three gates must pass before a diagnostic yields a Quick Fix:
//!
//!   1. The diagnostic must originate from `tarn` — `source == Some("tarn")`.
//!      Diagnostics from other sources (a linter plugin, a schema
//!      overlay, …) are silently skipped because the fix-plan library
//!      has no way to reason about them.
//!   2. The diagnostic must have a `code` that matches the validator's
//!      stable set (`tarn_parse`, `tarn_validation`, `yaml_syntax`).
//!      The library itself filters out YAML-syntax diagnostics as a
//!      second layer of defence, but we also drop them here so the
//!      LSP surface never has to think about non-validator codes.
//!   3. The library must return a [`FixPlan`] with non-empty edits.
//!      Advice-only plans (the report-driven path) are never produced
//!      for the LSP pipeline — the library contract guarantees it —
//!      but the provider asserts on that invariant defensively so a
//!      future mis-wired call cannot ship an unusable Quick Fix.
//!
//! # Output
//!
//! Every Quick Fix carries:
//!
//!   * `kind: CodeActionKind::QUICKFIX`
//!   * `title`: the library's plan title, prefixed with `"Apply fix: "`
//!   * `diagnostics: Some(vec![diagnostic])` — pins the action to the
//!     single diagnostic it addresses so clients can render it under
//!     the matching squiggle
//!   * `edit: WorkspaceEdit` — a `changes` map keyed on the current
//!     buffer URI, with a single-line replacement for the typo'd key
//!   * `is_preferred: Some(true)` — library-produced plans are
//!     unambiguous by construction, so clients that auto-apply the
//!     preferred action can do so without a prompt
//!
//! # Pure renderer
//!
//! Like every other L3 code action provider, `quick_fix_code_actions`
//! is pure: `(uri, source, range, ctx) -> Vec<CodeAction>`, no
//! filesystem, no clocks, no shared state. Unit tests drive it with a
//! synthetic [`CodeActionContext`].

use lsp_types::{
    CodeAction, CodeActionKind, Diagnostic, NumberOrString, Position, Range, TextEdit, Url,
    WorkspaceEdit,
};
use std::collections::HashMap;
use tarn::fix_plan::{generate_fix_plan, FixEdit, FixPlan};
use tarn::model::Location;
use tarn::validation::{Severity, ValidationCode, ValidationMessage};

use crate::code_actions::CodeActionContext;
use crate::diagnostics::DIAGNOSTIC_SOURCE;

/// Title prefix the provider stamps on every emitted Quick Fix.
/// Kept as a constant so integration tests and future localisation
/// work can match on one stable string.
pub const QUICK_FIX_TITLE_PREFIX: &str = "Apply fix: ";

/// Pure renderer — the dispatcher entry point.
///
/// Walks every diagnostic in `ctx.lsp_ctx.diagnostics`, runs each one
/// through the shared fix-plan library, and collects the resulting
/// actions into a vector preserving input order.
pub fn quick_fix_code_actions(
    uri: &Url,
    source: &str,
    _range: Range,
    ctx: &CodeActionContext<'_>,
) -> Vec<CodeAction> {
    let mut out: Vec<CodeAction> = Vec::new();
    for diagnostic in &ctx.lsp_ctx.diagnostics {
        if !is_tarn_validation_diagnostic(diagnostic) {
            continue;
        }
        let Some(validation_msg) = diagnostic_to_validation_message(diagnostic) else {
            continue;
        };
        let plans = generate_fix_plan(source, std::slice::from_ref(&validation_msg));
        for plan in plans {
            if plan.edits.is_empty() {
                // Defensive: the diagnostic-driven library path never
                // emits advice-only plans, but a future change could.
                // Drop them so the LSP surface never offers an empty
                // Quick Fix.
                continue;
            }
            out.push(build_code_action(uri, diagnostic, &plan));
        }
    }
    out
}

/// True when `d` is a diagnostic the fix-plan library knows how to
/// reason about. We require:
///
///   * `source == Some("tarn")` — foreign diagnostics are always
///     skipped so the provider cannot accidentally "fix" a message
///     emitted by another extension.
///   * `code` is one of the three stable validator codes. YAML syntax
///     errors pass this gate but the library drops them in its own
///     filter; we still accept them here so the path stays uniform.
fn is_tarn_validation_diagnostic(d: &Diagnostic) -> bool {
    if d.source.as_deref() != Some(DIAGNOSTIC_SOURCE) {
        return false;
    }
    matches!(
        d.code.as_ref(),
        Some(NumberOrString::String(c))
            if c == "tarn_parse" || c == "tarn_validation" || c == "yaml_syntax"
    )
}

/// Lift an LSP [`Diagnostic`] back into the [`ValidationMessage`] type
/// the library operates on. This is the inverse of
/// `diagnostics::tarn_messages_to_diagnostics` — every field the
/// library cares about (code, message, severity, location) comes
/// back, but the LSP-only fields (`related_information`, `tags`) are
/// dropped because the library never reads them.
fn diagnostic_to_validation_message(d: &Diagnostic) -> Option<ValidationMessage> {
    let code_str = match d.code.as_ref()? {
        NumberOrString::String(s) => s.as_str(),
        NumberOrString::Number(_) => return None,
    };
    let code = match code_str {
        "tarn_parse" => ValidationCode::TarnParse,
        "tarn_validation" => ValidationCode::TarnValidation,
        "yaml_syntax" => ValidationCode::YamlSyntax,
        _ => return None,
    };
    let severity = match d.severity {
        Some(lsp_types::DiagnosticSeverity::WARNING) => Severity::Warning,
        _ => Severity::Error,
    };
    // `location` is best-effort: the library's diagnostic path does
    // not actually read it for the unknown-field pattern (the YAML
    // walker re-derives positions from the source buffer), so we just
    // populate it with the diagnostic's own range start for the LSP
    // clients that might later expose location-aware fixes.
    Some(ValidationMessage {
        severity,
        code,
        message: d.message.clone(),
        location: Some(Location {
            file: String::new(),
            // 0-based LSP → 1-based tarn. Bump both fields to avoid
            // underflowing on a zero-zero range.
            line: (d.range.start.line as usize) + 1,
            column: (d.range.start.character as usize) + 1,
        }),
    })
}

/// Assemble the concrete [`CodeAction`] for one `(diagnostic, plan)`
/// pair. All positions come out of the library's 1-based [`Location`]
/// shape and get converted to 0-based LSP `Position`s exactly once
/// here.
fn build_code_action(uri: &Url, diagnostic: &Diagnostic, plan: &FixPlan) -> CodeAction {
    let mut text_edits: Vec<TextEdit> = plan.edits.iter().map(fix_edit_to_text_edit).collect();
    // Sort reverse document order so clients applying edits in
    // sequence do not invalidate earlier offsets.
    text_edits.sort_by(|a, b| {
        b.range
            .start
            .line
            .cmp(&a.range.start.line)
            .then(b.range.start.character.cmp(&a.range.start.character))
    });

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    changes.insert(uri.clone(), text_edits);
    let workspace_edit = WorkspaceEdit {
        changes: Some(changes),
        ..WorkspaceEdit::default()
    };

    CodeAction {
        title: format!("{QUICK_FIX_TITLE_PREFIX}{}", plan.title),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(workspace_edit),
        is_preferred: Some(plan.preferred),
        ..CodeAction::default()
    }
}

/// Convert a 1-based [`FixEdit`] into a 0-based LSP [`TextEdit`].
///
/// The library emits point-plus-length ranges because every fix it
/// currently produces is a single-line replacement of a YAML mapping
/// key. We materialise that shape into a `Range { start, end }` pair
/// where `end` is on the same line, `length` columns to the right of
/// `start`.
fn fix_edit_to_text_edit(edit: &FixEdit) -> TextEdit {
    let line = edit.range.line.saturating_sub(1) as u32;
    let character = edit.range.column.saturating_sub(1) as u32;
    let start = Position::new(line, character);
    let end = Position::new(line, character + edit.length as u32);
    TextEdit {
        range: Range::new(start, end),
        new_text: edit.new_text.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lsp_types::{CodeActionContext as LspCodeActionContext, DiagnosticSeverity};
    use std::collections::BTreeMap;
    use tarn::env::EnvEntry;

    // -- helpers ----------------------------------------------------

    fn fixture_uri() -> Url {
        Url::parse("file:///tmp/qf.tarn.yaml").unwrap()
    }

    fn diag(
        code: &str,
        message: &str,
        line: u32,
        character: u32,
        source: Option<&str>,
    ) -> Diagnostic {
        Diagnostic {
            range: Range::new(
                Position::new(line, character),
                Position::new(line, character),
            ),
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String(code.to_string())),
            code_description: None,
            source: source.map(|s| s.to_string()),
            message: message.to_string(),
            related_information: None,
            tags: None,
            data: None,
        }
    }

    fn make_ctx<'a>(
        uri: &'a Url,
        source: &'a str,
        env: &'a BTreeMap<String, EnvEntry>,
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

    // -- tests ------------------------------------------------------

    #[test]
    fn diagnostic_with_fix_plan_yields_quick_fix_action() {
        let uri = fixture_uri();
        let source = "name: x\nstep: []\n";
        let d = diag(
            "tarn_validation",
            "Unknown field 'step' at root. Did you mean 'steps'?",
            0,
            0,
            Some("tarn"),
        );
        let env = BTreeMap::new();
        let lsp_ctx = LspCodeActionContext {
            diagnostics: vec![d.clone()],
            only: None,
            trigger_kind: None,
        };
        let ctx = make_ctx(&uri, source, &env, &lsp_ctx);
        let actions = quick_fix_code_actions(&uri, source, Range::default(), &ctx);
        assert_eq!(actions.len(), 1);
        let action = &actions[0];
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        assert_eq!(action.title, "Apply fix: Change 'step' to 'steps'");
        assert_eq!(action.is_preferred, Some(true));
        assert_eq!(
            action.diagnostics.as_ref().map(|v| v.len()),
            Some(1),
            "quick fix must pin to the source diagnostic"
        );
        let edit = action.edit.as_ref().expect("workspace edit");
        let changes = edit.changes.as_ref().expect("changes");
        assert_eq!(changes.len(), 1, "only the current URI should carry edits");
        let edits = changes.get(&uri).expect("edits on current uri");
        assert_eq!(edits.len(), 1);
        let e = &edits[0];
        // `step:` at 1-based (2, 1) → 0-based start (1, 0), end (1, 4).
        assert_eq!(e.range.start, Position::new(1, 0));
        assert_eq!(e.range.end, Position::new(1, 4));
        assert_eq!(e.new_text, "steps");
    }

    #[test]
    fn diagnostic_without_fix_plan_yields_no_action() {
        let uri = fixture_uri();
        let source =
            "name: x\nsteps:\n  - name: s1\n    request:\n      method: GET\n      url: u\n";
        let d = diag("tarn_parse", "Step 's1' has empty URL", 2, 10, Some("tarn"));
        let env = BTreeMap::new();
        let lsp_ctx = LspCodeActionContext {
            diagnostics: vec![d],
            only: None,
            trigger_kind: None,
        };
        let ctx = make_ctx(&uri, source, &env, &lsp_ctx);
        let actions = quick_fix_code_actions(&uri, source, Range::default(), &ctx);
        assert!(actions.is_empty(), "no fix ⇒ no action, got {actions:?}");
    }

    #[test]
    fn two_diagnostics_with_fixes_yield_two_actions_in_order() {
        let uri = fixture_uri();
        let source = "name: x\nstep: []\nteardowns: []\n";
        let d1 = diag(
            "tarn_validation",
            "Unknown field 'step' at root. Did you mean 'steps'?",
            1,
            0,
            Some("tarn"),
        );
        let d2 = diag(
            "tarn_validation",
            "Unknown field 'teardowns' at root. Did you mean 'teardown'?",
            2,
            0,
            Some("tarn"),
        );
        let env = BTreeMap::new();
        let lsp_ctx = LspCodeActionContext {
            diagnostics: vec![d1.clone(), d2.clone()],
            only: None,
            trigger_kind: None,
        };
        let ctx = make_ctx(&uri, source, &env, &lsp_ctx);
        let actions = quick_fix_code_actions(&uri, source, Range::default(), &ctx);
        assert_eq!(actions.len(), 2);
        assert!(actions[0].title.contains("'step' to 'steps'"));
        assert!(actions[1].title.contains("'teardowns' to 'teardown'"));
        // Each action pins to exactly one diagnostic.
        assert_eq!(actions[0].diagnostics.as_ref().unwrap()[0], d1);
        assert_eq!(actions[1].diagnostics.as_ref().unwrap()[0], d2);
    }

    #[test]
    fn diagnostic_from_foreign_source_is_skipped() {
        let uri = fixture_uri();
        let source = "name: x\nstep: []\n";
        let d = diag(
            "tarn_validation",
            "Unknown field 'step' at root. Did you mean 'steps'?",
            1,
            0,
            Some("yaml-lint"),
        );
        let env = BTreeMap::new();
        let lsp_ctx = LspCodeActionContext {
            diagnostics: vec![d],
            only: None,
            trigger_kind: None,
        };
        let ctx = make_ctx(&uri, source, &env, &lsp_ctx);
        let actions = quick_fix_code_actions(&uri, source, Range::default(), &ctx);
        assert!(
            actions.is_empty(),
            "non-`tarn` source must be skipped as a safety gate"
        );
    }

    #[test]
    fn diagnostic_with_numeric_code_is_skipped() {
        // `code: Some(NumberOrString::Number(...))` has no meaning in
        // the tarn validator, so the provider must decline rather than
        // panic or miscast.
        let uri = fixture_uri();
        let source = "name: x\nstep: []\n";
        let d = Diagnostic {
            range: Range::new(Position::new(1, 0), Position::new(1, 0)),
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::Number(42)),
            code_description: None,
            source: Some("tarn".to_string()),
            message: "Unknown field 'step' at root. Did you mean 'steps'?".to_string(),
            related_information: None,
            tags: None,
            data: None,
        };
        let env = BTreeMap::new();
        let lsp_ctx = LspCodeActionContext {
            diagnostics: vec![d],
            only: None,
            trigger_kind: None,
        };
        let ctx = make_ctx(&uri, source, &env, &lsp_ctx);
        let actions = quick_fix_code_actions(&uri, source, Range::default(), &ctx);
        assert!(actions.is_empty());
    }

    #[test]
    fn empty_diagnostics_list_yields_empty_actions() {
        let uri = fixture_uri();
        let source = "name: x\nsteps:\n  - name: s1\n    request:\n      method: GET\n      url: u\n    assert:\n      status: 200\n";
        let env = BTreeMap::new();
        let lsp_ctx = LspCodeActionContext {
            diagnostics: Vec::new(),
            only: None,
            trigger_kind: None,
        };
        let ctx = make_ctx(&uri, source, &env, &lsp_ctx);
        let actions = quick_fix_code_actions(&uri, source, Range::default(), &ctx);
        assert!(actions.is_empty());
    }

    #[test]
    fn action_edit_keyed_on_current_uri_only() {
        let uri = fixture_uri();
        let source = "name: x\nstep: []\n";
        let d = diag(
            "tarn_validation",
            "Unknown field 'step' at root. Did you mean 'steps'?",
            1,
            0,
            Some("tarn"),
        );
        let env = BTreeMap::new();
        let lsp_ctx = LspCodeActionContext {
            diagnostics: vec![d],
            only: None,
            trigger_kind: None,
        };
        let ctx = make_ctx(&uri, source, &env, &lsp_ctx);
        let actions = quick_fix_code_actions(&uri, source, Range::default(), &ctx);
        let edit = actions[0].edit.as_ref().unwrap();
        let changes = edit.changes.as_ref().unwrap();
        assert_eq!(changes.keys().len(), 1);
        assert!(changes.contains_key(&uri));
    }

    #[test]
    fn quick_fix_title_uses_apply_fix_prefix() {
        let uri = fixture_uri();
        let source = "name: x\nstep: []\n";
        let d = diag(
            "tarn_validation",
            "Unknown field 'step' at root. Did you mean 'steps'?",
            1,
            0,
            Some("tarn"),
        );
        let env = BTreeMap::new();
        let lsp_ctx = LspCodeActionContext {
            diagnostics: vec![d],
            only: None,
            trigger_kind: None,
        };
        let ctx = make_ctx(&uri, source, &env, &lsp_ctx);
        let actions = quick_fix_code_actions(&uri, source, Range::default(), &ctx);
        assert!(
            actions[0].title.starts_with(QUICK_FIX_TITLE_PREFIX),
            "title must start with the stable prefix, got {:?}",
            actions[0].title
        );
    }

    #[test]
    fn quick_fix_marks_action_as_preferred() {
        let uri = fixture_uri();
        let source = "name: x\nstep: []\n";
        let d = diag(
            "tarn_validation",
            "Unknown field 'step' at root. Did you mean 'steps'?",
            1,
            0,
            Some("tarn"),
        );
        let env = BTreeMap::new();
        let lsp_ctx = LspCodeActionContext {
            diagnostics: vec![d],
            only: None,
            trigger_kind: None,
        };
        let ctx = make_ctx(&uri, source, &env, &lsp_ctx);
        let actions = quick_fix_code_actions(&uri, source, Range::default(), &ctx);
        assert_eq!(actions[0].is_preferred, Some(true));
    }

    #[test]
    fn nested_context_path_emits_edit_at_correct_line() {
        let uri = fixture_uri();
        // `header:` is at line 7 (1-based) column 7.
        let source = "name: x\nsteps:\n  - name: s1\n    request:\n      method: GET\n      url: u\n      header:\n        a: b\n    assert:\n      status: 200\n";
        let d = diag(
            "tarn_validation",
            "Unknown field 'header' at root.steps[0].request. Did you mean 'headers'?",
            6,
            6,
            Some("tarn"),
        );
        let env = BTreeMap::new();
        let lsp_ctx = LspCodeActionContext {
            diagnostics: vec![d],
            only: None,
            trigger_kind: None,
        };
        let ctx = make_ctx(&uri, source, &env, &lsp_ctx);
        let actions = quick_fix_code_actions(&uri, source, Range::default(), &ctx);
        assert_eq!(actions.len(), 1);
        let edit = &actions[0].edit.as_ref().unwrap();
        let edits = edit.changes.as_ref().unwrap().get(&uri).unwrap();
        let e = &edits[0];
        // 1-based (7, 7) → 0-based (6, 6), length 6 ⇒ end column 12.
        assert_eq!(e.range.start, Position::new(6, 6));
        assert_eq!(e.range.end, Position::new(6, 12));
        assert_eq!(e.new_text, "headers");
    }

    #[test]
    fn mix_of_fixable_and_non_fixable_diagnostics_only_returns_fixable() {
        let uri = fixture_uri();
        let source = "name: x\nstep: []\n";
        let d_fixable = diag(
            "tarn_validation",
            "Unknown field 'step' at root. Did you mean 'steps'?",
            1,
            0,
            Some("tarn"),
        );
        let d_not_fixable = diag(
            "tarn_parse",
            "Test file must have either 'steps' or 'tests'",
            0,
            0,
            Some("tarn"),
        );
        let env = BTreeMap::new();
        let lsp_ctx = LspCodeActionContext {
            diagnostics: vec![d_fixable.clone(), d_not_fixable],
            only: None,
            trigger_kind: None,
        };
        let ctx = make_ctx(&uri, source, &env, &lsp_ctx);
        let actions = quick_fix_code_actions(&uri, source, Range::default(), &ctx);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].diagnostics.as_ref().unwrap()[0], d_fixable);
    }
}
