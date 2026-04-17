import * as vscode from "vscode";
import {
  DebugPanel,
  type CaptureStatePayload,
  type DebugCommandExecutor,
} from "./DebugPanel";

/**
 * Adapter that forwards VS Code `workspace/executeCommand` calls to the
 * running `tarn-lsp` client. Split out as a small interface so the unit
 * test can stub it without spinning up a real language client.
 */
export interface LanguageClientLike {
  sendRequest(
    method: string,
    params: unknown,
  ): Promise<unknown>;
  onNotification(
    method: string,
    handler: (params: unknown) => void,
  ): vscode.Disposable;
}

/**
 * Build a `DebugCommandExecutor` that forwards every control command to
 * the LSP server via `workspace/executeCommand`.
 */
export function lspExecutor(client: LanguageClientLike): DebugCommandExecutor {
  return {
    async executeCommand(command: string, args: Record<string, unknown>): Promise<void> {
      await client.sendRequest("workspace/executeCommand", {
        command,
        arguments: [args],
      });
    },
  };
}

/**
 * Register the `Tarn: Debug Test` command and the `tarn/captureState`
 * notification listener. The returned `Disposable` tears down both so
 * `context.subscriptions.push(registerDebugCommands(...))` is enough.
 *
 * The command expects two arguments: the absolute file path and the
 * enclosing test name. This mirrors the VS Code test controller's code
 * lens signature so the "Tarn: Debug Test" lens can fire directly.
 */
export function registerDebugCommands(
  context: vscode.ExtensionContext,
  client: LanguageClientLike,
): vscode.Disposable {
  const disposables: vscode.Disposable[] = [];
  const panel = new DebugPanel(context.extensionUri, lspExecutor(client));
  disposables.push(panel);

  disposables.push(
    vscode.commands.registerCommand(
      "tarn.debugTest",
      async (file?: string, test?: string, env?: string) => {
        if (!file || !test) {
          vscode.window.showWarningMessage(
            vscode.l10n.t("Tarn: Debug Test requires a file and test name."),
          );
          return;
        }
        const raw = await client.sendRequest("workspace/executeCommand", {
          command: "tarn.debugTest",
          arguments: [{ file, test, env }],
        });
        const sessionId = extractSessionId(raw);
        if (!sessionId) {
          vscode.window.showErrorMessage(
            vscode.l10n.t("Tarn: Debug Test did not return a session id."),
          );
          return;
        }
        panel.show(sessionId);
      },
    ),
  );

  disposables.push(
    vscode.commands.registerCommand(
      "tarn.diffLastPassing",
      async (file?: string, test?: string, step?: number) => {
        if (!file || !test || typeof step !== "number") {
          vscode.window.showWarningMessage(
            vscode.l10n.t(
              "Tarn: Diff Last Passing needs file, test, and step index.",
            ),
          );
          return;
        }
        await runDiffLastPassingCommand(client, file, test, step);
      },
    ),
  );

  disposables.push(
    client.onNotification("tarn/captureState", (params: unknown) => {
      if (isCaptureStatePayload(params)) {
        panel.update(params);
      }
    }),
  );

  return vscode.Disposable.from(...disposables);
}

/**
 * Extract the session id from a `tarn.debugTest` response. The LSP wraps
 * the payload as `{ schema_version: 1, data: { sessionId } }` — the
 * helper handles both that shape and a plain `{ sessionId }` fallback
 * so future schema bumps do not break the extension.
 */
export function extractSessionId(raw: unknown): string | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const outer = raw as { data?: unknown; sessionId?: unknown };
  if (typeof outer.sessionId === "string") return outer.sessionId;
  if (outer.data && typeof outer.data === "object") {
    const inner = outer.data as { sessionId?: unknown };
    if (typeof inner.sessionId === "string") return inner.sessionId;
  }
  return undefined;
}

/**
 * Runtime guard mirroring the Rust `CaptureStateNotification`. Exported
 * so the unit test can assert the guard accepts realistic payloads and
 * rejects malformed ones without instantiating the panel.
 */
export function isCaptureStatePayload(raw: unknown): raw is CaptureStatePayload {
  if (!raw || typeof raw !== "object") return false;
  const obj = raw as Record<string, unknown>;
  if (typeof obj.sessionId !== "string") return false;
  if (typeof obj.stepIndex !== "number") return false;
  if (typeof obj.phase !== "string") return false;
  if (typeof obj.done !== "boolean") return false;
  return true;
}

/**
 * Invoke `tarn.diffLastPassing` and open a VS Code diff tab showing the
 * before/after response bodies side-by-side.
 */
