# Troubleshooting

Guide to diagnosing and fixing the most common Tarn failure modes.

Default to the **failures-first loop** before anything else:

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

## Response-shape drift

### Symptom

One step fails (usually `assertion_failed` on `status`, or `capture_error`
on a JSONPath) and several downstream steps in the same test are
reported with `failure_category: skipped_due_to_failed_capture`. The
suite looks like it has many independent failures but is actually
one root cause fanning out.

### How to identify it from `failures.json`

`tarn failures --format json` groups failures by fingerprint and
collapses cascade skips into their root-cause entry. A drift incident
shows up as a single group with a non-empty `cascades` list (or a
`cascades: N` counter in the human format):

```bash
tarn failures
tarn failures --format json --run <run_id>      # specific archive
tarn failures --include-cascades                # expand cascades only if needed
```

If one root-cause group carries many cascade skips and its failing
step's `response` block shows a body shape different from what the
test expected, you are looking at response-shape drift — not a bug in
your downstream assertions.

### Worked example — the reopen-request incident

A create/reopen endpoint used to return:

```json
{ "uuid": "e4f2…", "status": "pending" }
```

The test captured `user_id: "$.uuid"` and every downstream step used
`{{ capture.user_id }}` to drive `GET /requests/{{ capture.user_id }}`
calls.

The endpoint now returns an envelope:

```json
{ "request": { "uuid": "e4f2…" }, "stageStatus": "pending" }
```

`$.uuid` no longer matches. The create step fails on its body
assertion (or on the capture itself). Every downstream step is marked
`skipped_due_to_failed_capture`.

### Recovery loop

1. `tarn failures --format json` — confirm there is one root-cause
   group with N cascade skips. Do NOT open the cascades individually.
2. `tarn inspect last FILE::TEST::STEP --format json` — open the
   failing step. Read `response.body_excerpt` (or the full `response`
   block if `--verbose-responses` was used). Identify the new shape.
3. Update the capture JSONPath to match the observed body. For the
   example: `"$.request.uuid"`. Add a type assertion as a guard rail
   so the next drift fails at the source step instead of cascading.

   Before:

   ```yaml
   capture:
     user_id: "$.uuid"
   assert:
     status: 201
   ```

   After:

   ```yaml
   capture:
     user_id: "$.request.uuid"
   assert:
     status: 201
     body:
       "$.request": { type: object }
       "$.request.uuid": { type: string, not_empty: true }
   ```

4. `tarn rerun --failed` — replay only the affected `(file, test)`
   pairs. Do NOT rerun the whole suite.
5. `tarn diff prev last --format json` — verify the root-cause
   fingerprint moved to `fixed` and `new: []` is empty. If any
   `persistent` entries remain, repeat the loop on those.

### Mutation vs read-response patterns

Drift is disproportionately common on mutation endpoints because they
often wrap the resource in an envelope while the corresponding read
endpoint returns the resource directly.

- **Mutation endpoints (`POST`/`PUT`/`PATCH`)** — frequently return
  `{"request": {...}, "meta": {...}}` or similar. Assert the
  envelope explicitly with a type check and capture from the wrapped
  path (`$.request.uuid`), not the read-shape path (`$.uuid`).
- **Read endpoints (`GET /resource/:id`)** — typically return the
  resource directly and drift less, but paginated/list endpoints wrap
  in `{"items": [...], "page": N}` and must be captured as
  `$.items[0].id`, never flattened to `$[0].id`.
- **When authoring a new capture**, run the real request once with
  `debug: true` on the step (or inspect the step's fixture under
  `.tarn/fixtures/.../latest-passed.json`) and copy the minimal
  JSONPath from the *observed* body. Never capture from memory.

## Route ordering (NestJS and similar)

### What's happening

Many web frameworks match HTTP routes in registration order. When a
dynamic (parameterized) route is registered *before* a specific,
sibling route, the dynamic route **swallows** calls that were meant
for the specific route.

Classic shape:

```ts
// NestJS controller — order matters.
@Get(':id')          // registered first: /foo/:id
findOne(...) { ... }

@Get('approve')      // registered second: /foo/approve
approve(...) { ... }
```

A request to `POST /foo/approve` is matched by `/foo/:id` with
`id = "approve"`. The handler then tries to parse `"approve"` as a
UUID (or integer, or whatever the parameter type is), fails
validation, and returns an opaque 4xx such as:

```json
{
  "statusCode": 400,
  "message": "Validation failed (uuid is expected)",
  "error": "Bad Request"
}
```

The caller sees a 400/404 and assumes their payload is wrong. The
real problem is server-side route registration order.

