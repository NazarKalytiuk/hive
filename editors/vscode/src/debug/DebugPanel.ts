import * as vscode from "vscode";

/**
 * Shape of a `tarn/captureState` notification the LSP pushes between
 * steps. Mirrors the Rust `CaptureStateNotification` struct exported
 * from `tarn-lsp/src/debug_session.rs` — keep the field names in sync.
 *
 * Exported so the unit test can feed realistic payloads through the
 * webview message handler without pulling in `vscode-languageclient`.
 */
export interface CaptureStatePayload {
  sessionId: string;
  stepIndex: number;
  phase: "setup" | "test" | "teardown" | "finished";
  captures: Record<string, unknown>;
  lastResponse: unknown | null;
  lastStep: {
    name?: string;
    passed?: boolean;
    duration_ms?: number;
    response_status?: number | null;
    response_summary?: string | null;
    captures_set?: string[];
    assertion_failures?: Array<{
      assertion: string;
      expected: string;
      actual: string;
      message: string;
    }>;
  } | null;
  done: boolean;
}

/**
 * Command ids posted by the webview back to the extension host. These
 * map 1:1 to the `tarn.debug*` LSP commands the host forwards to the
 * server. Exported so the unit test can assert on the message payloads
 * the host will produce.
 */
export type DebugWebviewCommand =
  | "continue"
  | "stepOver"
  | "rerunStep"
  | "restart"
  | "stop";

/**
 * Serializable message the webview posts to the extension host. A
 * discriminated union keeps the handler TypeScript-safe: the host
 * rejects anything that doesn't match and logs a warning.
 */
export type DebugWebviewMessage =
  | { type: "control"; command: DebugWebviewCommand }
  | { type: "ready" };

/**
 * Pure runtime guard for `DebugWebviewMessage`. Exported so the unit
 * test can cover every accept/reject branch without instantiating the
 * full panel (which needs a real `vscode.WebviewPanel`).
 */
export function isDebugWebviewMessage(msg: unknown): msg is DebugWebviewMessage {
  if (!msg || typeof msg !== "object") return false;
  const obj = msg as Record<string, unknown>;
  if (obj.type === "ready") return true;
  if (obj.type !== "control") return false;
  return (
    obj.command === "continue" ||
    obj.command === "stepOver" ||
    obj.command === "rerunStep" ||
    obj.command === "restart" ||
    obj.command === "stop"
  );
}

/**
 * Map a webview-posted control command into the LSP `workspace/executeCommand`
 * command id. Pure and exported so the unit test can lock the mapping down
 * without a real `LanguageClient`.
 */
export function controlToLspCommand(cmd: DebugWebviewCommand): string {
  switch (cmd) {
    case "continue":
      return "tarn.debugContinue";
    case "stepOver":
      return "tarn.debugStepOver";
    case "rerunStep":
      return "tarn.debugRerunStep";
    case "restart":
      return "tarn.debugRestart";
    case "stop":
      return "tarn.debugStop";
  }
}

/**
 * Invocation shape the panel constructor accepts. Splitting the
 * dependency out from the concrete `LanguageClient` keeps the panel
 * testable: the unit test supplies an inline stub that just records
 * every outgoing request.
 */
export interface DebugCommandExecutor {
  executeCommand(command: string, args: Record<string, unknown>): Promise<void>;
}

/**
 * Singleton VS Code panel that drives a running tarn-lsp debug
 * session. The panel layout is intentionally plain HTML + vanilla JS
 * so it adds no framework dependency and the unit test can compute
 * its output deterministically.
 *
 * The panel's public surface is small on purpose:
 *
 *   - `show(sessionId)` — open the panel (or focus it) for a session.
 *   - `update(payload)` — called by the LSP notification dispatcher
 *     whenever a `tarn/captureState` notification arrives.
 *   - `dispose()` — tear down the webview.
 */
export class DebugPanel implements vscode.Disposable {
  private panel: vscode.WebviewPanel | undefined;
  private sessionId: string | undefined;
  private lastState: CaptureStatePayload | undefined;
  private readonly disposables: vscode.Disposable[] = [];

