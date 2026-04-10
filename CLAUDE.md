# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Tarn is a CLI-first API testing tool written in Rust. Tests are defined in `.tarn.yaml` files. Designed for AI-assisted workflows: an LLM writes tests, runs `tarn run`, parses structured JSON output, and iterates.

## Build & Run Commands

```bash
cargo build                    # dev build
cargo build --release          # release build (single binary, zero runtime deps)
cargo run -- run               # run all tests
cargo run -- run tests/x.tarn.yaml  # run specific test file
cargo run -- validate          # validate YAML without running
cargo run -- list              # dry run listing
cargo test                     # run all Rust tests
cargo test parser_test         # run a single test module
cargo clippy                   # lint
cargo fmt                      # format
```

## Mandatory Pre-Commit Checks

Always run these commands before every commit and push. Do not skip them — CI will fail otherwise. This has caused repeated CI failures across multiple sessions.

```bash
cargo fmt                      # fix formatting FIRST
cargo clippy -- -D warnings    # then fix all warnings
cargo test                     # then verify tests pass
```

Do not commit or push if any of these fail. Fix the issue, then commit.

## Rules Learned From Past Mistakes

### Quality gates
- Never suppress linter/clippy warnings with `#[allow(...)]`. Always fix the root cause — refactor functions, extract structs, simplify signatures. Suppression is not a fix. (User explicitly rejected this approach.)
- Never dismiss a failing test as "flaky" or "pre-existing" without investigating. Read the test, understand why it fails, fix it. Every test failure is a real signal until proven otherwise.
- Do not commit/push with known failing tests. If tests fail, fix them first — even if they look unrelated to your changes.

### Verification
- Always verify artifacts from the real production URL/endpoint, never from local files. For install scripts: `curl <url> | sh`, not `sh ./local/path/install.sh`. For release assets: download from GitHub releases, not from the local build. (User was frustrated twice by this.)
- When writing or updating docs (README, install instructions), verify every command and URL actually works end-to-end before committing. If a doc references a binary or asset, confirm it exists in the release pipeline.

### Bulk operations (renames, URL changes)
- After any bulk rename or URL change, run a project-wide grep to verify zero remaining references to the old name. Check: source code (struct/enum names, string literals), documentation (*.md), CI/CD configs (*.yml), scripts (*.sh), Cargo.toml, and schemas. One pass is never enough — always verify with grep.

### Releases and versioning
- When shipping a new binary, changing distribution channels, or making breaking changes: always bump the version in Cargo.toml (and any other version references). Do not wait to be asked.
- Think from the user's perspective when shipping: "How does a user get this fix?" Create a proper release — do not replace local binaries or suggest manual workarounds.

### Documentation accuracy
- Never reference URLs, domains, or external resources without verifying they exist. Do not fabricate domains (e.g., `tarn.dev` was invented and does not exist).
- When documenting features, verify the feature is actually shipped in the release pipeline. If `tarn-mcp` is not in the release workflow, do not document it as available to users.

## Architecture

The codebase follows a pipeline architecture: **parse YAML -> resolve env/variables -> execute HTTP -> assert responses -> report results**.

Key modules in `src/`:

