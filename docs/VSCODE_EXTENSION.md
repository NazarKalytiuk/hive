# Tarn VS Code Extension

This document is the canonical spec for the Tarn VS Code extension. It covers what the extension does, how it maps onto Tarn's CLI and report schema, which additive Tarn CLI changes it depends on, and the phased delivery plan.

The extension lives in `editors/vscode/`. It already ships as a declarative package — language id, grammar, snippets, and schema wiring — but has no extension host code yet. Everything in this document is additive to that package. Nothing existing gets removed.

## Goals

- First-class Test Explorer integration: discover, run, debug, cancel, filter, watch.
- Inline authoring: CodeLens, gutter icons, hover diagnostics, completion, go-to-definition, rename, schema validation, snippets, interpolated previews.
- Failure UX that beats the terminal: diff view, request/response inspector, "reveal in editor" on the exact failing line, fix plan panel.
- Workflow glue: env picker, tag picker, HTML report webview, curl export, bench runner, Hurl import, project scaffolding.
- Zero-config for standard repos; explicit settings for monorepos, custom binaries, Remote SSH, Dev Containers, WSL, Codespaces.
- Tarn-side changes stay small, additive, and backwards compatible. No forks.

## Non-Goals

- Rewriting the runner in TypeScript. The extension is a thin stateful shell around `tarn` and optionally `tarn-mcp`.
- A proprietary format. `.tarn.yaml` stays canonical.
- Web extension (`vscode.dev`) support in v1. We spawn a native binary.
- Cloud sync, team dashboards, or any network-side features.

## Current State

`editors/vscode/` today:

- `package.json` declares language `tarn` for `*.tarn.yaml` / `*.tarn.yml`, grammar at `syntaxes/tarn.tmLanguage.json`, snippets at `snippets/tarn.code-snippets`, and schema wiring for test files and report files via `redhat.vscode-yaml` as an `extensionDependency`.
- Snippet prefixes: `tarn-test`, `tarn-step`, `tarn-capture`, `tarn-poll`, `tarn-form`, `tarn-graphql`, `tarn-multipart`, `tarn-lifecycle`, `tarn-include`.
- Publisher: `nazarkalytiuk`. Version: `0.1.0`. Engine: `^1.90.0`.
- No `main`, no `src/`, no TypeScript, no activation beyond `onLanguage:tarn` / `onLanguage:json`.

The Phase 1 work adds a `main` entry, an `src/` tree, an esbuild bundle, and a test harness, without touching the grammar, snippets, or the existing `contributes` blocks. Version bumps to `0.2.0` when Phase 1 ships.

## Architecture

```
┌──────────────────── VS Code Extension Host ────────────────────┐
│                                                                 │
│  ┌─ Test Controller ─┐    ┌─── CodeLens Providers ───┐          │
│  │  discovery        │    │  per-test / per-step     │          │
│  │  run / debug      │    │  Run | Debug | Dry-run   │          │
│  │  cancel / watch   │    │  Copy as curl            │          │
│  └────────┬──────────┘    └──────────┬───────────────┘          │
│           │                          │                          │
│  ┌────────▼─────────── Core Services ▼──────────┐               │
│  │  WorkspaceIndex   YamlAst   EnvService       │               │
│  │  RunQueue         ResultMapper   Telemetry   │               │
│  └────────┬────────────────────────────┬────────┘               │
│           │                            │                        │
│  ┌────────▼────────┐           ┌───────▼─────────┐              │
│  │ TarnProcessRunner│           │ TarnMcpClient   │              │
│  │ spawn `tarn run`│           │ stdio JSON-RPC  │              │
│  └────────┬────────┘           └───────┬─────────┘              │
│           ▼                            ▼                        │
│   ┌─────────────┐                ┌─────────────┐                │
│   │  tarn CLI   │                │  tarn-mcp   │                │
│   └─────────────┘                └─────────────┘                │
└─────────────────────────────────────────────────────────────────┘
```

Two backends, one abstraction. `TarnBackend` exposes `run`, `list`, `validate`, `fixPlan`. The default implementation is `TarnProcessRunner`, which spawns `tarn run --format json`. Advanced users can switch to `TarnMcpClient`, a long-lived `tarn-mcp` process over stdio, for lower latency and shared state. Everything else is backend-agnostic.

### Core services

- `WorkspaceIndex` globs `**/*.tarn.yaml`, holds `Map<fileUri, ParsedFile>`, invalidates on change.
- `YamlAst` parses every test file with the `yaml` library in CST mode and keeps node → `Range` maps. This is how we anchor results, CodeLens, symbols, completion, rename, and diagnostics to exact lines, even before Tarn's JSON gains location metadata.
- `EnvService` reads `tarn.env*.yaml` and `tarn.config.yaml`, shells out to `tarn env --json` for named environments, and exposes the active selection via `workspaceState`.
- `RunQueue` serializes concurrent runs per file to prevent cookie-jar crosstalk, allows parallelism across files, and wires `CancellationToken` to `ChildProcess.kill`.
- `ResultMapper` joins a Tarn JSON report to the `YamlAst` to populate `TestRun.passed|failed|errored` with `TestMessage` anchored to ranges.

