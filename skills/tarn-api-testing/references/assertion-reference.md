# Tarn Assertion Reference

Complete list of all assertion operators available in `.tarn.yaml` files.

## Status Assertions

```yaml
assert:
  status: 200                    # exact match
  status: "2xx"                  # range shorthand (1xx–5xx)
  status: { in: [200, 201] }    # set membership
  status: { gte: 200, lt: 300 } # numeric range
```

## Duration Assertions

```yaml
assert:
  duration: "< 500ms"           # less than 500ms
  duration: "<= 1s"             # at most 1 second
  duration: "> 100ms"           # more than 100ms
  duration: ">= 200ms"         # at least 200ms
```

## Header Assertions

Header names are case-insensitive.

```yaml
assert:
  headers:
    content-type: "application/json"             # exact match
    x-request-id: 'matches "^[a-f0-9-]{36}$"'   # regex
    x-custom: 'contains "value"'                 # substring
```

## Redirect Assertions

Requires `follow_redirects: false` at step or defaults level to inspect the redirect chain.

```yaml
assert:
  redirect:
    url: "https://api.example.com/final"   # final URL after redirects
    count: 2                                # number of redirects followed
```

## Body Assertions

Body assertions use JSONPath keys. The special key `"$"` refers to the entire response body.

### Exact Match (Literal)

```yaml
body:
  "$.name": "Jane Doe"          # string
  "$.age": 25                   # number
  "$.active": true              # boolean
  "$.deletedAt": null           # null
```

### Operator Objects

Multiple operators on the same JSONPath combine with AND logic.

#### Equality

| Operator | Example | Description |
|----------|---------|-------------|
| `eq` | `{ eq: "Alice" }` | Explicit equality (same as literal) |
| `not_eq` | `{ not_eq: "Bob" }` | Not equal to value |

#### Type Checking

| Operator | Example | Description |
|----------|---------|-------------|
| `type` | `{ type: string }` | JSON type: `string`, `number`, `boolean`, `array`, `object`, `null` |

#### String Operators

| Operator | Example | Description |
|----------|---------|-------------|
| `contains` | `{ contains: "sub" }` | Substring match |
| `not_contains` | `{ not_contains: "err" }` | No substring match |
| `starts_with` | `{ starts_with: "usr_" }` | String prefix |
| `ends_with` | `{ ends_with: ".com" }` | String suffix |
| `matches` | `{ matches: "^[a-z]+$" }` | Regex match |

#### Emptiness and Existence

| Operator | Example | Description |
|----------|---------|-------------|
| `not_empty` | `{ not_empty: true }` | Non-empty string, array, or object |
| `empty` | `{ empty: true }` | Empty string, array, object, or null |
| `is_empty` | `{ is_empty: true }` | Alias for `empty` |
| `exists` | `{ exists: true }` | Field exists in response |
| `exists` | `{ exists: false }` | Field does not exist |

#### Numeric Comparisons

| Operator | Example | Description |
|----------|---------|-------------|
| `gt` | `{ gt: 0 }` | Greater than |
| `gte` | `{ gte: 1 }` | Greater than or equal |
| `lt` | `{ lt: 100 }` | Less than |
| `lte` | `{ lte: 99 }` | Less than or equal |

#### Length Operators

| Operator | Example | Description |
|----------|---------|-------------|
| `length` | `{ length: 5 }` | Exact length (string or array) |
| `length_gt` | `{ length_gt: 0 }` | Length greater than |
| `length_gte` | `{ length_gte: 1 }` | Length greater than or equal |
| `length_lte` | `{ length_lte: 100 }` | Length less than or equal |

#### Array Operators

| Operator | Example | Description |
|----------|---------|-------------|
| `contains` | `{ contains: "admin" }` | Array contains element |
| `not_contains` | `{ not_contains: "guest" }` | Array does not contain element |
| `length` | `{ length: 3 }` | Exact array length |

#### Format Validators

| Operator | Example | Description |
|----------|---------|-------------|
| `is_uuid` | `{ is_uuid: true }` | Valid UUID |
| `is_date` | `{ is_date: true }` | Valid date or datetime string |
| `is_ipv4` | `{ is_ipv4: true }` | Valid IPv4 address |
| `is_ipv6` | `{ is_ipv6: true }` | Valid IPv6 address |

#### Hash and Size Operators

| Operator | Example | Description |
|----------|---------|-------------|
| `bytes` | `{ bytes: 1024 }` | Byte length of value (for `$`, uses raw body) |
| `sha256` | `{ sha256: "2cf24d..." }` | SHA-256 hex digest |
| `md5` | `{ md5: "5d4140..." }` | MD5 hex digest |

### Combined Example

```yaml
body:
  "$.id": { type: string, is_uuid: true }
  "$.email": { type: string, contains: "@", matches: "^[^@]+@[^@]+\\.[^@]+$" }
  "$.age": { type: number, gte: 0, lt: 200 }
  "$.roles": { type: array, not_empty: true, contains: "user" }
  "$.metadata": { type: object, not_empty: true }
  "$.deletedAt": null
  "$": { bytes: 15 }
```
