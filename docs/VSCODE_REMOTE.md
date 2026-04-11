# Tarn VS Code Extension — Remote Environment Compatibility

This is the full audit writeup for NAZ-283 (Phase 5 remote compatibility work). It complements the short "Remote Setups" section in `editors/vscode/README.md` by recording **what was checked**, **what was fixed at the root cause**, and **the per-environment checklist**.

The four remote setups covered here are the ones VS Code ships first-class support for:

1. **Dev Container** (`ms-vscode-remote.remote-containers`)
2. **GitHub Codespaces** (browser or desktop; uses the Dev Container engine under the hood)
3. **WSL** (`ms-vscode-remote.remote-wsl`)
4. **Remote SSH** (`ms-vscode-remote.remote-ssh`)

Honesty note: the audit was performed on paper against the extension source and VS Code's Remote Development documentation, not by spinning up each of the four environments end-to-end. The deliverable is the code audit plus the setup docs and snippet. Live smoke-tests per environment are tracked as a follow-up.

## 1. Mental model — why the extension is mostly remote-safe by construction

VS Code's Remote Development model has one invariant that makes the extension's life easy: **extensions that declare themselves as "workspace" extensions always run on the same machine as the files**. For Tarn that means:

| Setup | Where the extension process lives | Where `tarn` is spawned | Where `fsPath` values point |
|---|---|---|---|
| Local | Your machine | Your machine | Your machine |
| Dev Container | Inside the container | Inside the container | Inside the container (`/workspaces/...`) |
| Codespaces | Inside the Codespace container | Inside the Codespace container | Inside the Codespace |
| WSL | Inside the WSL distro (Linux) | Inside the WSL distro | Inside WSL (`/mnt/c/...` or `/home/...`) |
| Remote SSH | On the remote host | On the remote host | On the remote host |

The extension never needs to translate paths across the boundary, because it never crosses the boundary in the first place. Node's `path` module, `vscode.Uri.fsPath`, and `child_process.spawn` all use the conventions of whichever OS the extension host is running on, and the Tarn binary is spawned on that same OS.

## 2. Audit — what the code does and does not do

### 2.1 Binary resolution (`src/backend/binaryResolver.ts`)

```ts
const { binaryPath } = readConfig(scope);
await execFileAsync(binaryPath, ["--version"], { timeout: 5000 });
```

- The setting is read via `vscode.workspace.getConfiguration("tarn", scope)`, which honors the **remote settings scope** automatically: when the extension runs on a Remote SSH host, `binaryPath` is read from the *remote* `settings.json`, not the local one.
- No `process.platform` branching, no `.exe` suffix handling, no PATH walking, no hardcoded paths. `execFile` delegates to the OS's `execvp` (POSIX) or `CreateProcess` (Windows) — whichever the extension host happens to be running on.
- Verdict: **safe**. The extension does the right thing on every remote target *as long as the setting is scoped correctly* (see §2.4).

### 2.2 Argv construction (`src/backend/TarnProcessRunner.ts`, `runArgs.ts`)

The runner only builds argv out of three things:

1. Tarn CLI flag strings (`--format`, `--env`, `--tag`, …) — platform-neutral.
2. Relative file paths computed via `path.relative(cwd, f.uri.fsPath)` in `src/testing/runHandler.ts`.
3. Temp file paths computed via `path.join(os.tmpdir(), ...)` inside `TarnProcessRunner.runNdjson`, `runHtmlReport`, and `formatDocument`.

All three use Node's `path` module, which resolves at runtime to `path.posix` on Linux/macOS and `path.win32` on Windows. Because the extension always runs on the same OS as the Tarn binary, the argv's separators always match what the binary expects.

Greps for hazards returned clean:

```
$ rg 'process\.platform|path\.sep|win32|\\\\\\\\|C:\\\\\\\\' editors/vscode/src
(no matches in path/spawn code — only escape-sequence helpers)
```

Verdict: **safe**. No Windows-style separators, no platform branching, no hardcoded absolute paths. The temp-file paths flow through `os.tmpdir()` which on Linux/macOS is `/tmp` (or `$TMPDIR`) and on Windows is `%TEMP%`. The extension never constructs a path by concatenating strings.

### 2.3 `spawn` options

```ts
spawn(this.binaryPath, args, {
  cwd,
  stdio: ["ignore", "pipe", "pipe"],
  windowsHide: true,
});
```

