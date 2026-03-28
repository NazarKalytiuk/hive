<p align="center">
  <strong>Hive</strong><br>
  <em>CLI-first API testing tool written in Rust</em>
</p>

<p align="center">
  <a href="https://github.com/NazarKalytiuk/hive/actions/workflows/ci.yml"><img src="https://github.com/NazarKalytiuk/hive/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/NazarKalytiuk/hive/releases/latest"><img src="https://img.shields.io/github/v/release/NazarKalytiuk/hive" alt="Release"></a>
  <a href="https://github.com/NazarKalytiuk/hive/blob/main/LICENSE"><img src="https://img.shields.io/badge/license-MIT-blue" alt="License"></a>
</p>

---

Tests are defined in `.hive.yaml` files &mdash; no code, no custom DSL. Simple enough that an LLM generates valid tests on the first try. Structured JSON output that an LLM can parse and iterate on.

```yaml
name: Health check
steps:
  - name: GET /health
    request:
      method: GET
      url: "{{ env.base_url }}/health"
    assert:
      status: 200
```

```
$ hive run
 HIVE  Running tests/health.hive.yaml

 ● Health check

   ✓ GET /health (4ms)

 Results: 1 passed (15ms)
```

## Install

```bash
# One-liner (macOS / Linux)
curl -fsSL https://raw.githubusercontent.com/NazarKalytiuk/hive/main/install.sh | sh

# Or build from source
cargo install --git https://github.com/NazarKalytiuk/hive.git --bin hive
```

