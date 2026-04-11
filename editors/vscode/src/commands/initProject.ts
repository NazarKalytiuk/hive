import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";
import type { TarnBackend } from "../backend/TarnBackend";
import { getOutputChannel } from "../outputChannel";

/** Scaffold flavor the wizard can emit. */
export type ScaffoldFlavor = "basic" | "all";

export interface InitProjectOptions {
  /** Absolute path to the directory that will receive the scaffold. */
  folder: string;
  flavor: ScaffoldFlavor;
  /**
   * Optional replacement values for `tarn.env.yaml`. Each key names
   * the env var to overwrite; values that are `undefined` are left
   * alone. Users can also pass fully new keys and they'll be added
   * at the end of the file.
   */
  envOverrides?: Record<string, string>;
}

export interface InitProjectOutcome {
  success: boolean;
  folder: string;
  created: string[];
  deleted: string[];
  validationErrors: number;
  errorMessage?: string;
}

export interface InitProjectDeps {
  backend: TarnBackend;
}

export function registerInitProjectCommand(
  deps: InitProjectDeps,
): vscode.Disposable {
  return vscode.commands.registerCommand("tarn.initProject", async () => {
    await runInitProjectWizard(deps);
  });
}

async function runInitProjectWizard(deps: InitProjectDeps): Promise<void> {
  const folderUri = await pickDestinationFolder();
  if (!folderUri) return;

  const existing = await detectExistingScaffold(folderUri);
  if (existing) {
    const proceedAction = vscode.l10n.t("Proceed");
    const choice = await vscode.window.showWarningMessage(
      vscode.l10n.t(
        "The folder already contains '{0}'. Running Tarn init here will overwrite scaffold files.",
        existing,
      ),
      { modal: true },
      proceedAction,
    );
    if (choice !== proceedAction) return;
  }

  const flavor = await pickScaffoldFlavor();
  if (!flavor) return;

  const envOverrides = await promptEnvOverrides();
  if (envOverrides === undefined) return; // user cancelled

  const outcome = await runInitProject(deps, {
    folder: folderUri.fsPath,
    flavor,
    envOverrides,
  });

  if (!outcome.success) {
    vscode.window.showErrorMessage(
      outcome.errorMessage ??
        vscode.l10n.t("Tarn: init failed. See the output channel."),
    );
    return;
  }

  // Auto-open the health-check fixture so the user can see real
  // Tarn YAML immediately.
  const healthUri = vscode.Uri.file(
    path.join(outcome.folder, "tests", "health.tarn.yaml"),
  );
  try {
    const doc = await vscode.workspace.openTextDocument(healthUri);
    await vscode.window.showTextDocument(doc, { preview: false });
  } catch {
    // If the scaffold changes its layout in the future this open
    // may fail — the wizard still succeeded, so we swallow the error.
  }

  if (outcome.validationErrors > 0) {
    vscode.window.showWarningMessage(
      vscode.l10n.t(
        "Tarn: scaffold created but {0} file(s) failed validation. See the output channel.",
        outcome.validationErrors,
      ),
    );
  } else {
    vscode.window.showInformationMessage(
      vscode.l10n.t(
        "Tarn: project ready in {0} ({1} scaffold).",
        vscode.workspace.asRelativePath(folderUri),
        flavor,
      ),
    );
  }

  // Offer to open the folder if it isn't already part of the
  // workspace — mirrors the previous command's behavior.
  const inWorkspace = vscode.workspace.workspaceFolders?.some(
    (f) => f.uri.fsPath === folderUri.fsPath,
  );
  if (!inWorkspace) {
    const openNewWindow = vscode.l10n.t("Open in New Window");
    const openCurrentWindow = vscode.l10n.t("Open in Current Window");
    const action = await vscode.window.showInformationMessage(
      vscode.l10n.t("Tarn: open the scaffolded project folder?"),
      openNewWindow,
      openCurrentWindow,
    );
    if (action === openNewWindow) {
      await vscode.commands.executeCommand("vscode.openFolder", folderUri, {
        forceNewWindow: true,
      });
    } else if (action === openCurrentWindow) {
      await vscode.commands.executeCommand("vscode.openFolder", folderUri);
    }
  } else {
    await vscode.commands.executeCommand("tarn.refreshDiscovery");
  }
}

/**
 * Drive the scaffold + prune + env rewrite + validate pipeline. Exported so
 * integration tests can run the full flow with explicit paths and
 * overrides, bypassing the VS Code dialogs.
 */
