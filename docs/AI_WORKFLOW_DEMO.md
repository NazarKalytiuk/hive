# Tarn AI Workflow Demo

This is the shortest text-first demo of Tarn's core promise: an agent writes a test, Tarn returns structured failures, and the agent fixes the exact mismatch instead of guessing from stdout.

## Goal

Generate a test from an endpoint description, run Tarn, inspect structured JSON, fix the test, rerun green.

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
tarn run health.tarn.yaml --format json --json-mode compact
```

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

## Rerun

```bash
tarn run health.tarn.yaml --format json --json-mode compact
```

Expected summary:

```json
{
  "summary": {
    "status": "PASSED"
  }
}
```
