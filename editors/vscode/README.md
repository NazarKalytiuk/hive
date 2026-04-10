# Tarn VS Code Extension

First-class editor support for Tarn API test files.

## Features

### Test Explorer

- Hierarchical discovery of `*.tarn.yaml` across the workspace: file → test → step.
- Run and Dry Run profiles.
- Cancellable runs.
- Rich failure messages: expected vs actual, unified diff, request, response, remediation hints, failure category, error code.

### Editor

- CodeLens above every test and step: `Run`, `Dry Run`, `Run step`.
- File-level schema validation for `*.tarn.yaml` via `redhat.vscode-yaml` and the Tarn JSON schema.
- Snippet library for common test patterns (`tarn-test`, `tarn-step`, `tarn-capture`, `tarn-poll`, `tarn-form`, `tarn-graphql`, `tarn-multipart`, `tarn-lifecycle`, `tarn-include`).
- Tarn-aware syntax highlighting for interpolation, JSONPath, and assertion operators.

### Commands

| Command | Description |
|---|---|
| `Tarn: Run All Tests` | Runs every discovered test file. |
| `Tarn: Run Current File` | Runs only the active `.tarn.yaml`. |
| `Tarn: Dry Run Current File` | Interpolates but does not send requests. |
| `Tarn: Validate Current File` | Invokes `tarn validate`. |
| `Tarn: Rerun Last Run` | Reuses the last run request. |
| `Tarn: Select Environment…` | Picks an environment from discovered `tarn.env.*.yaml`. |
| `Tarn: Set Tag Filter…` | Applies a comma-separated tag filter. |
| `Tarn: Show Output` | Focuses the Tarn output channel. |
| `Tarn: Install / Update Tarn` | Opens install instructions. |

### Status bar

- Left: active environment (click to pick).
- Right: last run summary (click to open output).

## Settings

All settings live under the `tarn.*` namespace. The most useful are:

- `tarn.binaryPath` — path to the Tarn CLI binary. Defaults to `tarn`.
- `tarn.testFileGlob` — discovery glob. Defaults to `**/*.tarn.yaml`.
- `tarn.excludeGlobs` — excluded globs. Defaults to `["**/target/**","**/node_modules/**","**/.git/**"]`.
- `tarn.defaultEnvironment` — environment passed as `--env` when nothing is picked.
- `tarn.defaultTags` — default tag filter.
- `tarn.parallel` — toggle `--parallel`.
- `tarn.jsonMode` — `verbose` or `compact`.
- `tarn.showCodeLens` — toggle CodeLens actions.
- `tarn.statusBar.enabled` — toggle the status bar entries.

See the full list in the VS Code Settings UI under `Extensions → Tarn`.

## Requirements

- Tarn CLI (`tarn`) on `PATH`, or a custom path configured via `tarn.binaryPath`.
- [`redhat.vscode-yaml`](https://marketplace.visualstudio.com/items?itemName=redhat.vscode-yaml) (declared as an extension dependency; installed automatically).

## Install Locally

1. From the `editors/vscode` folder, run `npm install && npm run build`.
2. `Developer: Install Extension from Location…` in VS Code and pick the `editors/vscode` folder.
3. Or run `npm run package` to build a VSIX, then `Extensions: Install from VSIX…`.

## Trusted vs Untrusted Workspaces

In untrusted workspaces the extension provides read-only features only (grammar, snippets, schema validation). Running tests, validating files, and spawning the Tarn binary are disabled until the workspace is trusted.

## What Gets Wired

- `*.tarn.yaml`, `*.tarn.yml` → language id `tarn`.
- Tarn test schema → `schemas/v1/testfile.json`.
- JSON report schema → `schemas/v1/report.json` for `tarn-report.json` and `*.tarn-report.json`.

## Release

The extension publishes to both the [VS Code Marketplace](https://marketplace.visualstudio.com/) and [Open VSX](https://open-vsx.org/) from tagged releases via `.github/workflows/vscode-extension-release.yml`.

### One-time setup

1. **Verify the publisher** on both marketplaces for `nazarkalytiuk` (manual, one-time on each site).
2. **Create marketplace PATs** and add them to the repo under **Settings → Secrets and variables → Actions**:
   - `VSCE_PAT` — from [dev.azure.com/<publisher>/_usersSettings/tokens](https://dev.azure.com/) with the `Marketplace › Manage` scope.
   - `OVSX_PAT` — from [open-vsx.org/user-settings/tokens](https://open-vsx.org/user-settings/tokens).
3. **Create the Open VSX namespace** (once): `npx ovsx create-namespace nazarkalytiuk -p "$OVSX_PAT"`.

### Cutting a release

1. Bump `editors/vscode/package.json` to the new version. The workflow will fail if `package.json` disagrees with the tag.
2. Add a `## <version>` section to `editors/vscode/CHANGELOG.md`.
3. Commit and tag: `git tag v<version> && git push origin v<version>`.
4. The `Release` workflow (Rust binaries) runs first. The `VS Code Extension Release` workflow runs in parallel, waits for the binary release to appear on GitHub, then packages and publishes the VSIX to both marketplaces.
5. Re-runs: if either publish fails, re-invoke via **Actions → VS Code Extension Release → Run workflow** with the failed tag as the `tag` input.

### Pre-releases

Tags with a hyphen suffix (e.g. `v0.19.0-rc.1`, `v0.20.0-beta`) are published with the `--pre-release` flag on both marketplaces. Stable tags (`v0.19.0`) publish as regular releases.

### Local dry-run

To verify the VSIX builds cleanly without publishing:

```bash
cd editors/vscode
npm ci
npm run lint
npm run test:unit
npm run build
npx vsce package --no-dependencies --out tarn-vscode.vsix
```

## Roadmap

See `docs/VSCODE_EXTENSION.md` for the full phased plan and Tarn-side dependencies (`T51`–`T57`).
