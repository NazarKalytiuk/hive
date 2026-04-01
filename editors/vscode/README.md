# Tarn VS Code Extension

This extension packages the editor support needed for Tarn test authoring:

- file association for `*.tarn.yaml` and `*.tarn.yml`
- Tarn-aware syntax highlighting for interpolation, JSONPath, and assertion operators
- schema wiring for Tarn test files and JSON reports
- snippets for the most common request, auth, capture, include, and polling patterns

## Install Locally

1. Open VS Code.
2. Run `Extensions: Install from VSIX...` if you packaged the folder, or use `Developer: Install Extension from Location...`.
3. Select `editors/vscode`.

The extension declares `redhat.vscode-yaml` as a dependency because Tarn test validation rides on the YAML language server.

If you want a packaged artifact instead of loading the folder directly, use `vsce package` from `editors/vscode/`.

## What Gets Wired

- `*.tarn.yaml`, `*.tarn.yml` -> language id `tarn`
- Tarn test schema -> `schemas/v1/testfile.json`
- JSON report schema -> `schemas/v1/report.json` for `tarn-report.json` and `*.tarn-report.json`

## Snippet Prefixes

- `tarn-test`
- `tarn-step`
- `tarn-capture`
- `tarn-poll`
- `tarn-form`
- `tarn-graphql`
- `tarn-multipart`
- `tarn-lifecycle`
- `tarn-include`