export async function runInitProject(
  deps: InitProjectDeps,
  options: InitProjectOptions,
): Promise<InitProjectOutcome> {
  const out = getOutputChannel();
  // l10n-ignore: debug log for engineers, shown with [tarn] prefix.
  out.appendLine(
    `[tarn] init project in ${options.folder} (flavor=${options.flavor})`,
  );
  const cts = new vscode.CancellationTokenSource();
  const initResult = await deps.backend.initProject(options.folder, cts.token);
  cts.dispose();

  if (initResult.exitCode !== 0) {
    out.appendLine(
      initResult.stderr || initResult.stdout || vscode.l10n.t("tarn init failed"),
    );
    out.show(true);
    return {
      success: false,
      folder: options.folder,
      created: [],
      deleted: [],
      validationErrors: 0,
      errorMessage: vscode.l10n.t(
        "tarn init exited with code {0}",
        String(initResult.exitCode),
      ),
    };
  }
  if (initResult.stdout.trim().length > 0) out.appendLine(initResult.stdout.trimEnd());

  const deleted: string[] = [];
  for (const rel of scaffoldFilesToPrune(options.flavor)) {
    const abs = path.join(options.folder, rel);
    try {
      const stat = await fs.promises.stat(abs);
      if (stat.isDirectory()) {
        await fs.promises.rm(abs, { recursive: true, force: true });
      } else {
        await fs.promises.unlink(abs);
      }
      deleted.push(rel);
    } catch {
      // Path wasn't there — fine, tarn init shape may change over time.
    }
  }

  if (options.envOverrides && Object.keys(options.envOverrides).length > 0) {
    const envPath = path.join(options.folder, "tarn.env.yaml");
    try {
      const raw = await fs.promises.readFile(envPath, "utf8");
      const rewritten = customizeEnvFile(raw, options.envOverrides);
      if (rewritten !== raw) {
        await fs.promises.writeFile(envPath, rewritten, "utf8");
      }
    } catch (err) {
      // l10n-ignore: debug log for engineers, shown with [tarn] prefix.
      out.appendLine(`[tarn] failed to rewrite tarn.env.yaml: ${String(err)}`);
    }
  }

  const validationErrors = await validateGeneratedFiles(deps, options.folder, out);

  const created = await listCreatedFiles(options.folder);
  return {
    success: true,
    folder: options.folder,
    created,
    deleted,
    validationErrors,
  };
}

/**
 * Rewrite a `tarn.env.yaml` file in place, replacing the values of
 * any top-level keys listed in `overrides` without touching
 * comments, blank lines, or unmatched keys. Unknown keys get
 * appended in a small `# Added by Tarn: Init Project Here` block at
 * the bottom so users can always spot what the wizard added.
 *
 * Exported for unit tests.
 */
export function customizeEnvFile(
  content: string,
  overrides: Record<string, string>,
): string {
  const lines = content.split(/\r?\n/);
  const seen = new Set<string>();
  const rewritten = lines.map((line) => {
    const match = line.match(/^(\s*)([A-Za-z_][A-Za-z0-9_]*)\s*:\s*(.*)$/);
    if (!match) return line;
    const [, indent, key] = match;
    if (!(key in overrides)) return line;
    seen.add(key);
    const value = overrides[key];
    return `${indent}${key}: ${formatYamlScalar(value)}`;
  });
  const additions = Object.entries(overrides).filter(([key]) => !seen.has(key));
  if (additions.length === 0) {
    return rewritten.join("\n");
  }
  // Trim trailing blank lines so the appended block sits tight.
  while (rewritten.length > 0 && rewritten[rewritten.length - 1].trim() === "") {
    rewritten.pop();
  }
  rewritten.push("");
  rewritten.push("# Added by Tarn: Init Project Here");
  for (const [key, value] of additions) {
    rewritten.push(`${key}: ${formatYamlScalar(value)}`);
  }
  rewritten.push("");
  return rewritten.join("\n");
}

/**
 * Relative paths (from the scaffold root) that the wizard deletes
 * post-init when the user picked a flavor other than "all". Kept
 * pure so the unit test can lock the mapping down.
 */
export function scaffoldFilesToPrune(flavor: ScaffoldFlavor): string[] {
  if (flavor === "basic") {
    return ["examples", "fixtures"];
  }
  return [];
}

function formatYamlScalar(value: string): string {
  // Quote anything that looks like it could be mis-parsed as a
  // non-string scalar or contains tricky whitespace. The rest we
  // leave bare so single-word values read cleanly.
  if (/^[A-Za-z0-9_./-]+$/.test(value)) {
    return value;
  }
  const escaped = value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
  return `"${escaped}"`;
}

async function pickDestinationFolder(): Promise<vscode.Uri | undefined> {
  type Pick = vscode.QuickPickItem & { value: vscode.Uri | "browse" };
  const workspaces = vscode.workspace.workspaceFolders ?? [];
  const items: Pick[] = [];
  for (const folder of workspaces) {
    items.push({
      label: vscode.l10n.t("$(folder) {0}", folder.name),
      description: folder.uri.fsPath,
      detail: vscode.l10n.t("Scaffold into this workspace folder"),
      value: folder.uri,
    });
  }
  items.push({
    label: vscode.l10n.t("$(folder-opened) Browse…"),
    description: vscode.l10n.t("Pick another folder on disk"),
    value: "browse",
  });
  const picked = await vscode.window.showQuickPick(items, {
    placeHolder: vscode.l10n.t("Where should Tarn scaffold the new project?"),
  });
  if (!picked) return undefined;
  if (picked.value === "browse") {
    const picks = await vscode.window.showOpenDialog({
      canSelectFiles: false,
      canSelectFolders: true,
      canSelectMany: false,
      openLabel: vscode.l10n.t("Initialize Tarn here"),
    });
    return picks?.[0];
  }
  return picked.value;
}

