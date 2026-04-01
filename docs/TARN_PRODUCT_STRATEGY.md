# Tarn Product Strategy

**Date**: 2026-04-01

This is the canonical strategy document for Tarn after the competitiveness roadmap was completed.

## Product Thesis

Tarn should keep positioning itself as:

`API testing that AI agents can write, run, and debug`

That remains a stronger and more defensible claim than:

- "better Hurl"
- "another Postman replacement"
- "YAML API testing for everyone"

## The Wedge

Tarn wins when the user values the full edit-run-fix loop, not just YAML syntax.

That loop is now materially stronger than it was pre-release:

- YAML that models generate reliably
- stable report schema and failure taxonomy
- machine-readable `error_code` and remediation hints
- MCP tool surface
- curl export for exact request replay
- lifecycle primitives: `setup`, `teardown`, includes, named jars

YAML is an enabler. The moat is the structured debugging loop.

## Target Audience

### Primary

- AI-assisted backend developers
- teams that want file-based API tests the model can patch directly
- CI-oriented teams that prefer structured output over text scraping

### Secondary

- platform and DevOps engineers who want a single-binary HTTP/API check runner
- small teams that do not want a GUI-first API platform

### Not the target

- browser testing users
- teams looking for a broad API management product
- users who specifically need the full Hurl or libcurl protocol surface

## What Tarn Is Strong At Now

The roadmap work materially changed Tarn's credibility. The product is no longer defined by obvious missing HTTP basics.

Current strengths:

- solid HTTP baseline: proxy, TLS controls, redirects, form, multipart, custom methods
- spec-aware cookies with persistence and named jars
- captures and transform-lite that cover common API chaining flows
- multiple outputs, curl export, richer HTML, benchmark thresholds and exports
- project-level config, named environments, include params and overrides
- VS Code extension, formatter, docs site, init templates, conformance suite
- MCP-native workflow with machine-oriented diagnostics

## Strategic Non-Goals

Tarn should still stay narrow.

It should not expand into:

- browser automation
- large platform-style API collaboration features
- arbitrary workflow orchestration
- a full Hurl clone

The remaining Hurl gaps are acceptable when they are off-wedge.

## Current Risks

### Product Risks

- Tarn still does not cover XPath / HTML assertions, certificate inspection, the full Hurl filter DSL, or exotic auth and libcurl-specific transport features.
- YAML has a real complexity ceiling. Tarn should keep adding native features selectively, not recreate a programming language in config.
- Lua is useful as an escape hatch, but over-reliance on Lua weakens the product story and makes agent reasoning worse.

### Market Risks

- The ecosystem is still small even after editor support and distribution improved.
- The product claim depends on keeping the report schema and MCP workflow stable across releases.
- Tarn must keep examples, docs, and conformance coverage aligned or it will lose trust quickly.

## Positioning Guidance

Good framing:

- "API testing that AI agents can write, run, and debug"
- "A CLI API test runner optimized for the write-run-fix loop"
- "Structured JSON output and MCP tools for agent-driven API testing"

Bad framing:

- "better Hurl"
- "YAML is always better than code"
- "general-purpose integration platform"

## What To Emphasize Publicly

- the agent loop: validate, run, inspect structured failure, patch, rerun
- lifecycle and reuse: `setup`, `teardown`, includes, named jars
- local-first distribution: single binary, no runtime, CI-friendly
- concrete migration path from Hurl for common JSON/API suites

## What To Invest In Next

1. Keep the compatibility surface stable: schemas, MCP contract, formatter behavior, examples.
2. Expand conformance coverage and release verification before adding broad new DSL.
3. Close only the remaining feature gaps that have strong migration leverage.
4. Keep documentation brutally current so the product claim stays credible.

## Related Documents

- [`docs/TARN_VS_HURL_COMPARISON.md`](./TARN_VS_HURL_COMPARISON.md)
- [`docs/HURL_MIGRATION.md`](./HURL_MIGRATION.md)
- [`docs/TARN_COMPETITIVENESS_ROADMAP.md`](./TARN_COMPETITIVENESS_ROADMAP.md)
