# Tarn MCP Workflow

Tarn's MCP server exists to keep the agent on a structured tool surface instead of asking it to parse shell output.

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
