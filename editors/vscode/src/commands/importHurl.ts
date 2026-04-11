import * as path from "path";
import * as vscode from "vscode";
import type { TarnBackend } from "../backend/TarnBackend";
import { getOutputChannel } from "../outputChannel";

export interface ImportHurlDeps {
  backend: TarnBackend;
}

export function registerImportHurlCommand(
  deps: ImportHurlDeps,
): vscode.Disposable {
  return vscode.commands.registerCommand("tarn.importHurl", async () => {
    await runImportHurlWizard(deps);
  });
}

async function runImportHurlWizard(deps: ImportHurlDeps): Promise<void> {
  const source = await pickHurlSource();
  if (!source) return;

  const defaultDest = vscode.Uri.file(defaultHurlDestination(source.fsPath));
  const dest = await vscode.window.showSaveDialog({
    defaultUri: defaultDest,
    filters: { Tarn: ["tarn.yaml", "tarn.yml", "yaml", "yml"] },
    saveLabel: vscode.l10n.t("Import Hurl File"),
    title: vscode.l10n.t("Choose destination for imported Tarn file"),
  });
  if (!dest) return;

  const cwd = resolveCwd(source);
  const result = await runImportHurl(deps.backend, source.fsPath, dest.fsPath, cwd);
  if (!result.success) {
    const message =
      result.exitCode !== null
        ? vscode.l10n.t(
            "Tarn: import-hurl failed (exit {0}). Check the Tarn output channel for details.",
            String(result.exitCode),
          )
        : vscode.l10n.t(
            "Tarn: import-hurl failed. Check the Tarn output channel for details.",
          );
    vscode.window.showErrorMessage(message);
    return;
  }

  try {
    const doc = await vscode.workspace.openTextDocument(dest);
    await vscode.window.showTextDocument(doc, { preview: false });
  } catch (err) {
    vscode.window.showErrorMessage(
      vscode.l10n.t("Tarn: imported file but could not open it: {0}", String(err)),
    );
    return;
  }

  const runAction = vscode.l10n.t("Run");
  const validateAction = vscode.l10n.t("Validate");
  const action = await vscode.window.showInformationMessage(
    vscode.l10n.t("Tarn: imported {0}", path.basename(dest.fsPath)),
    runAction,
    validateAction,
  );
  if (action === runAction) {
    await vscode.commands.executeCommand("tarn.runFile");
  } else if (action === validateAction) {
    await vscode.commands.executeCommand("tarn.validateFile");
  }
}

async function pickHurlSource(): Promise<vscode.Uri | undefined> {
  const uris = await vscode.window.showOpenDialog({
    canSelectFiles: true,
    canSelectFolders: false,
    canSelectMany: false,
    openLabel: vscode.l10n.t("Import Hurl File"),
    title: vscode.l10n.t("Select a .hurl file to import"),
    filters: { Hurl: ["hurl"] },
  });
  return uris?.[0];
}

function resolveCwd(source: vscode.Uri): string {
  const folder = vscode.workspace.getWorkspaceFolder(source);
  return folder?.uri.fsPath ?? path.dirname(source.fsPath);
}

/**
 * Drive the backend's `importHurl` with the supplied paths, log
 * outcomes, and return a success flag. Split out from the wizard so
 * the integration test can exercise the spawn-and-open path without
 * driving the VS Code dialogs.
 */
export async function runImportHurl(
  backend: TarnBackend,
  source: string,
  dest: string,
  cwd: string,
): Promise<{ success: boolean; exitCode: number | null; stderr: string }> {
  const out = getOutputChannel();
  const cts = new vscode.CancellationTokenSource();
  // l10n-ignore: debug log for engineers, shown with [tarn] prefix.
  out.appendLine(`[tarn] import-hurl ${source} -> ${dest}`);
  const result = await vscode.window.withProgress(
    {
      location: vscode.ProgressLocation.Notification,
      title: vscode.l10n.t("Tarn: importing {0}…", path.basename(source)),
      cancellable: true,
    },
    async (_progress, token) => {
      token.onCancellationRequested(() => cts.cancel());
      return backend.importHurl(source, dest, cwd, cts.token);
    },
  );
  cts.dispose();
  if (result.exitCode !== 0) {
    if (result.stderr) out.appendLine(result.stderr.trimEnd());
    if (result.stdout) out.appendLine(result.stdout.trimEnd());
    out.show(true);
    return { success: false, exitCode: result.exitCode, stderr: result.stderr };
  }
  if (result.stdout.trim().length > 0) {
    out.appendLine(result.stdout.trimEnd());
  }
  return { success: true, exitCode: result.exitCode, stderr: result.stderr };
}

/**
 * Compute the default destination path for an imported Hurl file.
 * Strips the `.hurl` suffix (preserving any prior segments) and
 * appends `.tarn.yaml` as a sibling. Used by the save dialog and
 * also exported for unit tests.
 */
export function defaultHurlDestination(sourcePath: string): string {
  const dir = path.dirname(sourcePath);
  const base = path.basename(sourcePath);
  const stem = base.toLowerCase().endsWith(".hurl")
    ? base.slice(0, -".hurl".length)
    : base;
  return path.join(dir, `${stem}.tarn.yaml`);
}