Binaries are available for **macOS** (Intel & Apple Silicon) and **Linux** (amd64 & arm64) on the [releases page](https://github.com/NazarKalytiuk/hive/releases).

## Quick Start

```bash
# Initialize a project
hive init

# This creates:
#   tests/health.hive.yaml   — example test
#   hive.env.yaml             — environment variables
#   hive.config.yaml          — project config

# Run all tests
hive run

# Run a specific file
hive run tests/users/crud.hive.yaml

# Run with a specific environment
hive run --env staging
```

## Table of Contents

- [Test File Format](#test-file-format)
  - [Minimal Test](#minimal-test)
  - [Full Format](#full-format)
  - [Setup and Teardown](#setup-and-teardown)
- [Assertions](#assertions)
  - [Status](#status)
  - [Body (JSONPath)](#body-jsonpath)
  - [Headers](#headers)
  - [Duration](#duration)
- [Variables](#variables)
  - [Environment Variables](#environment-variables)
  - [Captures (Chaining)](#captures-chaining)
  - [Built-in Functions](#built-in-functions)
- [CLI Reference](#cli-reference)
- [Output Formats](#output-formats)
- [Performance Testing](#performance-testing)
- [Configuration](#configuration)
- [Step Options](#step-options)
- [Shell Completions](#shell-completions)
- [Project Structure](#project-structure)
- [Development](#development)

## Test File Format

Test files use the `.hive.yaml` extension and can be organized in any directory structure.

```
tests/
  health.hive.yaml
  users/
    crud.hive.yaml
    validation.hive.yaml
  auth/
    login.hive.yaml
```

### Minimal Test

Three lines for a GET request + status check:

```yaml
name: Health check
steps:
  - name: GET /health
    request:
      method: GET
      url: "http://localhost:3000/health"
    assert:
      status: 200
```

### Full Format

```yaml
version: "1"
name: "User CRUD Operations"
description: "Tests complete user lifecycle"
tags: [crud, users, smoke]

env:
  base_url: "http://localhost:3000/api/v1"
  admin_email: "admin@example.com"

defaults:
  headers:
    Content-Type: "application/json"
    Accept: "application/json"
  timeout: 5000
  retries: 1

tests:
  create_and_verify:
    description: "Create a user, then verify it exists"
    tags: [smoke]
    steps:
      - name: Create user
        request:
          method: POST
          url: "{{ env.base_url }}/users"
          body:
            name: "Jane Doe"
            email: "jane.{{ $random_hex(6) }}@example.com"
        capture:
          user_id: "$.id"
        assert:
          status: 201
          body:
            "$.name": "Jane Doe"
            "$.id": { type: string, not_empty: true }

      - name: Verify user
        request:
          method: GET
          url: "{{ env.base_url }}/users/{{ capture.user_id }}"
        assert:
          status: 200
          body:
            "$.id": "{{ capture.user_id }}"
```

### Setup and Teardown

`setup` runs once before all tests. `teardown` runs after all tests **even if tests fail**.

```yaml
name: "CRUD with auth"

setup:
  - name: Login
    request:
      method: POST
      url: "{{ env.base_url }}/auth/login"
      body:
        email: "{{ env.admin_email }}"
        password: "{{ env.admin_password }}"
    capture:
      auth_token: "$.token"

teardown:
  - name: Cleanup
    request:
      method: POST
      url: "{{ env.base_url }}/test/cleanup"

tests:
  my_test:
    steps:
      - name: Authenticated request
        request:
          method: GET
          url: "{{ env.base_url }}/users"
          headers:
            Authorization: "Bearer {{ capture.auth_token }}"
        assert:
          status: 200
```

## Assertions

### Status

```yaml
assert:
  status: 200
```

### Body (JSONPath)

All assertions use [JSONPath](https://www.rfc-editor.org/rfc/rfc9535) expressions to select values from the response body.

**Equality:**

```yaml
body:
  "$.name": "Alice"              # string equality
  "$.age": 30                    # number equality
  "$.active": true               # boolean
  "$.deletedAt": null            # null check
  "$.field": { eq: "value" }     # explicit equality
  "$.field": { not_eq: "bad" }   # inequality
```

**Numeric comparisons:**

```yaml
body:
  "$.age": { gt: 18 }            # greater than
  "$.age": { gte: 18 }           # greater than or equal
  "$.count": { lt: 100 }         # less than
  "$.count": { lte: 100 }        # less than or equal
```

**String assertions:**

```yaml
body:
  "$.email": { contains: "@example.com" }
  "$.id": { starts_with: "usr_" }
  "$.file": { ends_with: ".pdf" }
  "$.id": { matches: "^usr_[a-z0-9]+$" }   # regex
  "$.name": { not_empty: true }
  "$.code": { length: 6 }
  "$.msg": { not_contains: "error" }
```

**Type checks:**

```yaml
body:
  "$.name": { type: string }
  "$.age": { type: number }
  "$.active": { type: boolean }
  "$.tags": { type: array }
  "$.meta": { type: object }
  "$.deleted": { type: "null" }
```

**Array assertions:**

```yaml
body:
  "$.tags": { length: 3 }
  "$.items": { length_gt: 0 }
  "$.items": { length_gte: 1 }
  "$.items": { length_lte: 100 }
  "$.tags": { contains: "admin" }
  "$.tags": { not_contains: "banned" }
```

**Existence:**

```yaml
body:
  "$.id": { exists: true }            # field present (value can be null)
  "$.internal": { exists: false }     # field absent
```

**Combined (AND logic):**

```yaml
body:
  "$.id": { type: string, not_empty: true, starts_with: "usr_" }
```

### Headers

```yaml
assert:
  headers:
    content-type: "application/json"                    # exact match
    content-type: contains "application/json"           # substring
    x-request-id: matches "^[a-f0-9-]{36}$"            # regex
```

Header names are matched case-insensitively.

### Duration

```yaml
assert:
  duration: "< 500ms"      # less than 500ms
  duration: "<= 1s"        # at most 1 second
  duration: "> 100ms"      # more than 100ms
```

## Variables

### Environment Variables

Variables can come from multiple sources (highest priority wins):

| Priority | Source | Example |
|----------|--------|---------|
| 1 (highest) | CLI `--var` | `--var base_url=http://staging` |
| 2 | Shell env `${VAR}` | `password: "${ADMIN_PASSWORD}"` |
| 3 | `hive.env.local.yaml` | (gitignored, for secrets) |
| 4 | `hive.env.{name}.yaml` | `--env staging` loads this |
| 5 | `hive.env.yaml` | default env file |
| 6 (lowest) | Inline `env:` block | in the test file itself |

**Usage in templates:**

```yaml
env:
  base_url: "http://localhost:3000"

steps:
  - name: test
    request:
      method: GET
      url: "{{ env.base_url }}/health"
```

**Environment files:**

```yaml
# hive.env.yaml (committed — no secrets)
base_url: "http://localhost:3000"
admin_email: "admin@example.com"

# hive.env.staging.yaml (per-environment)
base_url: "https://staging-api.example.com"

# hive.env.local.yaml (gitignored — secrets)
admin_password: "s3cret"
```

### Captures (Chaining)

Extract values from responses to use in later steps:

```yaml
steps:
  - name: Login
    request:
      method: POST
      url: "{{ env.base_url }}/auth/login"
      body:
        email: "admin@example.com"
        password: "password123"
    capture:
      token: "$.token"           # JSONPath expression

  - name: Use token
    request:
      method: GET
      url: "{{ env.base_url }}/users"
      headers:
        Authorization: "Bearer {{ capture.token }}"
```

Captures use JSONPath expressions and support nested paths (`$.user.profile.id`), array indexing (`$.items[0].id`), etc.

### Built-in Functions

```yaml
"{{ $uuid }}"                    # UUID v4
"{{ $random_hex(8) }}"           # 8-char hex string
"{{ $random_int(1, 100) }}"      # random integer in range
"{{ $timestamp }}"               # current unix timestamp
"{{ $now_iso }}"                 # ISO 8601 datetime
```

**Example:**

```yaml
body:
  email: "user_{{ $random_hex(6) }}@example.com"
```

## CLI Reference

```
hive run [PATH] [OPTIONS]          Run test files
hive bench <PATH> [OPTIONS]        Benchmark a step with concurrent requests
hive validate [PATH]               Validate YAML without running
hive list                          List all tests (dry run)
hive init                          Scaffold a new project
hive completions <SHELL>           Generate shell completions
```

### `hive run` Options

| Flag | Description |
|------|-------------|
| `--format <FORMAT>` | Output format: `human` (default), `json`, `junit`, `tap`, `html` |
| `--tag <TAGS>` | Filter by tag (comma-separated, AND logic) |
| `--var <KEY=VALUE>` | Override env variables (repeatable) |
| `--env <NAME>` | Load `hive.env.{name}.yaml` |
| `-v, --verbose` | Print full request/response for every step |
| `--dry-run` | Show interpolated requests without sending |

### Examples

```bash
# Run all tests in tests/ directory
hive run

# Run one file
hive run tests/auth.hive.yaml

# Run only smoke tests
hive run --tag smoke

# Run with staging environment
hive run --env staging

# Override a variable
hive run --var base_url=http://localhost:8080

# JSON output for CI/LLM consumption
hive run --format json

# HTML dashboard (auto-opens in browser)
hive run --format html

# Debug: see all requests/responses
hive run -v

# Preview without sending requests
hive run --dry-run

# Benchmark an endpoint (100 requests, 10 concurrent)
hive bench tests/health.hive.yaml -n 100 -c 10
```

### Exit Codes

| Code | Meaning |
|------|---------|
| `0` | All tests passed |
| `1` | One or more tests failed |
| `2` | Configuration/parse error |
| `3` | Runtime error (network, timeout) |

## Output Formats

### Human (default)

Colored terminal output with pass/fail indicators and failure details.

### JSON (`--format json`)

Structured JSON for programmatic consumption. Key design decisions:
- Full request/response included **only for failed steps** (keeps output compact)
- Every failed assertion has `expected`, `actual`, and `message` fields
- Secrets in headers are redacted to `***`

### JUnit XML (`--format junit`)

Standard JUnit XML for CI/CD systems (Jenkins, GitHub Actions, etc.)

### TAP (`--format tap`)

[Test Anything Protocol](https://testanything.org/) v13 format.

### HTML (`--format html`)

Self-contained HTML dashboard with:
- Pass/fail summary with progress bar
- Expandable test files and test groups
- Per-assertion details (click to expand)
- Failure diffs with expected vs actual
- Request/response viewer for failed steps
- Dark theme, auto-opens in browser

## Performance Testing

Hive includes a built-in benchmarking tool that reuses your existing test files. No new format to learn.

### `hive bench`

```bash
hive bench <FILE> [OPTIONS]
```

| Flag | Description |
|------|-------------|
| `-n, --requests <N>` | Total number of requests (default: 100) |
| `-c, --concurrency <N>` | Concurrent workers (default: 10) |
| `--step <INDEX>` | Step index to benchmark, 0-based (default: 0) |
| `--ramp-up <DURATION>` | Gradually add workers over this duration (`"5s"`, `"500ms"`) |
| `--var <KEY=VALUE>` | Override env variables |
| `--env <NAME>` | Load environment file |
| `--format <FORMAT>` | Output: `human` (default) or `json` |

### Examples

```bash
# Basic benchmark: 100 requests, 10 concurrent
hive bench tests/health.hive.yaml -n 100 -c 10

# Heavy load: 1000 requests, 50 concurrent
hive bench tests/health.hive.yaml -n 1000 -c 50

# Gradual ramp-up over 5 seconds
hive bench tests/health.hive.yaml -n 500 -c 25 --ramp-up 5s

# Benchmark a specific step (e.g., the 3rd step in a multi-step file)
hive bench tests/crud.hive.yaml --step 2 -n 100 -c 10

# JSON output for CI threshold checks
hive bench tests/health.hive.yaml -n 200 -c 20 --format json
```

### Output

```
 HIVE BENCH  GET http://localhost:3000/health — 200 requests, 200 concurrent

  Requests:      200 total, 200 ok, 0 failed (0.0%)
  Duration:      64ms
  Throughput:    3125.0 req/s

  Latency:
    min        1ms
    p50        2ms
    p95        43ms
    p99        45ms
    max        45ms
    stdev      12.60ms

  Status codes:
    200 — 200 responses
```

### JSON Output

With `--format json`, the output is a structured object suitable for CI pipelines:

```json
{
  "step_name": "GET /health",
  "total_requests": 200,
  "successful": 200,
  "failed": 0,
  "error_rate": 0.0,
  "throughput_rps": 3125.0,
  "latency": {
    "min_ms": 1,
    "median_ms": 2,
    "p95_ms": 43,
    "p99_ms": 45,
    "max_ms": 45,
    "stdev_ms": 12.60
  },
  "status_codes": { "200": 200 },
  "errors": []
}
```

### How It Works

- Uses **async concurrent workers** (tokio + async reqwest) for true parallelism
- Assertions from the test file are evaluated &mdash; only requests matching the expected status count as "successful"
- Latency stats are computed from successful requests only
- Status code distribution and unique errors are tracked
- Ramp-up gradually introduces workers to avoid thundering herd on cold starts

## Configuration

### `hive.config.yaml` (optional)

```yaml
test_dir: "tests"           # where to find .hive.yaml files
env_file: "hive.env.yaml"   # default env file
timeout: 10000              # global default timeout (ms)
retries: 0                  # default retries for all steps
parallel: false             # run test files in parallel (future)
```

### File-level defaults

Applied to every request in the file:

```yaml
defaults:
  headers:
    Content-Type: "application/json"
    Accept: "application/json"
  timeout: 5000
  retries: 1
```

## Step Options

### Retries

Retry failed steps automatically:

```yaml
steps:
  - name: Flaky endpoint
    request:
      method: GET
      url: "{{ env.base_url }}/sometimes-fails"
    retries: 3             # retry up to 3 times on failure
    assert:
      status: 200
```

Retries can also be set globally via `defaults.retries`.

### Timeout

Override the default timeout per step:

```yaml
steps:
  - name: Slow report
    request:
      method: GET
      url: "{{ env.base_url }}/generate-report"
    timeout: 30000         # 30 seconds for this step
    assert:
      status: 200
```

### Delay

Pause before executing a step:

```yaml
steps:
  - name: Wait then check
    delay: "2s"            # wait 2 seconds before running
    request:
      method: GET
      url: "{{ env.base_url }}/async-result"
    assert:
      status: 200
```

Supports `ms` and `s` units: `"500ms"`, `"2s"`.

## Shell Completions

```bash
# Bash
hive completions bash > /etc/bash_completion.d/hive

# Zsh
hive completions zsh > ~/.zsh/completions/_hive

# Fish
hive completions fish > ~/.config/fish/completions/hive.fish
```

## Project Structure

```
your-project/
  hive.config.yaml           # optional project config
  hive.env.yaml              # default environment variables
  hive.env.staging.yaml      # staging overrides
  hive.env.local.yaml        # local secrets (gitignored)
  tests/
    health.hive.yaml
    users/
      crud.hive.yaml
      validation.hive.yaml
    auth/
      login.hive.yaml
```

## Development

```bash
# Clone
git clone https://github.com/NazarKalytiuk/hive.git
cd hive

# Build
cargo build

# Run tests (280 unit + 13 integration)
cargo test --all

# Run the demo server (for manual testing)
PORT=3333 cargo run -p demo-server

# Run example tests against demo server
cargo run -p hive -- run examples/ --var base_url=http://localhost:3333

# HTML report
cargo run -p hive -- run examples/ --var base_url=http://localhost:3333 --format html

# Benchmark
cargo run -p hive -- bench examples/minimal.hive.yaml -n 200 -c 20 --var base_url=http://localhost:3333

# Lint
cargo clippy -- -D warnings
cargo fmt --check
```

### Architecture

Pipeline: **parse YAML &rarr; resolve env &rarr; interpolate templates &rarr; execute HTTP &rarr; assert responses &rarr; report results**

| Module | Role |
|--------|------|
| `model.rs` | Serde structs for `.hive.yaml` format |
| `parser.rs` | YAML file loading and validation |
| `env.rs` | Environment variable resolution (6-layer priority) |
| `interpolation.rs` | `{{ }}` template engine |
| `runner.rs` | Orchestrator: setup &rarr; tests &rarr; teardown |
| `http.rs` | HTTP client (reqwest, blocking) |
| `capture.rs` | JSONPath value extraction |
| `assert/` | Status, body, headers, duration assertions |
| `report/` | Human, JSON, JUnit, TAP, HTML reporters |
| `builtin.rs` | `$uuid`, `$random_hex`, `$timestamp`, etc. |
| `bench.rs` | Performance testing with async concurrency |
| `config.rs` | `hive.config.yaml` parsing |

## License

MIT
