import * as vscode from "vscode";
import type { TarnBackend } from "../backend/TarnBackend";
import { getOutputChannel } from "../outputChannel";

/**
 * DocumentFormattingEditProvider that routes through `tarn fmt`.
 *
 * Because the Tarn CLI has no stdin or `--stdout` mode, the backend
 * copies the document content into a tmp `.tarn.yaml` file, runs
 * `tarn fmt` on it, and reads the result back. The provider turns
 * that result into a single `TextEdit` covering the whole document
 * so a plain Format Document action undoes as one step.
 *
 * If the content is already canonical, the provider returns an empty
 * edit array so VS Code does not dirty-mark the file. If `tarn fmt`
 * fails with a parse error, the failure is logged to the output
 * channel and the buffer is left untouched — formatting an invalid
 * file should never corrupt it.
 */
export class TarnFormatProvider implements vscode.DocumentFormattingEditProvider {
  constructor(private readonly backend: TarnBackend) {}

  async provideDocumentFormattingEdits(
    document: vscode.TextDocument,
    _options: vscode.FormattingOptions,
    token: vscode.CancellationToken,
  ): Promise<vscode.TextEdit[]> {
    const folder = vscode.workspace.getWorkspaceFolder(document.uri);
    if (!folder) {
      return [];
    }

    const original = document.getText();
    const { formatted, error } = await this.backend.formatDocument(
      original,
      folder.uri.fsPath,
      token,
    );

    if (token.isCancellationRequested) {
      return [];
    }

    if (error) {
      const out = getOutputChannel();
      out.appendLine(`[tarn fmt] ${document.uri.fsPath}: ${error}`);
      vscode.window.showWarningMessage(
        "Tarn: format failed. Fix the parse error first. See the Tarn output channel for details.",
      );
      return [];
    }

    if (formatted === original) {
      return [];
    }

    const fullRange = new vscode.Range(
      document.positionAt(0),
      document.positionAt(original.length),
    );
    return [vscode.TextEdit.replace(fullRange, formatted)];
  }
}