### Repo layout (target)

```
editors/vscode/
├── package.json
├── tsconfig.json
├── esbuild.config.mjs
├── .vscodeignore
├── CHANGELOG.md
├── README.md                     (exists)
├── language-configuration.json   (exists)
├── syntaxes/                     (exists)
├── snippets/                     (exists)
├── media/
├── schemas/                      (optional local copy pinned at build time)
└── src/
    ├── extension.ts
    ├── backend/
    │   ├── TarnBackend.ts
    │   ├── TarnProcessRunner.ts
    │   ├── TarnMcpClient.ts
    │   └── binaryResolver.ts
    ├── workspace/
    │   ├── WorkspaceIndex.ts
    │   ├── YamlAst.ts
    │   ├── ParsedFile.ts
    │   └── fileWatcher.ts
    ├── testing/
    │   ├── TestController.ts
    │   ├── discovery.ts
    │   ├── runHandler.ts
    │   ├── ResultMapper.ts
    │   └── cancellation.ts
    ├── codelens/
    │   ├── TestCodeLensProvider.ts
    │   └── StepCodeLensProvider.ts
    ├── language/
    │   ├── HoverProvider.ts
    │   ├── CompletionProvider.ts
    │   ├── DefinitionProvider.ts
    │   ├── DiagnosticsProvider.ts
    │   └── injection.ts
    ├── views/
    │   ├── EnvironmentsView.ts
    │   ├── RunHistoryView.ts
    │   ├── ReportWebview.ts
    │   ├── RequestResponsePanel.ts
    │   └── CapturesInspector.ts
    ├── commands/
    │   ├── runFile.ts
    │   ├── runAll.ts
    │   ├── runSelection.ts
    │   ├── dryRun.ts
    │   ├── exportCurl.ts
    │   ├── importHurl.ts
    │   ├── initProject.ts
    │   ├── bench.ts
    │   ├── openHtmlReport.ts
    │   ├── setEnvironment.ts
    │   ├── setTagFilter.ts
    │   └── installTarn.ts
    ├── statusBar.ts
    ├── outputChannel.ts
    ├── config.ts
    ├── telemetry.ts
    └── util/
        ├── schemaGuards.ts
        ├── shellEscape.ts
        └── diff.ts
└── tests/
    ├── unit/        (vitest)
    └── integration/ (@vscode/test-electron)
```

## Tech Stack

- TypeScript 5, bundled with esbuild to a single `out/extension.js`.
- `yaml` v2 (eemeli) for CST parsing and range maps.
- `zod` for runtime guards against `schemas/v1/report.json`.
- `execa` for child processes, never via shell, always an argv array.
- `vitest` for unit tests.
- `@vscode/test-electron` for integration tests against a real `tarn` binary.
- `vsce` + `ovsx` for publishing.

## Feature Set

### Test Explorer

| Feature | Tarn mapping | Notes |
|---|---|---|
| Hierarchical tree: workspace → file → test → step | `files[].tests[].steps[]` plus YAML AST | Setup and teardown appear as virtual collapsible nodes per file. |
| Discovery on activation, file change, rename, delete | `WorkspaceIndex` plus `createFileSystemWatcher('**/*.tarn.yaml')` | Incremental: only reparses changed files. |
| Run profiles: Run, Debug, Dry-run, Run with env…, Run with --var… | `tarn run`, `tarn run --dry-run`, `tarn run --env`, `tarn run --var` | Four distinct `TestRunProfile` instances. |
| Continuous run | Extension-side file watching + `TestRunRequest.continuous` | We deliberately don't use `tarn run --watch` because it can't drive the Testing API cleanly. |
| Cancellation | `ChildProcess.kill('SIGINT')` | Tarn handles SIGINT cleanly. |
| Tag filter | `tarn run --tag` | Multi-select quick pick persisted per workspace. |
| Run failed only | Per-test / per-step selection | Depends on Tarn change §6.1. |
| Duration and sparkline per test | `duration_ms` | Rendered via `TestItem.sortText` plus inline decoration. |
| Failure annotations | `assertions.failures[]` → `TestMessage` with location | Location is AST-derived until §6.5 lands. |
| Expected vs actual diff | `assertions.details[].diff` | Surfaced via `TestMessage.actualOutput` / `.expectedOutput`. |
| Rich TestMessage with request / response | Failure `request` plus `response` | Rendered as markdown with method, URL, headers, body preview. |

### Editor features