  constructor(
    private readonly extensionUri: vscode.Uri,
    private readonly executor: DebugCommandExecutor,
  ) {}

  /**
   * Open the Tarn Debug panel for a session. Reveals the existing
   * panel when one is already visible so the extension never
   * accumulates duplicates.
   */
  show(sessionId: string): void {
    this.sessionId = sessionId;
    if (this.panel) {
      this.panel.reveal(vscode.ViewColumn.Beside, true);
      this.refresh();
      return;
    }
    this.panel = vscode.window.createWebviewPanel(
      "tarn.debugPanel",
      vscode.l10n.t("Tarn Debug"),
      { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true },
      {
        enableScripts: true,
        retainContextWhenHidden: true,
        localResourceRoots: [this.extensionUri],
      },
    );
    this.panel.onDidDispose(
      () => {
        this.panel = undefined;
        this.lastState = undefined;
      },
      undefined,
      this.disposables,
    );
    this.panel.webview.onDidReceiveMessage(
      (msg) => this.handleMessage(msg),
      undefined,
      this.disposables,
    );
    this.refresh();
  }

  /**
   * Apply a `tarn/captureState` notification. Updates the cached
   * payload and repaints the panel HTML.
   */
  update(payload: CaptureStatePayload): void {
    if (!this.sessionId || payload.sessionId !== this.sessionId) return;
    this.lastState = payload;
    this.refresh();
  }

  /**
   * Explicitly dispatch a control command. Exposed so callers (tests,
   * keybindings) can drive the panel without pretending to be the
   * webview.
   */
  async dispatch(command: DebugWebviewCommand): Promise<void> {
    if (!this.sessionId) return;
    const lspCommand = controlToLspCommand(command);
    await this.executor.executeCommand(lspCommand, {
      sessionId: this.sessionId,
    });
  }

  dispose(): void {
    for (const d of this.disposables) {
      d.dispose();
    }
    this.panel?.dispose();
  }

  private async handleMessage(raw: unknown): Promise<void> {
    if (!isDebugWebviewMessage(raw)) return;
    if (raw.type === "ready") {
      this.refresh();
      return;
    }
    await this.dispatch(raw.command);
  }

  private refresh(): void {
    if (!this.panel) return;
    this.panel.webview.html = renderPanelHtml(this.lastState, this.sessionId);
  }
}

/**
 * Pure HTML renderer. Exported so the unit test can snapshot the
 * output for representative payloads. Deliberately minimal — inline
 * CSS, no framework, one `<script>` tag for the message bus.
 */
export function renderPanelHtml(
  state: CaptureStatePayload | undefined,
  sessionId: string | undefined,
): string {
  const statusLine = renderStatusLine(state, sessionId);
  const captures = renderCaptures(state);
  const lastResponse = renderLastResponse(state);
  const failures = renderFailures(state);
  const buttonsDisabled = !state || state.done ? "disabled" : "";
  return /* html */ `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta http-equiv="Content-Security-Policy"
      content="default-src 'none'; style-src 'unsafe-inline'; script-src 'unsafe-inline';" />
    <style>
      body { font-family: var(--vscode-font-family); padding: 10px; color: var(--vscode-foreground); }
      h2 { margin-top: 0; font-size: 14px; text-transform: uppercase; color: var(--vscode-descriptionForeground); }
      .status { font-size: 13px; margin-bottom: 8px; }
      .controls { display: flex; gap: 6px; margin-bottom: 12px; }
      .controls button {
        padding: 4px 10px;
        background: var(--vscode-button-background);
        color: var(--vscode-button-foreground);
        border: none;
        cursor: pointer;
      }
      .controls button[disabled] { opacity: 0.4; cursor: not-allowed; }
      .section { margin-top: 12px; }
      .captures-table { width: 100%; border-collapse: collapse; font-family: var(--vscode-editor-font-family); }
      .captures-table th, .captures-table td { text-align: left; padding: 3px 6px; border-bottom: 1px solid var(--vscode-panel-border); }
      pre { background: var(--vscode-textBlockQuote-background); padding: 6px; overflow-x: auto; font-family: var(--vscode-editor-font-family); }
      .failures li { color: var(--vscode-errorForeground); }
    </style>
  </head>
  <body>
    <div class="status">${statusLine}</div>
    <div class="controls">
      <button id="btn-continue" ${buttonsDisabled}>Continue</button>
      <button id="btn-step" ${buttonsDisabled}>Step Over</button>
      <button id="btn-rerun" ${buttonsDisabled}>Rerun Step</button>
      <button id="btn-restart" ${buttonsDisabled}>Restart</button>
      <button id="btn-stop" ${buttonsDisabled}>Stop</button>
    </div>
    <div class="section">
      <h2>Captures</h2>
      ${captures}
    </div>
    <div class="section">
      <h2>Last Response</h2>
      ${lastResponse}
    </div>
    ${failures}
    <script>
      const vscode = acquireVsCodeApi();
      function post(cmd) { vscode.postMessage({ type: 'control', command: cmd }); }
      const map = {
        'btn-continue': 'continue',
        'btn-step': 'stepOver',
        'btn-rerun': 'rerunStep',
        'btn-restart': 'restart',
        'btn-stop': 'stop',
      };
      for (const [id, cmd] of Object.entries(map)) {
        const el = document.getElementById(id);
        if (el) el.addEventListener('click', () => post(cmd));
      }
      vscode.postMessage({ type: 'ready' });
    </script>
  </body>
</html>`;
}

