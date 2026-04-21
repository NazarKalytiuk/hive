# Tarn AI Workflow Demo

This is the shortest text-first demo of Tarn's core promise: an agent writes a test, Tarn returns structured failures, and the agent fixes the exact mismatch instead of guessing from stdout.

## Goal

Generate a test from an endpoint description, run Tarn, read the structured failure set, drill into one step, fix it, rerun only the failing subset, and confirm with a run diff.

## The failures-first loop

Default to this sequence. It keeps the agent off the megabyte-scale full report until it is strictly necessary.

```
1. tarn validate <path>                  # syntax/config before running
2. tarn run <path>                       # produces .tarn/runs/<run_id>/
3. tarn failures                         # root-cause groups; cascades collapsed
4. tarn inspect last FILE::TEST::STEP    # full context for ONE failure
5. Patch tests or application code
6. tarn rerun --failed                   # reruns only the failing subset
7. tarn diff prev last                   # confirm fixed / new / persistent
8. Full report.json only when 3–6 are insufficient
```

## Example Prompt

```text
Write a Tarn test for GET /health on http://127.0.0.1:3000.
It should expect status 200 and body.status == "ok".
```

## Generated Test

```yaml
name: Health check
steps:
  - name: GET /health
    request:
      method: GET
      url: "http://127.0.0.1:3000/health"
    assert:
      status: 201
      body:
        "$.status": "ok"
```

## Run Tarn

```bash
tarn validate health.tarn.yaml
tarn run health.tarn.yaml
```

The run writes `report.json`, `summary.json`, and `failures.json` under `.tarn/runs/<run_id>/` (with `.tarn/last-run.json`, `.tarn/summary.json`, and `.tarn/failures.json` as latest-run pointers). The CLI prints `run id:` and `run artifacts:` on stderr at the end.

## Triage with `tarn failures`

Read the root cause first, not the full report:

```bash
tarn failures --format json
```

The output groups cascade skips (`skipped_due_to_failed_capture`) under their upstream root cause, so one failing step with five downstream skips surfaces as one entry with `cascades: 5`, not six. Cascade entries stay collapsed unless you pass `--include-cascades`.

## Drill into one failing step

When you already know *which* step to open, go straight there — do not grep the full report:

```bash
tarn inspect last health.tarn.yaml::Health\ check::0 --format json
```

`last` (or `latest`, or `@latest`) resolves to the most recent archive; `prev` resolves to the one before. The step-level view returns the same `failure_category` / `assertions` / `request` / `response` block shown below without having to parse the whole `report.json`.

## Failure JSON Excerpt

```json
{
  "failure_category": "assertion_failed",
  "error_code": "STATUS_MISMATCH",
  "remediation_hints": [
    "Compare the expected status with the actual response status."
  ],
  "assertions": {
    "failures": [
      {
        "assertion": "status",
        "expected": "201",
        "actual": "200"
      }
    ]
  },
  "request": {
    "method": "GET",
    "url": "http://127.0.0.1:3000/health"
  },
  "response": {
    "status": 200,
    "body": {
      "status": "ok"
    }
  }
}
```

## Agent Diagnosis

- the request reached the correct endpoint
- the body assertion already passes
- the only mismatch is the expected status

At this point the agent can either patch the file directly or call `tarn_fix_plan` over the latest report.

## Fix

```yaml
status: 200
```

## Rerun only the failing subset

Do not re-execute the whole suite to confirm a fix. Replay just the failing `(file, test)` pairs:

```bash
tarn rerun --failed
```

`tarn rerun --failed` reads `.tarn/failures.json` (or `--run <id>` for a historical archive), runs the same tests, and writes a fresh archive under `.tarn/runs/<new_run_id>/`. The new report stamps its origin at `rerun_source: {run_id, source_path, selected_count}` so automation can chain reruns.

## Confirm with a run diff

```bash
tarn diff prev last --format json
```

The diff buckets failure fingerprints into `new` (only in the latest run), `fixed` (only in the prior run), and `persistent` (still present). A successful patch shows the root-cause fingerprint moving to `fixed` and an empty `new` array. Cascade-collapsed grouping is reused from `tarn failures`, so you never double-count a single root cause.

Expected summary on `tarn inspect last` once the patch lands:

```json
{
  "summary": {
    "status": "PASSED"
  }
}
```

## Tips for Large Suites

When the agent is iterating on a suite with hundreds of tests, two flags keep the feedback loop tight:

- `--only-failed` prunes passing files, tests, and steps from both human and JSON output. Summary counts still reflect the full run, so CI reports stay accurate, but the agent only has to read the failures it needs to fix.
- Progress streaming is on by default: with `--format json` the structured report goes to stdout and per-test progress lines go to stderr, so the agent can tail stderr for liveness while still parsing stdout at the end. Use `--no-progress` if a CI harness already timestamps every stdout line and you prefer the classic batch dump.

```bash
# CI-friendly: show only failures in JSON, no stderr noise
tarn run --only-failed --no-progress --format json

# Interactive debugging: stream progress to stderr, final JSON to stdout
tarn run --only-failed --format json
```