- CodeLens above every test and step: `▶ Run | 🐞 Debug | 🔁 Dry-run | 📋 Copy as curl`.
- Gutter icons on the line of each test and step name: green, red, not-run, running. Updated live as results stream in.
- Hover:
  - `{{ env.X }}` — resolved value and source file.
  - `{{ capture.Y }}` — where `Y` was captured, file and line, and its last seen type.
  - Any `url:` field — fully interpolated URL via cached `--dry-run`.
  - Status literal — link to MDN.
- Completion:
  - `{{ env.` — keys from all `tarn.env*.yaml` files with source labels.
  - `{{ capture.` — captures visible at the current position, scoped to the same test.
  - `{{ $` — built-ins: `$uuid()`, `$random_hex(n)`, `$timestamp`, `$timestamp_iso8601`, `$now_unix`.
  - `assert: status:` — common codes.
  - `method:` — HTTP verbs.
- Go-to-definition: `{{ capture.x }}` jumps to the step that captured it. `{{ env.x }}` jumps to the highest-priority env file containing it.
- Find-all-references for captures within a file.
- Rename symbol for captures within a file.
- Document symbols: outline shows tests and steps.
- Diagnostics on save via `tarn validate --format json` (depends on §6.2). Falls back to client-side validation against `schemas/v1/testfile.json`, which is already wired via `redhat.vscode-yaml`.
- Grammar injection for `{{ … }}` inside YAML strings (extends the existing `syntaxes/tarn.tmLanguage.json`).
- Schema contribution stays as-is from the current `package.json`.

### Commands (full list)

| Command | Behavior |
|---|---|
| `Tarn: Run All Tests` | Runs the whole workspace honoring active env and tag filter. |
| `Tarn: Run Current File` | Runs the active `.tarn.yaml`. |
| `Tarn: Run Test at Cursor` | Uses YAML AST to find enclosing test or step. Needs §6.1. |
| `Tarn: Dry Run Current File` | `--dry-run`, prints interpolated requests in output channel. |
| `Tarn: Validate Current File` | `tarn validate`. |
| `Tarn: Rerun Last Run` | Reuses the last `RunRequest`. |
| `Tarn: Rerun Failed Tests` | Needs §6.1. |
| `Tarn: Select Environment…` | Quick pick over `tarn env --json`. |
| `Tarn: Set Variable Override…` | Prompts key and value, persists to `workspaceState`. Secret-shaped keys are stored in `SecretStorage`. |
| `Tarn: Clear Variable Overrides` | |
| `Tarn: Set Tag Filter…` | Multi-select. |
| `Tarn: Open HTML Report` | Runs with `--format html=<tmp>` and opens in webview. |
| `Tarn: Copy Step as curl` | `tarn run --format curl` with step selection. Needs §6.1. |
| `Tarn: Export Failed as curl` | Uses existing `--format curl` for failed steps. |
| `Tarn: Import Hurl File…` | Wraps `tarn import-hurl`. |
| `Tarn: Init Project Here` | Wraps `tarn init`. |
| `Tarn: Benchmark Step…` | Wraps `tarn bench`, renders results in webview. |
| `Tarn: Format File` | Wraps `tarn fmt`. Also registered as a `DocumentFormattingEditProvider`. |
| `Tarn: Install / Update Tarn` | Offers Homebrew, cargo install, install.sh, or manual. |
| `Tarn: Show Output` | Focuses output channel. |
| `Tarn: Show Fix Plan` | Calls `tarn_fix_plan` via MCP if enabled, otherwise parses the last run. |
| `Tarn: Toggle Watch Mode` | |
| `Tarn: Clear Cookie Jar for File` | Deletes the jar file for stale-state scenarios. |

### Views (Tarn activity bar container)

1. **Tests** — the Testing view is primary; the container groups it with the extras below.
2. **Environments** — tree of `tarn.env.yaml`, `tarn.env.*.yaml`, and named envs from `tarn.config.yaml`. Decorated with a check on the active one.
3. **Run History** — last `tarn.history.max` runs with status, duration, env, tag filter, scope. Click to rerun, shift-click to open report, right-click to pin.
4. **Fix Plan** — ranked remediation hints grouped by failure category, each with a "jump to line" action.
5. **Captures Inspector** — tree of captured variables per test with expandable JSON values. Redaction-aware, with a "hide all capture values" toggle.
6. **Request/Response Inspector** — split webview opened when a failed step is selected. Tabs: Request, Response, Assertions. Redaction-aware.

### Status bar

- Left: `$(beaker) Tarn: dev` — active environment. Click opens the env picker.
- Left: `$(tag) smoke` — active tag filter if any. Click opens the tag picker.
- Right: `$(check) 42  $(x) 3  1.8s` — last run summary. Click focuses Test Explorer.
- Right during a run: `$(sync~spin) Running 12/42` — live progress. Click opens the output channel.

### Output and problems