export async function runDiffLastPassingCommand(
  client: LanguageClientLike,
  file: string,
  test: string,
  step: number,
): Promise<void> {
  const raw = await client.sendRequest("workspace/executeCommand", {
    command: "tarn.diffLastPassing",
    arguments: [{ file, test, step }],
  });
  const data = unwrapEnvelope(raw);
  if (!data) {
    vscode.window.showErrorMessage(
      vscode.l10n.t("Tarn: Diff Last Passing returned an unexpected payload."),
    );
    return;
  }
  if (typeof data === "object" && (data as { error?: unknown }).error === "no_baseline") {
    vscode.window.showInformationMessage(
      vscode.l10n.t(
        "Tarn: no passing run recorded for this step yet — run the test at least once.",
      ),
    );
    return;
  }
  // When there is a full diff payload we present a markdown summary in
  // a new editor. A richer side-by-side diff (using `vscode.diff`) is
  // left to a future iteration once the fixture layout stabilises.
  const summary = renderDiffSummary(data);
  const doc = await vscode.workspace.openTextDocument({
    language: "markdown",
    content: summary,
  });
  await vscode.window.showTextDocument(doc, { preview: false });
}

/**
 * Unwrap the `{ schema_version, data }` envelope. Exported so the unit
 * test can exercise the fallback paths.
 */
export function unwrapEnvelope(raw: unknown): unknown | undefined {
  if (!raw || typeof raw !== "object") return undefined;
  const outer = raw as { data?: unknown };
  if ("data" in outer) return outer.data;
  return raw;
}

/**
 * Pure function that turns a `tarn.diffLastPassing` payload into a
 * markdown summary. Exported for unit testing — assertions pin the
 * output shape so future renames break the test rather than silently
 * regress the UX.
 */
export function renderDiffSummary(data: unknown): string {
  if (!data || typeof data !== "object") {
    return "# Tarn diff\n\n_No diff data returned._\n";
  }
  const obj = data as Record<string, unknown>;
  const lines: string[] = ["# Tarn diff vs last passing", ""];
  if (obj.status && typeof obj.status === "object") {
    const s = obj.status as { was?: unknown; now?: unknown };
    lines.push(`**Status:** \`${JSON.stringify(s.was)}\` → \`${JSON.stringify(s.now)}\``, "");
  }
  const added = asStringArray(obj.headers_added);
  if (added.length > 0) {
    lines.push("**Headers added:**", ...added.map((h) => `- \`${h}\``), "");
  }
  const removed = asStringArray(obj.headers_removed);
  if (removed.length > 0) {
    lines.push("**Headers removed:**", ...removed.map((h) => `- \`${h}\``), "");
  }
  const changedHeaders = Array.isArray(obj.headers_changed) ? obj.headers_changed : [];
  if (changedHeaders.length > 0) {
    lines.push("**Headers changed:**");
    for (const raw of changedHeaders) {
      if (!raw || typeof raw !== "object") continue;
      const h = raw as { name?: unknown; was?: unknown; now?: unknown };
      lines.push(
        `- \`${String(h.name ?? "?")}\`: \`${JSON.stringify(h.was)}\` → \`${JSON.stringify(h.now)}\``,
      );
    }
    lines.push("");
  }
  const bodyAdded = asStringArray(obj.body_keys_added);
  if (bodyAdded.length > 0) {
    lines.push("**Body keys added:**", ...bodyAdded.map((k) => `- \`${k}\``), "");
  }
  const bodyRemoved = asStringArray(obj.body_keys_removed);
  if (bodyRemoved.length > 0) {
    lines.push("**Body keys removed:**", ...bodyRemoved.map((k) => `- \`${k}\``), "");
  }
  const bodyChanged = Array.isArray(obj.body_values_changed) ? obj.body_values_changed : [];
  if (bodyChanged.length > 0) {
    lines.push("**Body values changed:**");
    for (const raw of bodyChanged) {
      if (!raw || typeof raw !== "object") continue;
      const c = raw as { path?: unknown; was?: unknown; now?: unknown };
      lines.push(
        `- \`${String(c.path ?? "?")}\`: \`${JSON.stringify(c.was)}\` → \`${JSON.stringify(c.now)}\``,
      );
    }
    lines.push("");
  }
  if (lines.length <= 2) {
    lines.push("_Responses match — nothing to report._");
  }
  return lines.join("\n");
}

function asStringArray(raw: unknown): string[] {
  if (!Array.isArray(raw)) return [];
  return raw.filter((v): v is string => typeof v === "string");
}
