import { execFile } from "child_process";
import { promisify } from "util";
import * as vscode from "vscode";
import { getOutputChannel } from "../outputChannel";
import { readConfig } from "../config";

const execFileAsync = promisify(execFile);

export interface ResolvedBinary {
  path: string;
  version: string;
}

export class BinaryNotFoundError extends Error {
  constructor(binaryPath: string, cause: unknown) {
    super(
      vscode.l10n.t(
        "Tarn binary not found at '{0}'. Set 'tarn.binaryPath' in settings or install tarn. Cause: {1}",
        binaryPath,
        String(cause),
      ),
    );
    this.name = "BinaryNotFoundError";
  }
}

export async function resolveBinary(scope?: vscode.Uri): Promise<ResolvedBinary> {
  const { binaryPath } = readConfig(scope);
  try {
    const { stdout } = await execFileAsync(binaryPath, ["--version"], { timeout: 5000 });
    const version = stdout.trim();
    // l10n-ignore: debug log for engineers, shown with [tarn] prefix.
    getOutputChannel().appendLine(`[tarn] resolved binary ${binaryPath} (${version})`);
    return { path: binaryPath, version };
  } catch (err) {
    throw new BinaryNotFoundError(binaryPath, err);
  }
}

export async function promptInstallIfMissing(scope?: vscode.Uri): Promise<ResolvedBinary | undefined> {
  try {
    return await resolveBinary(scope);
  } catch (err) {
    const installAction = vscode.l10n.t("Install Instructions");
    const settingsAction = vscode.l10n.t("Open Settings");
    const choice = await vscode.window.showErrorMessage(
      err instanceof Error ? err.message : String(err),
      installAction,
      settingsAction,
    );
    if (choice === installAction) {
      await vscode.env.openExternal(
        vscode.Uri.parse("https://github.com/NazarKalytiuk/hive#install"),
      );
    } else if (choice === settingsAction) {
      await vscode.commands.executeCommand("workbench.action.openSettings", "tarn.binaryPath");
    }
    return undefined;
  }
}