- `cwd` is always `folder.uri.fsPath`, which is the OS-native absolute path of the workspace folder on whichever side the extension lives.
- `windowsHide: true` is harmless on POSIX; it only suppresses console popups when spawning on Windows.
- No `shell: true` — argv is passed to the OS directly, so there is no shell-quoting attack surface and no "which shell does WSL use" ambiguity. The Tarn binary is exec'd, not `/bin/sh -c "tarn ..."`.

Verdict: **safe**.

### 2.4 Setting scopes — the one fix this audit produced

VS Code settings have a `scope` that controls how they flow between local and remote workspaces:

| Scope | Meaning |
|---|---|
| `application` | Global, user-level only. |
| `window` | Per VS Code window. |
| `resource` | Per workspace folder. Default for most settings. |
| `machine` | Per machine. Cannot be overridden per workspace. Used for things that *must* differ between local and remote (e.g. a binary path). |
| `machine-overridable` | Per machine by default, but the workspace can still override. The right choice for "paths and hosts that usually differ between local and remote, but occasionally a workspace wants to pin them". |

Before this audit, the extension's scopes were:

- `tarn.binaryPath` — `machine-overridable` — correct.
- `tarn.requestTimeoutMs` — `resource` — **wrong**. A slow Remote SSH / WSL / Dev Container host has different network latency than the local machine, so a user who bumps the watchdog to `300000` for a slow remote should not have that value silently leak back into their local workspace runs when they reopen the same repo without the remote. `machine-overridable` is the correct scope: the remote host overrides once, the local side keeps its own default.
- Everything else (`testFileGlob`, `excludeGlobs`, `defaultEnvironment`, `defaultTags`, `parallel`, `jsonMode`, `showCodeLens`, `statusBar.enabled`, `validateOnSave`, `notifications.failure`, `cookieJarMode`) — `resource` — correct. These are workspace/project preferences; they should travel with the workspace, not with the machine.

**Fix:** `tarn.requestTimeoutMs` scope changed from `resource` to `machine-overridable` in `editors/vscode/package.json`. This is the only code-level change the audit produced.

### 2.5 File path values passed to Tarn

`runHandler.ts` passes relative paths computed from `vscode.workspace.getWorkspaceFolder(...)`:

```ts
files: filesToRun.map((f) => path.relative(cwd, f.uri.fsPath)),
```

On every remote target the extension host sees Linux-side paths (e.g. `/workspaces/foo/tests/health.tarn.yaml` in a Dev Container, `/home/you/foo/tests/health.tarn.yaml` on Remote SSH, `/mnt/c/Users/you/foo/tests/health.tarn.yaml` on WSL with a Windows-mounted project). `path.relative` uses POSIX on all of these, so the argv passed to Tarn is always `tests/health.tarn.yaml` — exactly what Tarn expects.

The scoped-discovery path (`listFile` in `TarnProcessRunner`) is slightly different: it passes the **absolute** `fsPath` to `tarn list --file` so the CLI can resolve the file deterministically without depending on `cwd`. On every remote target this is also a POSIX absolute path.

Verdict: **safe**.

### 2.6 Trust model and untrusted workspaces

`package.json` declares `capabilities.untrustedWorkspaces.supported = "limited"`, meaning:

- In **untrusted** workspaces (Restricted Mode), the extension provides only grammar/snippets/schema validation. It does **not** spawn the Tarn binary or run `validate`.
- In **trusted** workspaces, full behavior is enabled.

This is the correct stance for a tool that shells out to a CLI. It applies identically to remote setups: opening a Dev Container or a Remote SSH workspace for the first time prompts for trust, and the extension stays in limited mode until the user confirms.

Verdict: **safe**.

## 3. Per-environment checklist

Each environment has the same five acceptance criteria from the ticket: activation, binary resolution, test discovery, run, cancellation.

### 3.1 Dev Container

| Check | Expected | Notes |
|---|---|---|
| Activation | `onLanguage:tarn` + `workspaceContains:**/*.tarn.yaml` fire when the container opens with a project that contains any `.tarn.yaml`. | Declared in `package.json` `activationEvents`. |
| Binary resolution | `tarn.binaryPath = "tarn"` resolves via the container's PATH. | Base image `mcr.microsoft.com/devcontainers/rust:1-bookworm` adds `/usr/local/cargo/bin` to PATH for the `vscode` user; `postCreateCommand` installs `tarn-cli` there. |
| Test discovery | `WorkspaceIndex` walks the container filesystem and discovers tests, preferring scoped `tarn list --file`. | Paths inside the container are POSIX; nothing to translate. |
| Run | `spawn("tarn", args, { cwd })` runs inside the container against POSIX argv. | Same code path as local Linux. |
| Cancellation | SIGINT then SIGKILL on `CancellationToken`. | No shell wrapping the child, so signals reach Tarn directly. |

