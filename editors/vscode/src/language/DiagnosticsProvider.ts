import * as vscode from "vscode";
import type { TarnBackend } from "../backend/TarnBackend";
import type { ValidateError, ValidateFileResult } from "../util/schemaGuards";
import { readConfig } from "../config";
import { getOutputChannel } from "../outputChannel";

/**
 * Runs `tarn validate --format json` on every save of a `.tarn.yaml`
 * and publishes the resulting errors as `vscode.Diagnostic` entries in
 * a dedicated DiagnosticCollection.
 *
 * Honors the `tarn.validateOnSave` setting. Failing validations clear
 * on the next successful save. Files outside any workspace folder are
 * skipped because the backend needs a cwd. Untrusted workspaces are
 * also skipped (the extension already refuses to spawn there).
 */
export class TarnDiagnosticsProvider implements vscode.Disposable {
  private readonly collection: vscode.DiagnosticCollection;
  private readonly disposables: vscode.Disposable[] = [];
  private readonly inFlight = new Map<string, vscode.CancellationTokenSource>();

  constructor(private readonly backend: TarnBackend) {
    this.collection = vscode.languages.createDiagnosticCollection("tarn");

    this.disposables.push(
      vscode.workspace.onDidSaveTextDocument((doc) => {
        if (this.isTarnDocument(doc)) {
          void this.validate(doc);
        }
      }),
      vscode.workspace.onDidCloseTextDocument((doc) => {
        this.collection.delete(doc.uri);
      }),
    );

    for (const doc of vscode.workspace.textDocuments) {
      if (this.isTarnDocument(doc)) {
        void this.validate(doc);
      }
    }
  }

  async validate(doc: vscode.TextDocument): Promise<void> {
    if (!vscode.workspace.isTrusted) {
      return;
    }
    if (!readConfig(doc.uri).validateOnSave) {
      this.collection.delete(doc.uri);
      return;
    }
    const folder = vscode.workspace.getWorkspaceFolder(doc.uri);
    if (!folder) {
      return;
    }

    const key = doc.uri.toString();
    this.inFlight.get(key)?.cancel();
    const cts = new vscode.CancellationTokenSource();
    this.inFlight.set(key, cts);

    try {
      const report = await this.backend.validateStructured(
        [doc.uri.fsPath],
        folder.uri.fsPath,
        cts.token,
      );

      if (cts.token.isCancellationRequested) {
        return;
      }

      if (!report) {
        this.collection.delete(doc.uri);
        return;
      }

      if (report.error) {
        this.collection.set(doc.uri, [
          this.fileDiagnostic(
            doc,
            vscode.l10n.t("Tarn validate: {0}", report.error),
          ),
        ]);
        return;
      }

      const fileEntry = matchFileEntry(report.files, doc.uri.fsPath);
      if (!fileEntry) {
        this.collection.delete(doc.uri);
        return;
      }

      if (fileEntry.valid) {
        this.collection.delete(doc.uri);
        return;
      }

      const diagnostics = fileEntry.errors.map((err) =>
        this.toDiagnostic(doc, err),
      );
      this.collection.set(doc.uri, diagnostics);
    } catch (err) {
      // l10n-ignore: debug log for engineers, shown with [tarn] prefix.
      getOutputChannel().appendLine(
        `[tarn] validate-on-save failed for ${doc.uri.fsPath}: ${String(err)}`,
      );
    } finally {
      if (this.inFlight.get(key) === cts) {
        this.inFlight.delete(key);
      }
      cts.dispose();
    }
  }

  private isTarnDocument(doc: vscode.TextDocument): boolean {
    if (doc.languageId === "tarn") {
      return true;
    }
    // Fallback: match by filename in case another extension overrides
    // the language id for .tarn.yaml files.
    return /\.tarn\.ya?ml$/i.test(doc.uri.fsPath);
  }

  private toDiagnostic(
    doc: vscode.TextDocument,
    err: ValidateError,
  ): vscode.Diagnostic {
    const range = this.resolveRange(doc, err);
    const diag = new vscode.Diagnostic(
      range,
      err.message,
      vscode.DiagnosticSeverity.Error,
    );
    diag.source = "tarn";
    return diag;
  }

  private resolveRange(
    doc: vscode.TextDocument,
    err: ValidateError,
  ): vscode.Range {
    if (err.line === undefined || err.column === undefined) {
      return this.firstLineRange(doc);
    }
    // serde_yaml reports 1-based line/column. VS Code uses 0-based
    // indices but clamp the values to the document to avoid throwing.
    const line = clamp(err.line - 1, 0, Math.max(0, doc.lineCount - 1));
    const textLine = doc.lineAt(line);
    const col = clamp(err.column - 1, 0, textLine.range.end.character);
    const start = new vscode.Position(line, col);
    const end = textLine.range.end;
    if (start.isEqual(end)) {
      // Zero-width ranges don't render a squiggle; expand to the
      // whole line so the user sees something.
      return textLine.range;
    }
    return new vscode.Range(start, end);
  }

  private firstLineRange(doc: vscode.TextDocument): vscode.Range {
    if (doc.lineCount === 0) {
      return new vscode.Range(0, 0, 0, 0);
    }
    return doc.lineAt(0).range;
  }

  private fileDiagnostic(
    doc: vscode.TextDocument,
    message: string,
  ): vscode.Diagnostic {
    const diag = new vscode.Diagnostic(
      this.firstLineRange(doc),
      message,
      vscode.DiagnosticSeverity.Error,
    );
    diag.source = "tarn";
    return diag;
  }

  /** Clear the whole collection — used by tests and reload hooks. */
  clearAll(): void {
    this.collection.clear();
  }

  dispose(): void {
    for (const d of this.disposables) {
      d.dispose();
    }
    for (const cts of this.inFlight.values()) {
      cts.cancel();
      cts.dispose();
    }
    this.inFlight.clear();
    this.collection.dispose();
  }
}

function clamp(value: number, min: number, max: number): number {
  if (value < min) {
    return min;
  }
  if (value > max) {
    return max;
  }
  return value;
}

function matchFileEntry(
  files: ValidateFileResult[],
  absolutePath: string,
): ValidateFileResult | undefined {
  const exact = files.find((f) => f.file === absolutePath);
  if (exact) {
    return exact;
  }
  // Tarn may report the path relative to the cwd we passed in; match
  // by suffix so we still find it.
  return files.find(
    (f) => absolutePath.endsWith(f.file) || f.file.endsWith(absolutePath),
  );
}