function renderStatusLine(
  state: CaptureStatePayload | undefined,
  sessionId: string | undefined,
): string {
  if (!state) {
    return `<strong>Session:</strong> ${escapeHtml(sessionId ?? "(starting)")} — waiting for first step…`;
  }
  const phase = state.phase;
  if (state.done) {
    return `<strong>Session:</strong> ${escapeHtml(state.sessionId)} — <em>finished</em>.`;
  }
  const stepName = state.lastStep?.name ?? "(pending)";
  const passed = state.lastStep?.passed;
  const passedBadge =
    passed === true
      ? "<span style=\"color: var(--vscode-testing-iconPassed);\">PASS</span>"
      : passed === false
        ? "<span style=\"color: var(--vscode-testing-iconFailed);\">FAIL</span>"
        : "";
  return (
    `<strong>Session:</strong> ${escapeHtml(state.sessionId)} — ` +
    `<strong>phase:</strong> ${phase} — ` +
    `<strong>step ${state.stepIndex}:</strong> ${escapeHtml(stepName)} ${passedBadge}`
  );
}

function renderCaptures(state: CaptureStatePayload | undefined): string {
  const captures = state?.captures ?? {};
  const entries = Object.entries(captures);
  if (entries.length === 0) {
    return `<p><em>No captures recorded yet.</em></p>`;
  }
  const rows = entries
    .map(
      ([key, value]) =>
        `<tr><td>${escapeHtml(key)}</td><td>${escapeHtml(JSON.stringify(value))}</td></tr>`,
    )
    .join("");
  return `<table class="captures-table"><thead><tr><th>name</th><th>value</th></tr></thead><tbody>${rows}</tbody></table>`;
}

function renderLastResponse(state: CaptureStatePayload | undefined): string {
  if (!state?.lastResponse) {
    return `<p><em>No response captured for this step.</em></p>`;
  }
  const serialised = JSON.stringify(state.lastResponse, null, 2);
  return `<pre>${escapeHtml(serialised)}</pre>`;
}

function renderFailures(state: CaptureStatePayload | undefined): string {
  const failures = state?.lastStep?.assertion_failures ?? [];
  if (failures.length === 0) return "";
  const items = failures
    .map(
      (f) =>
        `<li><strong>${escapeHtml(f.assertion)}</strong>: ${escapeHtml(f.message)}</li>`,
    )
    .join("");
  return `<div class="section failures"><h2>Assertion failures</h2><ul>${items}</ul></div>`;
}

/**
 * Escape the short list of characters that can break out of HTML
 * attributes or element content. Exported so the unit test can
 * verify the escaping policy without spinning up the panel.
 */
export function escapeHtml(raw: string): string {
  return raw
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#039;");
}
