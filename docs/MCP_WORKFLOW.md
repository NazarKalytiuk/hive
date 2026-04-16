# Tarn MCP Workflow

Tarn's MCP server exists to keep the agent on a structured tool surface instead of asking it to parse shell output.

## Setup

The recommended setup is a project-level `.mcp.json` in the repo root:

```json
{
  "mcpServers": {
    "tarn": {
      "command": "tarn-mcp",
      "args": []
    }
  }
}
```

This is portable across Claude Code, Cursor, Windsurf, and any other client that reads `.mcp.json`. **opencode is the exception** — it uses its own `opencode.jsonc` schema (`mcp.tarn` with `type: "local"` and a `command` array). See `plugin/skills/tarn-api-testing/references/mcp-integration.md` for the opencode snippet and the other editor-specific alternatives (`.claude/settings.json`, `.cursor/mcp.json`, `.windsurf/mcp.json`).

## Agent Skill

The `plugin/skills/tarn-api-testing/` directory is the canonical home for the Tarn agent skill — structured knowledge about Tarn's workflow, commands, test file format, assertions, captures, and failure diagnosis. Two agents pick it up today:

- **Claude Code** loads it from the `tarn` plugin (the plugin ships `plugin/skills/tarn-api-testing/` as its skill directory).
- **opencode** loads it from `.opencode/skills/tarn-api-testing/`, which this repo provides as a relative symlink to the canonical path above — no content duplication.

## Available Tools

- `tarn_list`
- `tarn_validate`
- `tarn_run`
- `tarn_fix_plan`

## Recommended Loop

1. Call `tarn_list` to discover tests and steps.
2. Call `tarn_validate` after generating or editing `.tarn.yaml`.
3. Call `tarn_run` and inspect the structured report.
4. Branch first on `failure_category` and `error_code`.
5. Use `tarn_fix_plan` when you want prioritized next actions from the latest report.
6. Edit the test or the application code.
7. Rerun until `summary.status` is `PASSED`.

> **Editor consumers:** `tarn_fix_plan` is backed by the same `tarn::fix_plan` library surface that powers `tarn-lsp`'s `CodeActionKind::QUICKFIX` **Apply fix** code action (NAZ-305, L3.4). The MCP tool uses the report-driven path for prioritised advice; the LSP uses the diagnostic-driven path for structured edits that clients apply with one click. See [`docs/TARN_LSP.md`](./TARN_LSP.md#apply-fix-quickfix--new-in-l34) for the LSP-side contract.

## Fields That Matter Most

Focus on these first:

- `failure_category`
- `error_code`
- `remediation_hints`
- `assertions.failures`
- optional failed-step `request`
- optional failed-step `response`

That ordering is deliberate. It keeps the agent from patching assertions before it understands whether the failure is parse, connection, timeout, capture, or a plain mismatch.

## Why MCP Instead of Shelling Out

- no stdout scraping
- fewer quoting and path-resolution mistakes
- smaller tool surface for the model
- direct structured results instead of ad hoc parsing

## When Plain CLI Is Still Fine

Use the CLI directly when:

- you are in CI
- you do not want MCP setup overhead
- you want report files such as `json`, `html`, `junit`, or `curl`

The equivalent fallback is:

```bash
tarn run --format json --json-mode compact
```

The report contract is the same idea either way: machine-readable, versioned, and intentionally stable.
