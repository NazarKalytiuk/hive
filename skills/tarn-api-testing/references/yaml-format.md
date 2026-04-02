# Tarn Test File Format Reference

Complete reference for the `.tarn.yaml` test file structure.

## Top-Level Properties

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `name` | string | Yes | Human-readable name for this test file |
| `description` | string | No | What this test file covers |
| `version` | string | No | Schema version (always `"1"`) |
| `tags` | string[] | No | Tags for filtering with `--tag` |
| `env` | object | No | Inline env vars (lowest priority) |
| `cookies` | `"auto"` or `"off"` | No | Cookie handling mode (default: `"auto"`) |
| `redaction` | object | No | Header/value redaction policy for reports |
| `defaults` | object | No | Default settings for all requests |
| `setup` | step[] | No | Steps run once before all tests |
| `teardown` | step[] | No | Steps run after all tests (even on failure) |
| `tests` | object | One required | Named test groups (grouped format) |
| `steps` | step[] | One required | Flat step list (simple format) |

**Either `steps` or `tests` is required, but not both.**

## Two Formats

### Simple (flat steps)

```yaml
name: Health checks
steps:
  - name: GET /health
    request:
      method: GET
      url: "{{ env.base_url }}/health"
    assert:
      status: 200
```

### Grouped (named tests)

```yaml
name: User API
tests:
  create-user:
    description: "Creates a new user"
    tags: [smoke]
    steps:
      - name: POST /users
        request:
          method: POST
          url: "{{ env.base_url }}/users"
          body:
            name: "Jane"
        assert:
          status: 201
```

## Defaults Block

Applied to every request in the file. Step-level values override defaults.

```yaml
defaults:
  headers:
    Content-Type: "application/json"
    Accept: "application/json"
  auth:
    bearer: "{{ capture.token }}"
  timeout: 5000               # ms
  connect_timeout: 3000       # ms
  follow_redirects: true
  max_redirs: 10
  retries: 0
  delay: "0ms"                # e.g., "100ms", "2s"
```

## Step Properties

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `name` | string | Yes | Human-readable step name |
| `request` | object | Yes | HTTP request definition |
| `capture` | object | No | Extract values from response |
| `assert` | object | No | Assertions on response |
| `retries` | integer | No | Retry count on failure |
| `timeout` | integer | No | Step timeout in ms |
| `connect_timeout` | integer | No | Connect timeout in ms |
| `follow_redirects` | boolean | No | Follow HTTP redirects |
| `max_redirs` | integer | No | Max redirects to follow |
| `delay` | string | No | Delay before step (`"100ms"`, `"2s"`) |
| `poll` | object | No | Polling configuration |
| `script` | string | No | Lua script for custom validation |
| `cookies` | bool/string | No | Cookie jar control |

## Request Properties

| Property | Type | Required | Description |
|----------|------|----------|-------------|
| `method` | string | Yes | HTTP method (GET, POST, PUT, DELETE, PATCH, etc.) |
| `url` | string | Yes | Request URL (supports interpolation) |
| `headers` | object | No | Request headers |
| `auth` | object | No | Auth helper (bearer or basic) |
| `body` | any | No | JSON request body |
| `form` | object | No | URL-encoded form body |
| `graphql` | object | No | GraphQL query/mutation |
| `multipart` | object | No | Multipart form data |

**Only one of `body`, `form`, `graphql`, `multipart` should be used per request.**

## Auth Config

```yaml
# Bearer token
auth:
  bearer: "{{ capture.token }}"

# Basic auth
auth:
  basic:
    username: "{{ env.api_user }}"
    password: "{{ env.api_pass }}"
```

## Capture Formats

### JSONPath shorthand

```yaml
capture:
  user_id: "$.id"
  token: "$.auth.token"
```

### Extended capture

```yaml
capture:
  session:                       # from header
    header: "set-cookie"
    regex: "session=([^;]+)"     # optional regex
  csrf:                          # from cookie
    cookie: "csrf_token"
  final_url:                     # final URL after redirects
    url: true
  status_code:                   # HTTP status code
    status: true
  raw_body:                      # whole response body
    body: true
  explicit_jsonpath:             # explicit JSONPath form
    jsonpath: "$.data.id"
```

## Include Directive

Reuse steps from another file:

```yaml
setup:
  - include: ./shared/auth-setup.tarn.yaml
    with:                          # inject parameters
      role: admin
    override:                      # deep-merge into each imported step
      timeout: 10000
```

Included file receives `with` values as `{{ params.name }}`.

## Polling Config

```yaml
poll:
  until:                           # assertions that must pass
    body:
      "$.status": "completed"
  interval: "2s"                   # time between attempts
  max_attempts: 10                 # max tries
```

## Redaction Config

```yaml
redaction:
  headers:                         # header names to redact (case-insensitive)
    - authorization
    - cookie
    - set-cookie
    - x-api-key
  replacement: "***"               # replacement string
  env:                             # env var values to redact
    - api_key
    - secret
  captures:                        # capture values to redact
    - token
```

## Multipart Config

```yaml
multipart:
  fields:
    - name: "title"
      value: "My Document"
  files:
    - name: "file"
      path: "./fixtures/test.pdf"
      content_type: "application/pdf"
      filename: "renamed.pdf"      # optional override
```

## GraphQL Config

```yaml
graphql:
  query: |
    query GetUser($id: ID!) {
      user(id: $id) { name email }
    }
  variables:
    id: "{{ capture.user_id }}"
  operation_name: "GetUser"        # optional, for multi-operation queries
```

## Interpolation

All string values support template interpolation:

- `{{ env.name }}` — environment variable
- `{{ capture.name }}` — captured value from previous step
- `{{ params.name }}` — parameter from include `with:` block
- `{{ $uuid }}` — UUID v4
- `{{ $timestamp }}` — Unix epoch seconds
- `{{ $now_iso }}` — ISO 8601 datetime
- `{{ $random_hex(N) }}` — random hex string
- `{{ $random_int(min, max) }}` — random integer

## Schema Validation

Add to the top of test files for IDE autocompletion:

```yaml
# yaml-language-server: $schema=https://raw.githubusercontent.com/NazarKalytiuk/hive/main/schemas/v1/testfile.json
```