async function pickScaffoldFlavor(): Promise<ScaffoldFlavor | undefined> {
  type Item = vscode.QuickPickItem & { value: ScaffoldFlavor };
  const items: Item[] = [
    {
      label: vscode.l10n.t("$(rocket) All templates (recommended)"),
      description: vscode.l10n.t(
        "health check + auth / multipart / polling / multi-user examples",
      ),
      value: "all",
    },
    {
      label: vscode.l10n.t("$(file-code) Basic"),
      description: vscode.l10n.t(
        "just the health check and configs — no examples/ folder",
      ),
      value: "basic",
    },
  ];
  const picked = await vscode.window.showQuickPick(items, {
    placeHolder: vscode.l10n.t("Pick a scaffold flavor"),
  });
  return picked?.value;
}

async function promptEnvOverrides(): Promise<
  Record<string, string> | undefined
> {
  const answer = await vscode.window.showQuickPick(
    [
      {
        label: vscode.l10n.t("$(check) Use defaults"),
        description: vscode.l10n.t(
          "base_url=http://localhost:3000, admin@example.com / secret",
        ),
        value: false,
      },
      {
        label: vscode.l10n.t("$(edit) Customize env values"),
        description: vscode.l10n.t("Prompt for base_url and admin credentials"),
        value: true,
      },
    ],
    { placeHolder: vscode.l10n.t("Customize starter env values?") },
  );
  if (!answer) return undefined;
  if (!answer.value) return {};

  const overrides: Record<string, string> = {};
  const baseUrl = await vscode.window.showInputBox({
    prompt: vscode.l10n.t("Base URL for the API under test"),
    value: "http://localhost:3000",
    validateInput: (raw) =>
      /^https?:\/\//.test(raw.trim())
        ? undefined
        : vscode.l10n.t("Enter a URL starting with http:// or https://"),
  });
  if (baseUrl === undefined) return undefined;
  overrides.base_url = baseUrl.trim();

  const adminEmail = await vscode.window.showInputBox({
    prompt: vscode.l10n.t("Admin email (used by the auth-flow template)"),
    value: "admin@example.com",
    validateInput: (raw) =>
      raw.includes("@")
        ? undefined
        : vscode.l10n.t("Enter a valid email address"),
  });
  if (adminEmail === undefined) return undefined;
  overrides.admin_email = adminEmail.trim();

  const adminPassword = await vscode.window.showInputBox({
    prompt: vscode.l10n.t("Admin password (stored as plaintext — only for local dev!)"),
    value: "secret",
    password: true,
  });
  if (adminPassword === undefined) return undefined;
  overrides.admin_password = adminPassword;

  return overrides;
}

async function detectExistingScaffold(folder: vscode.Uri): Promise<string | undefined> {
  const candidates = ["tarn.config.yaml", "tarn.env.yaml", "tests", "examples"];
  for (const name of candidates) {
    try {
      await vscode.workspace.fs.stat(vscode.Uri.joinPath(folder, name));
      return name;
    } catch {
      // not present, keep going
    }
  }
  return undefined;
}

async function validateGeneratedFiles(
  deps: InitProjectDeps,
  folder: string,
  out: vscode.OutputChannel,
): Promise<number> {
  const files: string[] = [];
  async function walk(dir: string): Promise<void> {
    let entries: fs.Dirent[];
    try {
      entries = await fs.promises.readdir(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const abs = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        await walk(abs);
      } else if (entry.name.endsWith(".tarn.yaml") || entry.name.endsWith(".tarn.yml")) {
        files.push(abs);
      }
    }
  }
  await walk(folder);
  if (files.length === 0) return 0;

  const cts = new vscode.CancellationTokenSource();
  try {
    const report = await deps.backend.validateStructured(files, folder, cts.token);
    if (!report) return 0;
    let errors = 0;
    for (const fileResult of report.files) {
      if (fileResult.valid) continue;
      errors += fileResult.errors.length;
      // l10n-ignore: debug log for engineers, shown with [tarn] prefix.
      out.appendLine(
        `[tarn] validate: ${fileResult.file} has ${fileResult.errors.length} error(s)`,
      );
      for (const err of fileResult.errors) {
        const at = err.line !== undefined ? ` line ${err.line}` : "";
        // l10n-ignore: debug log line following the validate summary above.
        out.appendLine(`  -${at} ${err.message}`);
      }
    }
    return errors;
  } finally {
    cts.dispose();
  }
}

async function listCreatedFiles(folder: string): Promise<string[]> {
  const created: string[] = [];
  async function walk(dir: string): Promise<void> {
    let entries: fs.Dirent[];
    try {
      entries = await fs.promises.readdir(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const abs = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        await walk(abs);
      } else {
        created.push(path.relative(folder, abs));
      }
    }
  }
  await walk(folder);
  return created.sort();
}