- Output channel `"Tarn"` logs every invocation: resolved argv (redacted), stderr, parsed JSON summary.
- `Problems` view gets `vscode.Diagnostic`s from failed validation and failed runs, anchored to the exact YAML range. Severity map: `parse_error` / `validation_failed` / `assertion_mismatch` → Error; `unresolved_template` → Warning unless the run was actually triggered.

### Settings (prefix `tarn.`)

| Key | Type | Default | Purpose |
|---|---|---|---|
| `tarn.binaryPath` | string | `"tarn"` | Override CLI path. |
| `tarn.mcpBinaryPath` | string | `"tarn-mcp"` | Override MCP path. |
| `tarn.backend` | `"cli" \| "mcp"` | `"cli"` | Runtime backend. |
| `tarn.testFileGlob` | string | `"**/*.tarn.yaml"` | Discovery pattern. |
| `tarn.excludeGlobs` | string[] | `["**/target/**","**/node_modules/**"]` | |
| `tarn.defaultEnvironment` | string \| null | `null` | Initial active env. |
| `tarn.defaultTags` | string[] | `[]` | Initial tag filter. |
| `tarn.parallel` | bool | `true` | Pass `--parallel`. |
| `tarn.jobs` | number \| null | `null` | `--jobs`. |
| `tarn.runOnSave` | `"off" \| "file" \| "affected"` | `"off"` | Auto-run trigger. |
| `tarn.validateOnSave` | bool | `true` | Run `tarn validate` on save. |
| `tarn.runOnOpen` | bool | `false` | Run discovery-only on open. |
| `tarn.progressMode` | `"ndjson" \| "poll"` | `"ndjson"` | `ndjson` depends on §6.3. |
| `tarn.jsonMode` | `"verbose" \| "compact"` | `"verbose"` | |
| `tarn.followRedirects` | bool \| null | `null` | |
| `tarn.insecure` | bool | `false` | `--insecure`, guarded by confirmation. |
| `tarn.proxy` | string \| null | `null` | |
| `tarn.httpVersion` | `"auto" \| "1.1" \| "2"` | `"auto"` | |
| `tarn.requestTimeoutMs` | number | `30000` | Process-level watchdog. |
| `tarn.cookieJarMode` | `"default" \| "per-test"` | `"default"` | `per-test` depends on §6.4. |
| `tarn.redactionExtraHeaders` | string[] | `[]` | Merged with Tarn's redaction list. |
| `tarn.showCodeLens` | bool | `true` | |
| `tarn.showGutterIcons` | bool | `true` | |
| `tarn.statusBar.enabled` | bool | `true` | |
| `tarn.history.max` | number | `20` | |
| `tarn.telemetry.enabled` | bool | `false` | Local-only logs even when enabled. |
| `tarn.dryRunPreviewOnHover` | bool | `true` | |
| `tarn.notifications.failure` | `"always" \| "focused" \| "off"` | `"focused"` | |

Every setting is `machine-overridable` where appropriate so Remote-SSH and Dev Containers work correctly.

### Remote and multi-root

- Each workspace folder is indexed independently.
- Binary resolution runs inside the remote extension host, not locally.
- Dev Container: extension contributes a recommended snippet adding `/usr/local/cargo/bin` to `remoteEnv.PATH` plus an install step.
- WSL, Codespaces, Remote SSH: no special casing.
- Web extension: not supported in v1.

### Trust and security

- Activation is gated by `workspaceTrust`. Untrusted workspaces: read-only YAML parsing only. No spawn, no validate, no run.
- Spawning is shell-free. Every invocation is `execa(bin, argsArray)`.
- Variable overrides whose keys match secret shapes (`*_token`, `*_password`, `authorization`) are stored in `SecretStorage`.
- First `--insecure` run in a workspace prompts a modal confirmation.
- All output that might contain secrets rides Tarn's redaction pipeline via `tarn.redactionExtraHeaders`.
- Copy as curl is redaction-aware.

## Mapping Results to Editor Ranges

Tarn's JSON report carries an optional `location: { file, line, column }` on every `StepResult`, `AssertionDetail`, and `AssertionFailure` that maps back to a YAML operator key. This field was added by Tarn T55 (NAZ-260) and is 1-based to match every other line/column Tarn already prints in its human and error output. `ResultMapper` prefers this JSON-reported location over the editor's current YAML AST for runtime result anchoring. The AST layer still builds `NodeRangeMap` for the authoring features below — it just loses its job as the anchor source for red squiggles.

```
NodeRangeMap
  testRanges:  Map<testName, { nameRange, bodyRange }>
  stepRanges:  Map<"{testName}::{stepIndex}", { nameRange, requestRange, assertRange, captureRange }>
  setupRanges: StepRange[]
  teardownRanges: StepRange[]
```

### Preference order

When a JSON report arrives, `ResultMapper.buildFailureMessages` resolves the source anchor for each failure in this exact order:

1. **`failure.location`** (per-assertion) — used for the individual assertion failure's `TestMessage`. Lands on the exact operator node (`status:`, `body $.path:`, `headers:`, etc.) the user authored.
2. **`step.location`** (step-level) — used as the fallback for any assertion failure that lacks its own location, and as the anchor for generic (non-assertion) failures like connection errors or capture failures. Lands on the step's `- name:` key.
3. **`stepItem.range`** (AST) — used only when the JSON report omits `location` entirely. This covers older Tarn versions that predate T55, and `include:`-expanded steps where Tarn emits `location: None` because the step was synthesized from an include directive rather than the top-level file.

The 1-based `line` and `column` from Tarn are decremented by 1 before they become a `vscode.Position`. A Tarn location is a single point, not a range, so the mapper builds a zero-width `vscode.Range` at that point; VS Code expands it to the enclosing token for rendering.

### Drift-free by construction

The whole reason this precedence exists is drift. The AST layer is rebuilt every time the file changes on disk, so `stepItem.range` reflects *the current file*, not the one Tarn actually executed. If the user edits the file between the moment Tarn starts a run and the moment the extension renders the report — or runs several tests in parallel while the editor keeps auto-formatting — the AST range can land dozens of lines away from the real step.

The JSON-reported `location` was captured inside Tarn at parse time, before any HTTP work ran. It is pinned to the exact file the CLI saw, and it survives every subsequent edit in the workbench. Integration tests in `resultMapperLocation.test.ts` verify this by inserting two blank lines at the top of the fixture between run start and report parse, then asserting the diagnostic still lands on the original assertion node.

The AST path is never removed — it is the source of truth for authoring features (CodeLens, document symbols, hover, completion, rename) and it is also the fallback for reports that don't carry `location`. Both paths coexist permanently.

## Streaming Results Live

Tarn already has a `ProgressReporter` trait in `tarn/src/report/progress.rs` wired to the human reporter for sequential and parallel modes. Adding an NDJSON implementation is additive and self-contained.

Until §6.3 ships, the extension falls back to polling the final report. The `tarn.progressMode` setting lets users force one mode. The UI contract is identical either way.

## Tarn-Side Changes Required

All additive, all backwards compatible. None is a blocker for Phase 1, which ships against Tarn 0.4.0 with the poll fallback.

These items are tracked as `T51`–`T57` in `docs/TARN_COMPETITIVENESS_ROADMAP.md` under "Post-Roadmap Additions: VS Code Extension Contract". Mapping:

- §6.1 ↔ T51 — `--select FILE::TEST::STEP`
- §6.2 ↔ T52 — `tarn validate --format json`
- §6.3 ↔ T53 — NDJSON progress reporter
- §6.4 ↔ T54 — Per-test cookie jar
- §6.5 ↔ T55 — Location metadata in results
- §6.6 ↔ T56 — `tarn env --json`
- §6.7 ↔ T57 — `tarn list --file`

### §6.1 Selective execution via `--select`

New flag `--select FILE::TEST::STEP`, repeatable. `STEP` optional. ANDs with `--tag`.

```
--select tests/users.tarn.yaml::create_and_verify_user
--select tests/users.tarn.yaml::create_and_verify_user::"Create user"
```

Enables: run-test-at-cursor, rerun-failed, per-step curl export.

Scope: `runner.rs`, `main.rs`, one CLI integration test. Roughly 150 LoC.

### §6.2 Structured validation output

Add `tarn validate --format json` emitting:

```json
{
  "files": [
    {
      "file": "tests/users.tarn.yaml",
      "valid": false,
      "errors": [
        {"message": "...", "line": 14, "column": 7, "path": "tests.create_and_verify_user.steps[0].assert"}
      ]
    }
  ]
}
```

serde_yaml already surfaces line and column in errors, so this is ~60 LoC in `parser.rs` plus `main.rs`.

### §6.3 NDJSON progress reporter

New `NdjsonProgressReporter` behind `--ndjson` or `--format ndjson`. Event shape:

```jsonl
{"event":"file_started","file":"...","timestamp":"..."}
{"event":"step_finished","file":"...","test":"...","step":"...","status":"PASSED","duration_ms":12}
{"event":"test_finished","file":"...","test":"...","status":"FAILED"}
{"event":"file_finished","file":"...","summary":{...}}
{"event":"done","summary":{...}}
```

Co-exists with `--format json=path`. Scope: one new module implementing the existing trait, ~80 LoC plus a unit test.

### §6.4 Per-test cookie jar

Add `cookies: per-test` in the model plus `--cookie-jar-per-test` CLI flag. Resets the jar between named tests in a file so IDE subset runs don't pollute each other.

### §6.5 Location metadata on results

Extend `StepResult` and `AssertionFailure` in `tarn/src/assert/types.rs` with:

