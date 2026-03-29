# Tarn CLI Retrospective v3 — Final Assessment

## Context

Third round of testing tarn after two feedback cycles. All 9 original issues from v1 are now resolved. This retrospective evaluates the final state after using includes, `cookies: false`, cookie jar, header capture, status ranges, multipart, and type-aware captures in a real 29-file test suite.

---

## All Original Issues: Resolved

| # | Issue (v1) | v2 Status | v3 Status |
|---|-----------|-----------|-----------|
| 1 | No header capture | Fixed | **Works perfectly** |
| 2 | No cookie jar | Fixed | **Works perfectly** |
| 3 | No shared setup / includes | Broken (didn't execute) | **Fixed — works in setup, teardown, steps, test groups** |
| 4 | Capture failure aborts run | Fixed | **Works perfectly** |
| 5 | No multipart upload | Fixed | **Works perfectly** (relative paths resolve correctly) |
| 6 | Status only exact numbers | Fixed | **Works perfectly** (`"2xx"`, `{ in: [...] }`, `{ gte: N }`) |
| 7 | Captured values always strings | Fixed | **Works perfectly** |
| 8 | Body assertion docs mismatch | Fixed | **Resolved** |
| 9 | No default delay | Fixed | **Available** |

**9/9 fully resolved.** The includes fix in v3 was the last missing piece.

---

## Feature-by-Feature Assessment

### Includes (10/10 — the v3 hero)

Everything we wanted:
- Works in `setup:`, `teardown:`, `steps:`, and `tests:` group steps
- Relative paths resolve from the test file's directory (not CWD)
- Captures and cookie jar state flow from included steps to the parent file
- Nested includes work (include a file that itself includes another)
- Circular include detection with clear error message
- Bad path gives immediate, actionable error: `Include file not found: ./x.tarn.yaml (resolved to /full/path)`

**Impact:** 23 of 29 files now use includes. Setup duplication dropped from 150 steps to 0. The two shared files (auth-single: 71 lines, auth-two-users: 156 lines) replace ~3,500 lines of duplicated setup.

Only 6 files don't use includes (4 auth tests with custom setup, 2 no-auth tests).

### Cookie Jar (9/10)

Automatic Set-Cookie capture and sending is seamless. The `cookies: false` step-level override is the clean escape hatch we asked for — no more `Cookie: ""` hacks.

One minor friction: in two-user setups, the jar holds the last user's cookies. User-switching requires explicit `Cookie:` headers with captured tokens. This is inherent to the single-jar design and documented in our shared include file.

### `cookies: false` (10/10)

Exactly what we needed. Clean, declarative, step-level:
```yaml
- name: Without auth returns 401
  cookies: false
  request:
    method: GET
    url: "{{ env.base_url }}/api/endpoint"
  assert:
    status: 401
```

No side effects on the jar — the next step still sends cookies normally. Boolean type validation catches typos (`cookies: "invalid"` → clear parse error).

### Header Capture (9/10)

Clean syntax, case-insensitive header lookup, regex extraction:
```yaml
capture:
  session_token:
    header: "set-cookie"
    regex: "better-auth\\.session_token=([^;]+)"
```

Only used in two-user setups now (single-user relies on cookie jar). Would be nice to have a shorthand for common patterns like `capture: { token: { cookie: "session_token" } }` but the current syntax is fine.

### Status Ranges (9/10)

All three forms work:
- `status: "2xx"` / `status: "4xx"` — shorthand ranges
- `status: { in: [400, 401, 422] }` — explicit set
- `status: { gte: 400 }` — numeric range

Used `"4xx"` extensively in auth error tests. Makes tests resilient to minor status code changes (400 vs 422 for validation errors).

### Multipart Upload (8/10)

File uploads work end-to-end:
```yaml
request:
  method: POST
  url: "{{ env.base_url }}/api/photos"
  multipart:
    fields:
      - name: "purpose"
        value: "profile"
    files:
      - name: "file"
        path: "../../../e2e/fixtures/test-photo.jpg"
        content_type: "image/jpeg"
```

Relative paths resolve from the test file's directory. Confirmed with our test photo fixture.

Minor note: can't generate file content dynamically (e.g., random image bytes for fuzz testing). Only real files on disk.

### Graceful Capture Failure (10/10)

Step is marked failed, next step runs. Exit code 1 (not 3). This is exactly right for CI.

### Type-Aware Captures (9/10)

Numbers, booleans, and nulls preserve their types in the `captures` Lua table. No more `tonumber()` workarounds. Confirmed with `type(captures["count"]) == "number"`.

### Lua Scripting (8/10 — same as before, less needed)

Still essential for Mailpit email token extraction (15 scripts remain). Everything else that previously needed Lua is now handled by native features. The Lua scripts could be further reduced if tarn added a regex body capture (extract from HTML/text responses without JSONPath).

### Error Messages (9/10)

Significant improvement over v1:
- Include not found: shows resolved path
- Parse errors: line/column numbers with context
- Invalid `cookies:` value: clear type error
- Capture failure: shows available keys in response

Only gap: when a setup step fails silently (e.g., Mailpit returns no messages), downstream captures fail with confusing "JSONPath matched no values" instead of pointing to the root cause.

---

## Quantitative Evolution

| Metric | v1 | v2 | v3 |
|--------|-----|-----|-----|
| YAML (test files) | 26,000 | 8,600 | **6,124** |
| YAML (shared) | 0 | 0 | **227** |
| YAML (total) | 26,000 | 8,600 | **6,351** |
| Lua scripts | 47 | ~15 | **~15** |
| Cookie hacks | 120 headers | 30 `Cookie: ""` | **0** |
| Setup duplication | 150 steps | 150 steps | **0** |
| Test steps | 614 | 602 | **597** |
| Pass rate | 100% | 100% | **100%** |
| Reduction from v1 | — | -67% | **-76%** |

### Lines per Domain (v3)

| Domain | Files | Lines | Steps |
|--------|-------|-------|-------|
| auth | 4 | 1,224 | 90 |
| settings | 9 | 1,172 | 137 |
| photos | 4 | 962 | 102 |
| profile | 2 | 760 | 51 |
| discover | 3 | 719 | 65 |
| social | 3 | 539 | 61 |
| chat | 2 | 471 | 57 |
| admin | 2 | 277 | 34 |
| **Total** | **29** | **6,124** | **597** |

---

## Remaining Friction (Minor)

### 1. Mailpit Lua Scripts (~15 remaining)
The only boilerplate left. Each file with email verification needs 2 Lua scripts to extract the verification token from Mailpit's API response. A native regex body capture would eliminate these:
```yaml
# Wanted
capture:
  verify_token:
    jsonpath: "$.Text"
    regex: "token=([\\w\\-\\.]+)"
```

### 2. Two-User Cookie Switching
With the single cookie jar, two-user tests need explicit Cookie headers for the first user. This is a fundamental design choice (not a bug), but it means two-user tests are slightly more verbose than single-user ones.

### 3. No `tarn run --flush-between` for Rate-Limited APIs
We still flush Redis externally between test files. A built-in delay or hook between files would help, though this is an edge case specific to aggressive rate limiting in dev.

### 4. Include Files Show as Validation Failures
`tarn validate` reports shared include files as parse errors (they're step arrays wrapped in a minimal TestFile, not standalone tests). Would be nice to either skip `shared/` or have a `.tarnignore`.

---

## What We'd Change If Starting Over

Not much. The workflow landed in a good place:

1. **Shared includes for auth** — correct from the start
2. **Cookie jar on by default** — right default, `cookies: false` for exceptions
3. **Status ranges for error tests** — more resilient than exact codes
4. **Multipart for uploads** — should have added upload tests earlier
5. **Header capture for two-user tokens** — cleaner than Lua extraction

The one thing we'd do differently: start with the shared include files *before* writing any test files, not after.

---

## DX Rating

| Aspect | v1 | v2 | v3 |
|--------|-----|-----|-----|
| YAML format | 9 | 9 | **9** |
| CLI experience | 8 | 8 | **9** (better errors) |
| Auth/cookie handling | 3 | 8 | **10** (includes + jar + cookies:false) |
| Error handling | 4 | 8 | **9** (graceful + ranges) |
| File uploads | 0 | 7 | **8** |
| Code reuse | 0 | 2 | **10** (includes work!) |
| Lua scripting | 8 | 8 | **8** (less needed) |
| LLM-friendliness | 9 | 10 | **10** |
| Error messages | 5 | 7 | **9** |

**Overall: 7/10 → 8/10 → 9/10**

---

## Verdict

Tarn is now a production-ready API testing tool. The three-round feedback loop took it from "solid foundation with painful gaps" (7/10) to "would recommend for real projects" (9/10).

The killer combination is:
- **Includes** for DRY setup
- **Cookie jar** for zero-config auth
- **`cookies: false`** for clean unauth tests
- **YAML brevity** for LLM-friendly generation

For the SWD2 project specifically: 597 test steps in 6,351 lines of YAML, covering every API endpoint, finding 3 real bugs. The tests run in ~15 seconds per file (including user creation and email verification). That's a good return.

The only 10/10 blocker remaining is eliminating the Mailpit Lua scripts via native regex body capture. Everything else is there.
