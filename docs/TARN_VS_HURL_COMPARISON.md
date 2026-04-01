# Tarn vs Hurl

**Date**: 2026-04-01  
**Scope**: current-state comparison after Tarn completed the competitiveness roadmap in [`docs/TARN_COMPETITIVENESS_ROADMAP.md`](./TARN_COMPETITIVENESS_ROADMAP.md).

This document replaces the older pre-release deep dive. That version was useful while Tarn still had obvious transport and tooling gaps; it became misleading once proxy/TLS/certs, formatter, editor support, migration tooling, richer reports, and the conformance suite landed.

## Executive Summary

Tarn is now competitive with Hurl for the common JSON/API workflow that most teams actually run day to day:

- standard HTTP requests with proxy and TLS controls
- redirects, cookies, forms, multipart, GraphQL, polling, retries
- captures from JSONPath, headers, cookies, status, final URL, and body regex
- multi-format reporting, curl export, benchmark thresholds, and machine-readable diagnostics

Hurl still leads when the requirement is deeper libcurl-level protocol coverage or its richer query/filter surface:

- XPath / HTML assertions and captures
- certificate inspection queries
- full Hurl filter DSL
- exotic auth and libcurl-only transports

The practical decision is now much narrower than it was earlier:

- choose **Tarn** when you want YAML, lifecycle (`setup` / `teardown`), structured JSON for agents, MCP, watch mode, curl export, and reusable include-driven suites
- choose **Hurl** when you need maximum protocol depth and the exact Hurl surface, especially HTML/XPath or libcurl-specific networking knobs

## Where Tarn Now Reaches Practical Parity

### HTTP and Transport

Tarn now covers the baseline HTTP surface that previously blocked serious adoption:

- `proxy` / `no-proxy`
- `cacert`, `cert`, `key`, `insecure`
- separate `connect_timeout` and total `timeout`
- `follow_redirects`, `max_redirs`, redirect assertions, and final-URL capture
- explicit `http1.1` / `http2`
- custom HTTP methods

For the normal REST/JSON environment, this moved Tarn from "interesting but not deployable" to "usable without transport caveats".

### Request Authoring

Tarn now has first-class support for:

- JSON bodies
- `form:` URL-encoded payloads
- `multipart:` with fields and files
- GraphQL requests
- auth helpers for bearer and basic auth
- include parameterization via `with:` and `override:`
- named environments via project config plus `tarn env`

This matters because it removes a large amount of manual header/body boilerplate that earlier Tarn users had to hand-roll.

### Cookies and State

Tarn no longer uses a toy cookie implementation. It now has:

- spec-aware matching for domain, path, secure, expiry, `HttpOnly`, and `SameSite`
- named cookie jars
- cookie-jar import/export
- per-step cookie control

That closes most realistic API-suite cookie scenarios, especially multi-user flows that are awkward in flat tools.

### Assertions and Captures

Tarn now covers the high-frequency assertion and extraction paths teams expect:

- whole-body text / JSON equality with unified diff
- `is_uuid`, `is_date`, `is_ipv4`, `is_ipv6`, `empty`, `bytes`, `sha256`, `md5`
- captures from status, final URL, headers, cookies, and body regex
- transform-lite pipeline: `first`, `last`, `count`, `join`, `split`, `replace`, `to_int`, `to_string`

This is still not the full Hurl filter language, but it is enough for a large share of real API chaining without dropping into Lua.

### Reports, Diagnostics, and Tooling

This is where Tarn is now clearly differentiated rather than merely "good enough":

- stable JSON schema with `failure_category`, `error_code`, and remediation hints
- compact and verbose JSON modes
- multiple output targets in one run
- `curl` and `curl-all` exporters
- richer HTML report with diff views and copy-curl
- MCP server with `tarn_run`, `tarn_validate`, `tarn_list`, `tarn_fix_plan`
- VS Code extension, `tarn fmt`, `tarn init`, docs site, and public conformance suite

Hurl has mature outputs and excellent plain-text ergonomics. Tarn's advantage is that the machine loop is first-class instead of accidental.

## Where Hurl Still Leads

Tarn did not and should not try to erase every Hurl advantage.

### Intentionally Unclosed Gaps

- XPath / HTML assertions and captures
- certificate inspection queries
- full Hurl-style filter DSL
- digest / NTLM / Negotiate / AWS SigV4 parity
- raw libcurl-level protocol completeness

These are consistent with the `Not now` section in the roadmap rather than missing engineering follow-through.

### libcurl-Specific Networking

Hurl still has the stronger story when you need features tied to libcurl's transport surface, such as:

- HTTP/3
- unix sockets
- custom DNS resolve rules
- netrc
- transfer speed limiting

Tarn's reqwest/rustls stack is simpler, easier to distribute, and fully self-contained, but it is not a replacement for libcurl in those niches.

## Decision Guide

Choose **Tarn** when:

- the suite is authored or maintained by AI agents
- you want YAML plus JSON schema support
- lifecycle matters: `setup`, `teardown`, reusable includes, named jars
- structured diagnostics matter more than handwritten HTTP aesthetics
- benchmarking and API testing should live in the same binary

Choose **Hurl** when:

- your tests depend on XPath / HTML parsing
- you need full Hurl filter expressions
- you need exotic auth or libcurl-only transport knobs
- protocol completeness matters more than AI tooling and suite lifecycle

## Migration Reality

For many existing Hurl suites, the situation is now:

- **simple JSON/API files**: migrate cleanly
- **auth/setup heavy suites**: often become cleaner in Tarn because `setup`, `teardown`, named jars, and includes are first-class
- **HTML/XPath or libcurl-heavy suites**: keep those pieces in Hurl for now

Use [`docs/HURL_MIGRATION.md`](./HURL_MIGRATION.md) for the practical rewrite path and `tarn import-hurl` for the conservative converter path.

## Bottom Line

Tarn no longer needs to apologize for missing the obvious enterprise HTTP basics. That gap is closed.

What remains is mostly intentional:

- Hurl is still the stronger protocol-depth tool
- Tarn is the stronger AI-loop and lifecycle tool

That is a real product boundary, not a temporary state.