```rust
pub struct ResultLocation {
    pub file: String,
    pub line: u32,
    pub column: u32,
}
```

Thread the value through from the parser. Add the optional field to `schemas/v1/report.json`.

### §6.6 `tarn env --json`

Return all configured named environments, their source files, and resolved variables with redaction applied. Enables the env picker without client-side config parsing.

### §6.7 `tarn list --file PATH --format json`

Scoped discovery for a single file. Avoids the extension globbing the workspace for list calls.

Tarn T57 (NAZ-261) shipped this command and extension version `0.22.0` (NAZ-282) wires it into the incremental `WorkspaceIndex` refresh path. The discovery precedence is now:

1. **Startup discovery** — `WorkspaceIndex.initialize()` globs `**/*.tarn.yaml` and parses every match with the client-side YAML AST. This deliberately does NOT spawn Tarn once per file, because activation latency dominates on workspaces with dozens of test files.
2. **Incremental refresh on `onDidChange` / `onDidCreate`** — `WorkspaceIndex.refreshSingleFile(uri)` calls `tarn list --file <uri.fsPath> --format json` via the backend. If the outcome is `{ ok: true, file }` the extension merges Tarn's authoritative tests/steps with the AST's ranges via `mergeScopedWithAst`, then compares against the cached entry with `rangesStructurallyEqual` and only notifies the TestController when the structure actually changed.
3. **Per-file fallback** — if Tarn returns `{ ok: false, reason: "file_error" }` (the YAML parses in the editor but Tarn rejects it at load time, e.g., "Test file must have either 'steps' or 'tests'"), the refresh path falls back to the client AST for that one file only and leaves scoped discovery enabled for the rest of the session.
4. **Session-wide fallback** — if Tarn returns `{ ok: false, reason: "unsupported" }` (missing binary, spawn error, watchdog, older Tarn without `--file`, or a completely unrecognized JSON shape), the extension flips a session-local capability flag and stays on the AST path until the next explicit `Tarn: Refresh Discovery`, which re-runs `initialize()` and resets the flag.

Because Tarn resolves `include:` directives at parse time, the scoped path is the only way the Test Explorer can show `include:`-expanded steps with their real names — the client AST alone only sees the `{ include: "./shared.tarn.yaml" }` entry.

All seven items are tracked as `T51`–`T57` in `docs/TARN_COMPETITIVENESS_ROADMAP.md` with per-item acceptance criteria.

## Phased Delivery

Every phase is shippable on its own.

### Phase 1 — Foundation (extension `0.2.0`, Tarn `0.4.0`)

- Extension host scaffold, activation, binary resolver, settings, output channel, status bar skeleton.
- `WorkspaceIndex` and `YamlAst`, range maps, document symbols.
- TestController with discovery, full hierarchy, Run and Dry-run profiles, cancellation, results via final JSON report.
- CodeLens on tests and steps with Run, Dry-run, Copy as curl.
- Gutter icons, TestMessage with diff, request, response.
- Environment picker, tag filter, Run History view.
- Rerun last run, run current file, run all.
- Trust model, shell-escape utilities, redaction-extra-headers passthrough.
- Walkthrough and sample workspace command.
- Unit tests (vitest) plus integration tests (`@vscode/test-electron`) against real `tarn`.
- CI: GitHub Actions matrix macOS / Linux / Windows, publishes VSIX artifact.

### Phase 2 — Streaming plus run-at-cursor (extension `0.3.0`, Tarn §6.1, §6.3)

- NDJSON-driven live updates in Test Explorer, gutter, status bar.
- `Tarn: Run Test at Cursor`, `Tarn: Run Step at Cursor`.
- Rerun failed only.
- Captures Inspector view.
- Fix Plan view via `tarn run` plus `tarn_fix_plan` if available.
- Request/Response Inspector webview.
- Continuous run via `TestRunRequest.continuous`.

### Phase 3 — Authoring power (extension `0.4.0`, Tarn §6.2, §6.6)

- Completion, hover, definition, references, rename.
- Structured `tarn validate` diagnostics on save.
- YAML grammar injection for `{{ … }}` scopes.
- Environments tree view with set-active and open actions.
- `tarn fmt` format provider.

### Phase 4 — Reports and rich UX (extension `0.5.0`)

- HTML report webview.
- Bench runner wizard with charts.
- Import Hurl wizard.
- Init Project wizard.
- Run History pinning and filtering.
- Failure notifications with inline actions.
- Local-only telemetry log.

### Phase 5 — MCP backend plus advanced (extension `0.6.0`, Tarn §6.4, §6.5, §6.7)

- Optional `TarnMcpClient` backend, one long-lived `tarn-mcp` process per workspace.
- Per-test cookie jar isolation honored.
- Tarn-side location metadata replaces AST matching for runtime results.
- Scoped `tarn list --file`.
- Remote compatibility audits (Dev Container, Codespaces, WSL, Remote SSH). Full writeup in [`VSCODE_REMOTE.md`](VSCODE_REMOTE.md).
- Published to VS Code Marketplace and Open VSX.

