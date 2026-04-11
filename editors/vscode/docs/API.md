# Tarn VS Code Extension — Public API

A quick reference for extensions and scripts that want to consume the Tarn VS Code extension programmatically. The canonical type definition lives in [`../src/api.ts`](../src/api.ts); this document is the user-facing summary.

## tl;dr

```ts
import type { TarnExtensionApi } from "nazarkalytiuk.tarn-vscode";
import * as vscode from "vscode";

async function getTarn(): Promise<TarnExtensionApi | undefined> {
  const ext = vscode.extensions.getExtension<TarnExtensionApi>(
    "nazarkalytiuk.tarn-vscode",
  );
  if (!ext) return undefined;
  return ext.activate();
}
```

`activate()` returns `undefined` in untrusted workspaces. Always handle that case — the extension deliberately exposes zero surface until the user grants trust.

## Shape

| Field | Type | Stability |
|---|---|---|
| `testControllerId` | `string` | **stable** |
| `indexedFileCount` | `number` | **stable** |
| `commands` | `readonly string[]` | **stable** |
| `testing` | `TarnExtensionTestingApi` | **internal** |

### `testControllerId` (stable)

The id of the `vscode.TestController` that drives the extension's Test Explorer view. Use it via the VS Code Testing API if you need to correlate test runs across extensions.

### `indexedFileCount` (stable)

Number of `.tarn.yaml` files tracked by the workspace index at the moment `activate()` resolved. This is a **one-shot snapshot**, not a live value — if files are added after activation, this number will not update. Use the Testing API or a `FileSystemWatcher` for live counts.

### `commands` (stable)

The full list of command ids the extension contributes, e.g. `"tarn.runAll"`, `"tarn.runFile"`, `"tarn.selectEnvironment"`. Useful for extensions that want to render a Tarn palette or a QuickPick over Tarn actions without hard-coding command ids. The order of the array is not guaranteed.

### `testing` (internal)

Opaque, test-only sub-object. **No compatibility guarantees whatsoever.** Its shape may change between any two releases — including patch releases — without a changelog entry. It exists solely so the extension's own `@vscode/test-electron` integration tests can poke at internal state (workspace index, notifier, fix plan view, run history store, etc.). Downstream code that reads `testing.*` will break silently on upgrade. Do not use `testing` from production code.

If you think you need something that only `testing` currently exposes, open an issue so we can promote the underlying capability to a `stable` or `preview` field instead.

## Stability tiers

- **stable** — breaking changes require a major version bump (`1.x.y` → `2.0.0`). Removing a field, renaming a field, narrowing a return type, or widening a parameter type are all breaking. Adding a new optional field to a stable object is NOT breaking.
- **preview** — may change in any minor release. Preview fields are shipped so integrators can experiment and give feedback. Always listed explicitly in this document before you depend on one. There are currently no preview fields.
- **internal** — no compatibility guarantees. Do not use.

## Semver policy

The extension follows semantic versioning for its **public API**, not for its user-facing VS Code behavior. The user-facing side (commands, settings, views) is free to iterate — adding a new command or changing a setting default only needs a changelog entry, not a major bump. The public API side is frozen as described above.

Internal fields, and only internal fields, are allowed to change in patch releases. Every other level of change is bound by the stability tier of the affected field: preview bumps minor, stable bumps major.

## 1.0.0 gate

Until the extension ships `1.0.0`, the stable surface is still subject to one last round of pruning. When `1.0.0` ships, every field currently marked `@stability stable` in `src/api.ts` is frozen under the semver policy above, and the set of stable fields is locked to whatever `api.ts` declares at tag time.

Between now and then, the extension keeps shipping normal minor releases on the `0.x` track. The `1.0.0` cut is a deliberate, coordinated event — if you are building against the API today, you are building against `0.x` and should pin your dependency to a minor range.

## Enforcement

A golden-snapshot test at [`../tests/unit/apiSurface.test.ts`](../tests/unit/apiSurface.test.ts) compares a normalized version of [`../src/api.ts`](../src/api.ts) against [`../tests/golden/api.snapshot.txt`](../tests/golden/api.snapshot.txt). Any edit to the interface declaration — adding a field, removing a field, renaming a field, changing a stability annotation, changing the semver policy prose, or changing an imported type — fails the test unless the golden is updated in the same commit. The test is picked up by `npm run test:unit` and therefore runs on every PR, so an API drift that is not accompanied by a documented change gets caught locally before it ever reaches review.

For the full roadmap context and the prose version of this document, see [`../../../docs/VSCODE_EXTENSION.md`](../../../docs/VSCODE_EXTENSION.md), section "Public API".
