# Changelog

## 0.5.0 — Phase 6: Coordinated release (NAZ-288)

First release of Tarn cut under the **coordinated-release** policy
introduced by NAZ-288: a single git tag (`v0.5.0`) now triggers both
the Rust binary pipeline (`.github/workflows/release.yml`) and the VS
Code extension publish pipeline (`.github/workflows/vscode-extension-release.yml`).
Both artifacts ship from the same commit, and both declare the same
version number.

Paired with **Tarn VS Code extension `0.5.0`** — see
[`editors/vscode/CHANGELOG.md`](editors/vscode/CHANGELOG.md) for the
matching extension release notes.

### Version alignment policy

Extension `X.Y.*` tracks Tarn `X.Y.*`: the minor number is always
identical, so a user on Tarn `0.5.x` knows any extension `0.5.x` is
tested against their CLI. Patch numbers may diverge — a hotfix to the
CLI can ship as Tarn `0.5.1` against extension `0.5.0` without a
matching extension bump, and vice versa. A new minor always bumps
both sides in lockstep.

The invariant is enforced by a unit test in the extension
(`editors/vscode/tests/unit/version.test.ts`) that cross-reads
`editors/vscode/package.json` and `tarn/Cargo.toml` on every CI pass
and fails the build if they drift. The extension also spawns
`tarn --version` at activation and warns the user if the installed
CLI is older than its declared `tarn.minVersion` field.

### Added

- **`tarn 0.5.0` is the first CLI release paired with a Marketplace
  extension drop.** All earlier CLI releases (`0.1.0 – 0.4.x`) shipped
  standalone with no Marketplace presence.
- **Phase 6 T-tickets bundled into 0.5.0** (shipped across prior
  commits, now cut as a coordinated release):
  - **T54** per-test cookie jar isolation (NAZ-259)
  - **T55** test-file location metadata on JSON report (NAZ-260)
  - **T57** scoped `tarn list --file` discovery (NAZ-261)
  - **T58** `--redact-header` flag (NAZ-262)
  See the `Unreleased` section below for the full per-ticket detail;
  that content has been promoted in this release.

### Changed

- **`tarn/Cargo.toml` version**: `0.4.4 → 0.5.0` (coordinated minor
  bump to join the extension alignment track).

## 0.1.0

- initial public Tarn release
- YAML-based API tests in `.tarn.yaml`
- structured JSON, JUnit, TAP, HTML, and human output
- setup/teardown, captures, cookies, includes, polling, retries, Lua scripting
- GraphQL support
- MCP server (`tarn-mcp`)
- benchmark mode (`tarn bench`)

## 0.4.0

### Bug Fixes

- **Unresolved template detection** (NAZ-233): steps using `{{ capture.x }}` or `{{ env.x }}` that failed to resolve now fail immediately with a clear error (`failure_category: "unresolved_template"`) instead of sending garbled requests with literal `%7B%7B` in URLs
- **Lua `json` global** (NAZ-231): `json.decode(string)` and `json.encode(value)` are now available in Lua scripts — previously `json` was nil at runtime
- **MCP env var resolution** (NAZ-232): `tarn_run` MCP tool now resolves `tarn.env.yaml` from the project root (matching CLI behavior) instead of only looking in the test file's directory

### Improvements

- **AI-optimized JSON output** (NAZ-235, NAZ-234):
  - `response_status` and `response_summary` fields on all steps (passed and failed) — AI agents can see what a passed step returned without forcing a failure
  - `captures_set` field on steps listing which capture variables were set
  - `captures` map on test groups showing all captured values at end of test
  - Response bodies truncated to ~200 chars in `--json-mode compact`
  - `response_summary` provides brief descriptions like `"200 OK: Array[20]"` or `"403 Forbidden: error message"`
- **JSONPath array search** (NAZ-230): documented and tested that wildcard paths (`$[*].field`) with `contains` and filter expressions (`$[?@.field == 'value']`) work in poll `until` assertions for searching object arrays

### Schema