- **model.rs** - Serde-derived Rust structs mirroring the YAML test format (TestFile, Step, Assertion, etc.)
- **parser.rs** - Loads `.tarn.yaml` files into `TestFile` structs
- **env.rs** - Environment variable resolution with priority chain: CLI `--var` > shell env > `tarn.env.local.yaml` > `tarn.env.{name}.yaml` > `tarn.env.yaml` > inline `env:` block
- **interpolation.rs** - `{{ env.x }}` and `{{ capture.x }}` template resolution across all string fields
- **runner.rs** - Orchestrator: load file -> resolve env -> run setup -> run tests -> run teardown
- **http.rs** - Request execution via reqwest (blocking initially)
- **capture.rs** - JSONPath + header extraction from responses for variable chaining between steps (type-preserving)
- **cookie.rs** - Automatic cookie jar: captures Set-Cookie, sends Cookie on subsequent requests
- **assert/** - Assertion modules: status, headers, body (JSONPath), duration, types
- **report/** - Output formatters: human, json, junit, tap, html, curl
- **builtin.rs** - Built-in functions: `$uuid`, `$random_hex(n)`, `$random_int(min,max)`, `$timestamp`, `$now_iso`
- **config.rs** - Optional `tarn.config.yaml` parsing
- **main.rs** - CLI entry point using clap (derive)

## Key Crates

| Purpose | Crate |
|---------|-------|
| CLI | `clap` (derive) |
| YAML | `serde` + `serde_yaml` |
| HTTP | `reqwest` (blocking) |
| JSONPath | `serde_json_path` |
| JSON Schema | `jsonschema` |
| Regex | `regex` |
| Colored output | `colored` |
| Diff | `similar` |
| Templates | `handlebars` or manual `{{ }}` |

## Test File Format

Files use `.tarn.yaml` extension. Minimal test:
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

Full format supports: `env`, `defaults`, `setup`, `teardown`, `tests` (with `steps`), `capture`, `cookies`, `form`, `multipart`, `include`, `auth`, polling, and rich assertions. Use `README.md`, `docs/INDEX.md`, and `schemas/v1/testfile.json` as the canonical surface reference.

## Exit Codes

- 0: all tests passed
- 1: one or more tests failed
- 2: configuration/parse error
- 3: runtime error (network failure, timeout)

## Project Status

The competitiveness roadmap is complete through `T50`. Use `docs/TARN_PRODUCT_STRATEGY.md` for current direction and `docs/TARN_COMPETITIVENESS_ROADMAP.md` only as historical sequencing context.

## Design Decisions

- JSON output includes full request/response ONLY for failed steps (keeps output compact)
- Runtime failures are still emitted as structured failed steps in JSON; connection failures usually have `request` but no `response`
- Secrets in headers are redacted to `***` in output
- Assertions on the same JSONPath use AND logic (all must pass)
- Tests within a file run sequentially; steps within a test are sequential
- Each test is independent but steps within a test share captured variables
- Setup runs once before all tests; teardown runs even if tests fail
- Capture failures are graceful — step is marked failed, run continues (no exit code 3 abort)
- Captures preserve JSON types (numbers, booleans) — not coerced to strings
- Automatic cookie jar is on by default; disable with `cookies: "off"` per file
- Status assertions support exact (`200`), shorthand (`"2xx"`), sets (`in: [200, 201]`), ranges (`gte: 400, lt: 500`)
- Supports multipart/form-data via `multipart:` field (separate from `body:`)
- Shared setup via `include:` directives in step arrays, resolved at parse time
- Project config also controls defaults, redaction policy, environments, and transport settings

## AI Workflow

Preferred diagnosis loop:

1. `cargo run -- validate <file>` for syntax/config issues
2. `cargo run -- run <file> --format json --json-mode compact`
3. inspect `failure_category` and `error_code`
4. inspect `assertions.failures`
5. inspect optional `request` / `response`
6. optionally use `tarn_fix_plan`
7. patch YAML or application code

Useful docs:

- `docs/AI_WORKFLOW_DEMO.md`
- `docs/MCP_WORKFLOW.md`
- `schemas/v1/report.json`


# Testing Strategy

You are acting as a senior QA engineer AND developer on this project. I am a solo developer — I write code, I test code, I ship code. There is no separate QA team. Tests are my only safety net.

## Core Philosophy
- Every piece of code must be tested before it's considered done
- Tests must catch real bugs, not just satisfy coverage metrics
- A test that can't fail is worthless — every assertion must be meaningful
- Test behavior, not implementation details

## What to Test (Priority Order)

### 1. Critical Path (MUST have)
- All public API endpoints: valid input, invalid input, auth/unauth, edge cases
- All service methods: happy path + every error branch
- All database operations: create, read, update, delete + constraint violations
- All business logic: calculations, state transitions, validations

### 2. Edge Cases (MUST have)
- Empty inputs, null, undefined
- Boundary values (0, -1, MAX_INT, empty string, very long string)
- Concurrent operations where applicable
- Malformed data, unexpected types

### 3. Error Handling (MUST have)
- Every catch block must be triggered by a test
- External service failures (DB down, API timeout, network error)
- Validation errors — test every validation rule
- Auth failures: expired token, wrong role, missing token

### 4. Integration Points (SHOULD have)
- Service-to-service communication
- Database queries with realistic data
- Message queue producers/consumers

## How to Write Tests

### Structure
- Use AAA pattern: Arrange → Act → Assert
- One logical assertion per test (multiple expect() is OK if testing one behavior)
- Test name must describe the scenario: `should return 404 when user does not exist`
- Group tests logically with describe blocks by method/feature

### Mocking Rules
- Mock external dependencies (DB, HTTP, message queues), NOT the unit under test
- Never mock what you're testing
- Use realistic mock data, not `{ foo: 'bar' }`
- Verify mock interactions (was the DB called with correct params?)

### Quality Checks
- Every test must fail if the corresponding code is removed/broken (mutation-resistant)
- No test should depend on another test's state (isolated)
- No hardcoded dates/times — use relative or frozen time
- No flaky patterns: no `setTimeout`, no reliance on execution order

## Coverage Requirements
- Aim for >90% line coverage on business logic / services / controllers
- 100% coverage on validators, guards, interceptors, pipes
- Every public method must have at least: 1 happy path + 1 error path test
- Every `if` branch must be covered
- Every `catch` block must be covered

## When Writing New Code
After implementing any feature or fixing a bug:
1. Write tests for the happy path first
2. Write tests for every error/edge case
3. Run all tests to make sure nothing is broken
4. If coverage for the changed file is <90%, add more tests

## When Writing Tests for Existing Code
When I ask you to "cover X with tests" or "add tests for this module":
1. First, READ the entire file and understand all branches/paths
2. List all scenarios that need testing (show me the plan)
3. Write ALL the tests — do not skip scenarios, do not say "similar tests can be added"
4. Run the tests. Fix any failures.
5. Report final coverage for the file.

## Anti-Patterns to AVOID
- ❌ `expect(result).toBeDefined()` alone — too weak, assert the actual value
- ❌ Testing private methods directly — test through public API
- ❌ Copy-paste tests with minor variations — use `test.each` / parameterized tests
- ❌ Snapshot tests for logic (OK for UI components only)
- ❌ Testing framework/library code — only test YOUR code
- ❌ Writing `// TODO: add more tests` — write them NOW or never
