# Changelog

## 0.2.0 — Phase 1 foundation

Adds extension host integration on top of the existing declarative package.

### Added

- Test Explorer integration via the VS Code Testing API.
  - Hierarchical discovery: workspace → file → test → step.
  - `Run` and `Dry Run` test run profiles.
  - Cancellation via SIGINT with SIGKILL fallback after 2 s, plus a configurable watchdog timeout.
  - Result mapping from `tarn run --format json --json-mode verbose` into `TestRun.passed / failed`.
  - Rich failure `TestMessage` with expected/actual, unified diff, request, response, remediation hints, and failure category/error code.
- CodeLens above each test and step: `Run`, `Dry Run`, `Run step`.
- Document symbol provider: outline view of tests, steps, setup, teardown with scope-aware hierarchy.
- Tarn activity bar container with a **Run History** tree view persisting the last 20 runs (status, env, tags, duration, files).
- Status bar entries: active environment (click to pick) and last run summary (click to open output).
- Commands:
  - `Tarn: Run All Tests`
  - `Tarn: Run Current File`
  - `Tarn: Dry Run Current File`
  - `Tarn: Validate Current File`
  - `Tarn: Rerun Last Run`
  - `Tarn: Select Environment…`
  - `Tarn: Set Tag Filter…`, `Tarn: Clear Tag Filter`
  - `Tarn: Export Current File as curl` (all or failed-only via `--format curl-all` / `--format curl`)
  - `Tarn: Clear Run History`
  - `Tarn: Open Getting Started`
  - `Tarn: Show Output`
  - `Tarn: Install / Update Tarn`
- **Getting Started walkthrough** with five steps: install, open example, run, select env, inspect failure.
- Workspace indexing with on-change reparsing via `FileSystemWatcher`, idempotent initialization.
- YAML AST with range maps for tests, steps, setup, and teardown — foundation for CodeLens, document symbols, result anchoring, and future authoring features.
- Settings namespace `tarn.*` with 13 keys covering binary path, discovery globs, parallelism, JSON mode, timeouts, redaction passthrough, and UI toggles.
- Workspace trust gating: untrusted workspaces keep grammar, snippets, and schema wiring but do not spawn the Tarn binary.
- Shell-free process spawning via Node's built-in `child_process.spawn` with an argv array, plus a log formatter for copyable command lines in the output channel.
- Zod-validated parsing of Tarn JSON reports.

### Tests

- **Unit tests** (vitest, 76 tests across 5 files):
  - `shellEscape` — safe identifier passthrough, space/quote/dollar/backtick escaping.
  - `schemaGuards` — passing report, failing report with full rich detail, enum rejection, missing-field rejection.
  - `YamlAst` — file name, tests, steps, setup, teardown, flat `steps`, malformed input.
  - `YamlAstSweep` — parses every `.tarn.yaml` fixture in `examples/` and verifies non-empty names plus non-negative ranges (55 dynamic tests).
  - `ResultMapper.buildFailureMessages` — rich assertion failure, multi-assertion, generic fallback, and every `failure_category` enum value.
- **Integration tests** (`@vscode/test-electron` + mocha): smoke suite covering activation, test controller registration, discovery of a fixture workspace, document symbols, and command registration. Runs via `npm run test:integration`.

### CI

- GitHub Actions workflow `.github/workflows/vscode-extension.yml` running typecheck, unit tests, and build across `ubuntu-latest`, `macos-latest`, `windows-latest`; Ubuntu job also packages a VSIX artifact.

### Preserved from 0.1.0

- Language id `tarn` for `*.tarn.yaml` / `*.tarn.yml`.
- Grammar at `syntaxes/tarn.tmLanguage.json`.
- Snippets (`tarn-test`, `tarn-step`, `tarn-capture`, `tarn-poll`, `tarn-form`, `tarn-graphql`, `tarn-multipart`, `tarn-lifecycle`, `tarn-include`).
- Schema wiring for test files and report files via `redhat.vscode-yaml`.

### Known gaps (tracked in `docs/VSCODE_EXTENSION.md` and `T51`–`T57`)

- Streaming progress requires Tarn NDJSON reporter (`T53`); Phase 1 uses the final JSON report.
- Run-at-cursor and run-failed-only require selective execution (`T51`).
- Structured validation diagnostics require `tarn validate --format json` (`T52`).
- Runtime result ranges are AST-inferred until Tarn exposes location metadata (`T55`).

## 0.1.0

Initial declarative package: language id, grammar, snippets, schema wiring.