- Added `unresolved_template` to `failureCategory` enum
- Added optional `response_status`, `response_summary`, `captures_set` to step results
- Added optional `captures` to test results

## Unreleased

- **Per-test cookie jar isolation** (NAZ-259): new `cookies: "per-test"` file-level mode and `--cookie-jar-per-test` CLI flag clear the default cookie jar between named tests within a file so IDE subset runs and flaky integration suites never see session state from a prior test. Setup and teardown still share the file-level jar. Named cookie jars (multi-user scenarios) are untouched. The CLI flag overrides whatever the file declares, except when the file sets `cookies: "off"` — that always wins. Unknown `cookies:` values now fail parsing with a clear error instead of silently falling back to auto.
- **`tarn validate --format json`**: structured validation output for editors and CI. Emits `{"files": [{"file", "valid", "errors": [{"message", "line", "column"}]}]}`. YAML syntax errors include precise `line` and `column` extracted from `serde_yaml`. Parser semantic errors fall back to `message`-only when no location is known (`line`/`column` are optional). Exit codes unchanged: `0` when every file is valid, `2` otherwise. Unknown format values are rejected with exit `2`. The human format (the default) is unchanged.
- **`tarn env --json` schema polish + redaction**: inline vars declared in `tarn.config.yaml` environments are now redacted when they match `redaction.env` (case-insensitive) so `tarn env --json` never prints literal secrets. Renamed the per-environment file field from `env_file` to `source_file` for consistency with the VS Code extension contract. Environments are sorted alphabetically. Exit code stays `0` on success, `2` on configuration error. Human output is unchanged.
- **`--ndjson` flag**: `tarn run --ndjson` streams machine-readable events to stdout, one JSON object per line. Events: `file_started`, `step_finished` (per step, with `phase` set to `setup` / `test` / `teardown`), `test_finished`, `file_finished`, and a final `done` event carrying the aggregated summary. Failing `step_finished` events include `failure_category`, `error_code`, and `assertion_failures`. Composes with `--format json=path` to write the final report to a file while streaming NDJSON on stdout. In parallel mode, each file's event stream is emitted atomically on `file_finished` to avoid interleaving across files. The default human format is silently suppressed on stdout when `--ndjson` is set; other stdout-bound formats raise an error. Primary consumer: the VS Code extension's live Test Explorer updates.
- **`--select` flag**: `tarn run --select FILE[::TEST[::STEP]]` narrows execution to specific files, tests, or steps. Repeatable (multiple selectors union). ANDs with `--tag`. STEP accepts either a name or a 0-based integer index. Step selection runs only that step with no prior steps — captures from earlier steps will be unset, so prefer test-level selectors for chained flows. Enables editor-driven "run test at cursor" and "rerun failed" workflows.
- **Streaming progress output**: `tarn run` now prints results as each test (sequential) or file (parallel) finishes instead of dumping everything at the end. When stdout is `--format human` the stream writes directly to stdout; when stdout is a structured format (`json`, `junit`, `tap`, etc.) the stream goes to stderr so stdout stays parseable. Parallel mode buffers per file and emits each file atomically to avoid interleaving. Add `--no-progress` to restore batch-only output.
- **`--only-failed` flag**: `tarn run --only-failed` hides passing tests and steps from human and JSON output, keeping only the failures. Summary counts still reflect the full run. Works with streaming too.
- transport and runtime parity work: proxy, TLS controls, redirects, HTTP version selection, richer cookies, form support, custom methods
- richer assertion/capture surface: whole-body diffs, more format/hash operators, status/url/header/cookie/body captures, transform-lite pipeline
- machine-oriented diagnostics: `error_code`, remediation hints, compact/verbose JSON, curl export, richer HTML, golden reporter coverage
- product DX: VS Code extension, `tarn fmt`, improved `tarn init`, docs site, Hurl migration guide, conservative Hurl importer
- project workflow: config defaults/redaction/environments, include params and overrides, auth helpers, impacted watch mode, public conformance suite
- benchmark upgrades: thresholds, exports, and timing breakdowns