Express, Fastify, FastAPI, ASP.NET, and many other frameworks have
the same trap under different names.

### How Tarn flags it

When a test expects a 2xx status but receives a 4xx, and the response
body contains a strong textual signal of parameter-validation failure
(for example `"invalid uuid"`, `"cannot parse"`, `"validation failed"`,
`"route not found"`, or a framework-style error that names a URL
segment), Tarn prints a diagnostic note under the failure:

```
 ✗ POST /orders/approve (12ms)
   ├─ Expected HTTP status 201, got 400
   └─ note: the server may have matched this path to a dynamic
      route (e.g. /foo/:id); check for route ordering conflicts
      (see docs/TROUBLESHOOTING.md#route-ordering).
```

In JSON output the same hint appears on the failing `status`
assertion:

```json
{
  "assertion": "status",
  "expected": "201",
  "actual": "400",
  "message": "Expected HTTP status 201, got 400",
  "hints": [
    "note: the server may have matched this path to a dynamic route (e.g. /foo/:id); check for route ordering conflicts (see docs/TROUBLESHOOTING.md#route-ordering)."
  ]
}
```

The hint is intentionally conservative — it only fires on a clear
textual signal. Absence of the hint is **not** evidence that route
ordering is fine; it means the body didn't give a reliable clue.

### How to confirm

1. **Dump the server's route table** in registration order. In NestJS
   this is most easily done by temporarily logging in `main.ts`:

   ```ts
   const server = app.getHttpServer();
   const router = server._events.request._router;
   console.log(router.stack
     .filter(l => l.route)
     .map(l => `${Object.keys(l.route.methods)[0].toUpperCase()} ${l.route.path}`));
   ```

   For Express/Fastify/FastAPI, use the framework's equivalent
   introspection.

2. **Look for a specific route registered after a sibling dynamic
   route** on the same path prefix. Any `/foo/:something` that
   appears before `/foo/<literal>` is a trap.

3. **Try the call directly** with the literal path and a syntactically
   valid param value. If the handler accepts the UUID-shaped value
   but rejects the literal, you've confirmed the collision.

### How to fix

Reorder the route registrations so that **specific routes come
before dynamic routes**:

```ts
// Specific first.
@Get('approve')
approve(...) { ... }

// Dynamic last.
@Get(':id')
findOne(...) { ... }
```

In frameworks where controllers are composed from multiple modules,
check the module import order as well — a later-imported module's
dynamic route can still shadow an earlier-imported specific one if
the path prefixes line up.

When specific-before-dynamic isn't enough (for example, `approve`
really is a legal `:id` value in your domain), disambiguate with a
verb prefix on one side:

```ts
@Post(':id/approve')   // /foo/<uuid>/approve — unambiguous
```

## Common failure categories

| `failure_category` | Typical root cause |
|--------------------|--------------------|
| `connection_error` | Server is down, wrong host/port, DNS issue, TLS/connect failure |
| `timeout` | Step timed out before receiving a complete response |
| `assertion_failed` | Request succeeded, but a status/header/body/duration check failed |
| `capture_error` | The step passed assertions, but extraction failed afterward |
| `parse_error` | Invalid YAML, invalid JSONPath, or invalid config surface |

## Agent diagnosis loop

The canonical order is the failures-first loop at the top of this
document. Inside step 4 (opening a single failing step via
`tarn inspect last FILE::TEST::STEP --format json`), apply these
sub-rules:

1. `tarn validate` first — catches syntax and config surface errors.
2. `tarn run` writes `report.json`, `summary.json`, `failures.json`
   under `.tarn/runs/<run_id>/`. You usually only need the latter two.
3. Read `failure_category` and `error_code` before the free-text
   message.
4. If `failure_category` is `skipped_due_to_failed_capture`, STOP —
   fix the upstream root cause shown by `tarn failures` first;
   cascade skips clear automatically once the root cause passes.
5. If a failed `status` assertion carries `hints`, follow the first
   hint before second-guessing the test.
6. If `response` exists, inspect it before editing assertions or
   payloads. Response-shape drift (see the section above) is the
   most common non-business-logic cause of an `assertion_failed` +
   multiple cascade skips pattern.
7. If `request.url` still contains `{{ ... }}`, fix env/capture
   interpolation before retrying.
8. `tarn rerun --failed` and `tarn diff prev last` close the loop.

## Non-JSON bodies

- Tarn preserves plain text / HTML responses as JSON strings in the
  structured report.
- Use `body: { "$": "plain text response" }` to assert the whole root
  string when needed.