### Phase 6 — Ecosystem (extension `1.0.0`)

- Stable API promise.
- Localization baseline (EN).
- Marketplace assets, screenshots, animated GIFs, README demo.
- Tarn `README.md` references the extension as the canonical editor experience.
- Version bumps in Tarn `Cargo.toml` and extension `package.json` are cut from one tag.

## Public API

The extension exposes a structured object to other extensions via `vscode.extensions.getExtension('nazarkalytiuk.tarn-vscode').exports`. That object conforms to `TarnExtensionApi`, defined in [`editors/vscode/src/api.ts`](../editors/vscode/src/api.ts). `api.ts` is the single source of truth — `extension.ts` re-exports the type but does not redeclare its shape.

### Obtaining the API

```ts
import type { TarnExtensionApi } from "nazarkalytiuk.tarn-vscode";
import * as vscode from "vscode";

const ext = vscode.extensions.getExtension<TarnExtensionApi>(
  "nazarkalytiuk.tarn-vscode",
);
if (!ext) {
  // extension not installed
  return;
}
const api = await ext.activate();
if (!api) {
  // activation was blocked (e.g. untrusted workspace)
  return;
}
```

`activate()` returns `undefined` in untrusted workspaces. Downstream integrators must handle that branch — the extension deliberately does not spawn Tarn, index files, or expose any surface until the user grants trust.

### Shape

| Field | Type | Stability | Description |
|---|---|---|---|
| `testControllerId` | `string` | **stable** | The `vscode.TestController` id used by the extension's Test Explorer integration. Other extensions can reference runs via the Testing API by looking up this id. |
| `indexedFileCount` | `number` | **stable** | Number of `.tarn.yaml` files tracked by the workspace index at the moment `activate()` resolved. Use the Testing API for live updates — this field is a one-shot snapshot. |
| `commands` | `readonly string[]` | **stable** | Full list of command ids the extension contributes. Useful for extensions that want to build their own palette or wire UI to Tarn actions without hard-coding command ids. The order of the array is not guaranteed. |
| `testing` | `TarnExtensionTestingApi` | **internal** | Opaque, test-only sub-object. No compatibility guarantees — its shape may change between any two releases (including patch releases) without a changelog entry. Exists solely for the extension's own `@vscode/test-electron` integration tests. Must not be used from production code. |

There are currently no `@stability preview` fields. When one is added, it will be listed in this table with a **preview** tier.

### Stability tiers

- **stable** — breaking changes require a major version bump (`1.x.y` → `2.0.0`). Removing a field, renaming a field, narrowing a return type, or widening a parameter type all count as breaking. Adding a new optional field to a stable object is NOT breaking.
- **preview** — may change in any minor release (`1.1.0` → `1.2.0`). Preview fields are shipped so integrators can experiment and give feedback before a field is promoted to stable. Always listed explicitly in the table above before you depend on one.
- **internal** — no compatibility guarantees whatsoever. Shape, presence, and behavior can change between any two releases, including patch releases. Downstream code that reads internal fields will break silently on upgrade. Do not use internal fields from production code.

### Semver policy in prose

The extension follows semantic versioning for its public API, not for its user-facing VS Code behavior. The user-facing side is free to iterate — adding a new command, changing a setting default, or renaming a view only needs a changelog entry, not a major bump. The public API side is frozen as described above.

Internal fields, and only internal fields, are allowed to change in patch releases. Every other level of change is bound by the stability tier of the affected field: preview bumps minor, stable bumps major. A change that touches both a stable field and an internal field is bound by the strictest tier, i.e. the stable field's major bump.

### 1.0.0 gate

Until the extension ships `1.0.0`, the stable surface is still subject to one last round of pruning. When `1.0.0` ships, every field currently marked `@stability stable` in `src/api.ts` is frozen under the semver policy above, and the set of stable fields is locked to whatever `api.ts` declares at tag time. The `0.x` → `1.0.0` cut is tracked as NAZ-288. Between now and then, the extension keeps shipping normal minor releases on the `0.x` track; the `1.0.0` cut is a deliberate, coordinated event.

### Enforcement

A CI-enforced golden-snapshot test at [`editors/vscode/tests/unit/apiSurface.test.ts`](../editors/vscode/tests/unit/apiSurface.test.ts) compares a normalized version of `src/api.ts` against [`editors/vscode/tests/golden/api.snapshot.txt`](../editors/vscode/tests/golden/api.snapshot.txt). Any edit to the interface declaration — adding a field, removing a field, renaming a field, changing a stability annotation, changing the semver policy prose, changing an imported type — fails the test unless the golden is updated in the same commit. The test is picked up by `npm run test:unit` and therefore runs on every PR.

The test also asserts three invariants:

