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
      `Tarn binary not found at '${binaryPath}'. Set 'tarn.binaryPath' in settings or install tarn. Cause: ${String(
        cause,
      )}`,
    );
    this.name = "BinaryNotFoundError";
  }
}

export async function resolveBinary(scope?: vscode.Uri): Promise<ResolvedBinary> {
  const { binaryPath } = readConfig(scope);
  try {
    const { stdout } = await execFileAsync(binaryPath, ["--version"], { timeout: 5000 });
    const version = stdout.trim();
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
    const choice = await vscode.window.showErrorMessage(
      err instanceof Error ? err.message : String(err),
      "Install Instructions",
      "Open Settings",
    );
    if (choice === "Install Instructions") {
      await vscode.env.openExternal(
        vscode.Uri.parse("https://github.com/NazarKalytiuk/hive#install"),
      );
    } else if (choice === "Open Settings") {
      await vscode.commands.executeCommand("workbench.action.openSettings", "tarn.binaryPath");
    }
    return undefined;
  }
}
