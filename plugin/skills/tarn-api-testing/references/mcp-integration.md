# Tarn MCP Integration Reference

Tarn ships with `tarn-mcp`, an MCP (Model Context Protocol) server that lets AI agents run, validate, and inspect API tests directly.

## Setup

### Claude Code

Add to `.claude/settings.json` in the project root:

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

### Cursor

Add to `.cursor/mcp.json`:

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

### Windsurf

Add to `.windsurf/mcp.json`:

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

**Prerequisite:** `tarn-mcp` binary must be in `$PATH`. Build with `cargo build --release -p tarn-mcp`.

## Available Tools

### tarn_run

Run API tests and return structured JSON results.

**Parameters:**
- `file` (optional) — path to a specific `.tarn.yaml` file; omit to run all
- `env` (optional) — environment name (maps to `tarn.env.{name}.yaml`)
- `tag` (optional) — run only tests matching this tag
- `vars` (optional) — key=value overrides

**Returns:** Full JSON report matching `schemas/v1/report.json`.

### tarn_validate

Validate YAML syntax without executing HTTP requests.

**Parameters:**
- `file` (optional) — path to validate; omit for all files

**Returns:** Validation result with any parse errors and their locations.

### tarn_list

List all available test files, test groups, and steps.

**Parameters:**
- `file` (optional) — list steps for a specific file

**Returns:** Structured listing of tests and steps.

### tarn_fix_plan

Generate a fix plan for failed test results.

**Parameters:**
- `report` — JSON report from a failed `tarn_run`

**Returns:** Structured remediation plan with suggested fixes per failed step.

## Recommended Agent Loop

```
1. After generating or editing .tarn.yaml → call tarn_validate
2. If validation passes → call tarn_run
3. Read summary.status
4. If FAILED:
   a. Find failed steps → read failure_category
   b. Read assertions.failures[] for expected vs actual
   c. If request.url contains unresolved {{ }} → fix env/capture
   d. Optionally call tarn_fix_plan for structured remediation
   e. Fix YAML or application code
   f. Go to step 1
5. If PASSED → done
```

## When to Use MCP vs CLI

**Use MCP (tarn_run tool)** when:
- Working inside Claude Code, Cursor, or Windsurf
- You want structured JSON returned directly to the agent context
- Iterating on test failures in an agent loop

**Use CLI directly** when:
- Running in CI/CD pipelines
- You need specific output formats (junit, tap, html)
- Running benchmarks or using advanced CLI flags
- Human is reading the output directly

## Key Fields to Focus On

When processing `tarn_run` results, prioritize these fields:

1. `summary.status` — overall pass/fail
2. `files[].tests[].steps[].failure_category` — why a step failed
3. `files[].tests[].steps[].assertions.failures[]` — what exactly was wrong
4. `files[].tests[].steps[].request.url` — check for unresolved templates
5. `files[].tests[].steps[].response.body` — actual server response
6. `files[].tests[].steps[].remediation_hints` — suggested fixes