1. Every `readonly` field of `TarnExtensionApi` carries a `@stability` annotation in its JSDoc.
2. The file-level semver-policy block comment mentions every stability tier (`stable`, `preview`, `internal`).
3. The `testing` sub-object is annotated `@stability internal`.

If a future PR tries to promote `testing.backend` (or any other internal field) into the public surface without annotating it, the test catches it locally before it ever reaches review. This is the "CI lint step that fails on unannounced breaking API changes" referenced by NAZ-285's acceptance criteria — the existing test pipeline is the lint step, and no separate GitHub Actions config is required.

See [`editors/vscode/docs/API.md`](../editors/vscode/docs/API.md) for a user-facing quick reference aimed at integrators rather than contributors.

## Testing Strategy

Follows the repo's testing guidance: every branch covered, tests must fail if the code path is broken.

Unit tests (vitest), pure functions only:

- `YamlAst` range queries for every fixture in `examples/`.
- `ResultMapper` against synthetic JSON reports covering every `failure_category` and `error_code`.
- `EnvService` against every permutation of the env resolution chain.
- `schemaGuards` zod schemas round-trip every `schemas/v1/report.json` example.
- `shellEscape` fuzzed against names with spaces, quotes, `$`, backticks, Unicode.
- `binaryResolver` for missing binary, version too old, custom path, Homebrew path, cargo path.

Integration tests (`@vscode/test-electron`) against a real `tarn` binary, using `examples/` and `research/tarn-vs-hurl/tarn/` as fixtures:

- Discovery produces the expected `TestItem` tree.
- Passing file marks `TestRun.passed` with correct durations.
- Failing file produces `TestMessage` with correct location, expected, actual.
- Run-at-cursor and selection-based runs target the right test.
- Cancellation kills the process.
- Concurrent runs per file are serialized.
- Env picker changes propagate to subsequent runs.
- Dry-run shows interpolated preview without network.
- Validate-on-save populates `Problems` at the correct ranges.

Performance tests: 1000 synthetic tests across 100 files. Discovery under 500 ms, result mapping under 200 ms for a full run, memory under 150 MB.

`cargo fmt && cargo clippy -- -D warnings && cargo test` run before every commit, matching `CLAUDE.md`. Extension side runs `npm run lint && npm run test && npm run build` before every commit.

## Packaging and Release

- `editors/vscode/` bundles with esbuild to a single `out/extension.js`. `.vscodeignore` keeps the VSIX under 500 KB. We do not ship the `tarn` binary; we detect or install.
- `engines.vscode` stays at `^1.90.0`. Testing API has been stable since `1.68.0`.
- CI publishes to VS Code Marketplace (`vsce publish`) and Open VSX (`ovsx publish`) from tagged releases.
- Extension patch versions ship independently of Tarn. Major versions align with Tarn feature parity.
- Release notes in `editors/vscode/CHANGELOG.md` link back to any Tarn release the version depends on.
- Signed VSIX via Microsoft signing pipeline once publisher is verified.

## Open Questions and Risks

1. Duplicate step names inside a single test break AST-key matching. Mitigation: index-based fallback, optional lint warning in `tarn fmt`.
2. Long polling steps need a live "attempt N of M" state. Needs a `poll_attempt` NDJSON event alongside §6.3.
3. Extension `--watch` vs Tarn `--watch` double-trigger. Decision: extension owns watching, Tarn `--watch` is never invoked by the extension.
4. `tarn-mcp` availability varies by release. Backend resolver falls back silently to CLI.
5. Custom token headers outside the default redact list can leak into TestMessage. Mitigation: `tarn.redactionExtraHeaders` merged into CLI flags and surfaced in the walkthrough.
6. Captures can contain PII. Captures Inspector respects redaction and exposes a hide-all toggle.
7. Subset runs without §6.4 may see stale cookie jars. Workaround: delete the jar file before subset runs and warn.
8. Lua script steps: no syntax highlighting or completion inside `script:` in v1. Deferred to v1.1.
9. Large response bodies truncate to 10 KB in TestMessage with an action to open the full body in the Request/Response panel.

## References

- `tarn/src/report/json.rs` — JSON report writer.
- `tarn/src/report/progress.rs` — streaming reporter trait the NDJSON backend plugs into.
- `tarn/src/assert/types.rs` — failure categories, error codes, result structs.
- `tarn/src/model.rs` — YAML data model.
- `tarn/src/runner.rs` — execution order.
- `tarn/src/env.rs` — environment resolution chain.
- `tarn/src/main.rs` — CLI surface.
- `schemas/v1/testfile.json`, `schemas/v1/report.json` — canonical schemas.
- `docs/MCP_WORKFLOW.md` — MCP backend option.
- `editors/vscode/README.md` — current declarative package.
- `plugin/skills/tarn-api-testing/references/json-output.md` — report field reference.