**Snippet:** `editors/vscode/media/remote/devcontainer.json` (copy to `.devcontainer/devcontainer.json`). Includes the extension recommendation, the `tarn-cli` install step, and an explicit `PATH += /usr/local/cargo/bin` in `containerEnv`.

**Smoke-test command (run once the container is open):** `tarn --version && tarn validate tests/health.tarn.yaml`.

### 3.2 GitHub Codespaces

| Check | Expected | Notes |
|---|---|---|
| Activation | Same as Dev Container. | Codespaces reuses the Dev Container engine. |
| Binary resolution | `tarn` from PATH. | Same `postCreateCommand`; prebuild it to avoid paying the `cargo install` cost on every new Codespace. |
| Test discovery | Same as Dev Container. | — |
| Run | Same. | — |
| Cancellation | Same. | — |

**Config:** reuses `editors/vscode/media/remote/devcontainer.json` unchanged — no separate `codespaces.json`.

**README row:** the VS Code README now lists the extension as a Codespaces pre-install candidate, with instructions to enable prebuilds.

### 3.3 WSL

| Check | Expected | Notes |
|---|---|---|
| Activation | Fires on Linux-side `.tarn.yaml` files. | `workspaceContains` checks the WSL filesystem because the extension host is on the WSL side. |
| Binary resolution | Uses the WSL-side `tarn`, **not** `tarn.exe` on the Windows PATH. | Because the extension host runs inside the WSL distro, `execFile("tarn", ...)` uses the WSL `$PATH`. There is no `.exe` lookup. |
| Test discovery | POSIX argv, POSIX cwd. | `path.relative` uses `path.posix` on the Linux extension host. |
| Run | POSIX argv. | `/mnt/c/...` paths still work — they are valid POSIX paths from WSL's perspective. |
| Cancellation | SIGINT then SIGKILL. | — |

**Minimum config:** none. Install the extension "in WSL" from the Extensions view, make sure `tarn` is on your WSL `$PATH` (`cargo install tarn-cli` or a prebuilt release dropped into `~/.local/bin`).

**Caveat:** if a user sets `tarn.binaryPath` on the *Windows* side (User settings) and then opens a WSL workspace, the WSL-side settings take precedence because `binaryPath` is `machine-overridable`. Good.

### 3.4 Remote SSH

| Check | Expected | Notes |
|---|---|---|
| Activation | Fires on remote-side `.tarn.yaml` files. | Extension host lives on the remote host. |
| Binary resolution | Uses the **remote host's** PATH. | `execFile` runs on the remote host. |
| Test discovery | Remote-side absolute paths. | `fsPath` is POSIX on Linux remotes, Windows-native on Windows remotes. |
| Run | Remote-side argv. | — |
| Cancellation | SIGINT then SIGKILL. | — |

**Minimum config:** if `tarn` is on the remote host's PATH, nothing. If not, set `tarn.binaryPath` to an **absolute remote path** (e.g. `/home/you/.cargo/bin/tarn` or `/usr/local/bin/tarn`). Do not use `~` — VS Code does not expand it when execing. Do not rely on shell profiles like `~/.bashrc` — `execFile` does not source them.

**Caveat:** the `machine-overridable` scope means the binary path and the watchdog timeout are stored per remote host (under `Remote [SSH: host]` in the settings UI). A workspace can still override either, but the default is inherited from the remote machine.

## 4. Summary of fixes the audit produced

1. `tarn.requestTimeoutMs` scope changed from `resource` to `machine-overridable` in `editors/vscode/package.json`. Reason: slow remote hosts should tune the watchdog without polluting the local workspace.
2. `editors/vscode/media/remote/devcontainer.json` added as a drop-in config for Dev Container + Codespaces.
3. Remote Setups section added to `editors/vscode/README.md`.
4. This audit doc.

No changes were needed to `TarnProcessRunner`, `binaryResolver`, `runArgs`, `config.ts`, or any path-handling code: the extension was already remote-safe by construction. The only hazard surfaced by the audit was one misclassified setting scope.

## 5. Follow-ups (out of scope for NAZ-283)

- Live smoke-tests: actually open each of the four environments, create a trivial test file, and verify the full Run / Cancel cycle. Track as a separate ticket.
- Consider whether `tarn.statusBar.enabled` and `tarn.notifications.failure` should be `window`-scoped rather than `resource`-scoped — they are UI preferences, not workspace config. Not urgent.
- Codespaces prebuild recipe: a short `.github/workflows/codespaces-prebuilds.yml` snippet would speed up cold-start further. Documented inline in the README for now.
