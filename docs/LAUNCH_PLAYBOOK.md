# Launch Playbook

This is the canonical launch document for Tarn.

## Launch Preconditions

Do not launch broadly until these are true:

- release binaries are uploaded
- install paths are documented
- README quick start is verified end to end
- smoke CI and conformance CI are green
- docs site and VS Code extension are in sync with the shipped surface

## Core Message

Tarn is a CLI-first API testing tool written in Rust.

The positioning to keep repeating:

- tests are YAML
- output is structured JSON for agents and CI
- single binary, no runtime dependencies
- includes an MCP server for Claude Code / Cursor / Windsurf

The core workflow Tarn is optimized for:

1. agent writes a test
2. Tarn runs it
3. structured failure output identifies the mismatch
4. agent fixes the test or app code
5. rerun until green

## Comparison Talking Points

### Tarn vs Hurl

- Hurl is still stronger for XPath / HTML, filter depth, and libcurl-level transport features.
- Tarn is stronger for AI-assisted write-run-debug loops.
- Tarn uses YAML the model already knows, structured failure JSON, MCP integration, lifecycle, and curl export.
- Tarn also combines API testing and lightweight benchmarking in one binary.

### Tarn vs Bruno CLI

- Bruno has a broader ecosystem, GUI workflows, and richer auth/import surfaces today.
- Tarn is smaller, single-binary, easier to drop into CI, and more focused on machine-readable execution output.
- Tarn's MCP story is materially stronger for Claude Code / Cursor workflows.

### Tarn vs StepCI

- StepCI is strong on OpenAPI-driven flows and schema-aware generation.
- Tarn is stronger when the starting point is "describe an endpoint and let the agent iterate".
- Tarn keeps the whole loop local and binary-first instead of Node-first.

### Short reusable lines

- "Hurl is great for handwritten HTTP specs; Tarn is for write-run-debug loops with AI agents."
- "Bruno is a broader API client platform; Tarn is a narrower CLI runner with a stronger machine-readable contract."
- "StepCI starts from specs; Tarn starts from executable tests the model can edit directly."

## Channel Notes

### Hacker News

- Post only after release assets and README are final.
- Link directly to the repo.
- Be ready to answer comparison questions quickly.

### r/rust

- Lead with the single-binary Rust CLI story.
- Keep the AI angle, but do not make it the only angle.

### Dev.to / Hashnode

- Publish a workflow article:
  `Generating, running, and fixing API tests with Tarn and Claude Code`
- Reuse the comparison talking points above.

### awesome-mcp-servers

- Submit after MCP docs and the public workflow demo are live.

### Social Clip

- Keep it to 30-60 seconds.
- Show:
  generate test -> run -> JSON failure -> fix -> green

## Show HN Draft

### Title

`Show HN: Tarn, a single-binary API test runner built for Claude/Cursor workflows`

### Body

Tarn is a CLI-first API testing tool written in Rust.

- Tests are YAML (`.tarn.yaml`)
- Output is structured JSON for agents and CI
- Single binary, no runtime dependencies
- Includes an MCP server for Claude Code / Cursor / Windsurf

The loop we optimized for is:

1. agent writes a test
2. Tarn runs it
3. JSON failure output points to the exact mismatch
4. agent fixes the test or app code
5. rerun until green

## Questions To Expect

- why not Hurl / Bruno / Postman?
- how complete is Hurl parity really?
- why no OpenAPI-first workflow yet?
- how safe is Lua scripting?
- what does MCP buy over `tarn run --format json`?
