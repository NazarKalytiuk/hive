import * as vscode from "vscode";
import type { StepSnapshot } from "../testing/LastRunCache";
import type { StepResult } from "../util/schemaGuards";

const PANEL_ID = "tarn.requestResponseInspector";
const MAX_BODY_BYTES = 10 * 1024;

/**
 * Singleton webview manager for the Request/Response Inspector. One
 * panel per window; subsequent `show` calls reuse the existing panel.
 *
 * Renders three tabs in plain HTML/JS with no framework dependency:
 * Request (method + URL + headers + body), Response (status + headers
 * + body), and Assertions (per-assertion pass/fail with diff).
 *
 * Body rendering auto-detects JSON (pretty-prints), falls back to
 * plain text for anything else, and truncates at 10 KB with an
 * "Open full in new editor" action so the panel stays snappy.
 */
export class RequestResponsePanel implements vscode.Disposable {
  private panel: vscode.WebviewPanel | undefined;
  private current: StepSnapshot | undefined;
  private readonly disposables: vscode.Disposable[] = [];

  // The extensionUri is intentionally accepted for future local
  // resource loading (e.g., a bundled highlighter) even though v1
  // ships without any local resources.
  constructor(_extensionUri: vscode.Uri) {}

  show(snapshot: StepSnapshot): void {
    this.current = snapshot;
    if (this.panel) {
      this.panel.reveal(vscode.ViewColumn.Beside, true);
      this.refresh();
      return;
    }
    this.panel = vscode.window.createWebviewPanel(
      PANEL_ID,
      "Tarn: Step Details",
      { viewColumn: vscode.ViewColumn.Beside, preserveFocus: true },
      { enableScripts: true, retainContextWhenHidden: true },
    );
    this.panel.onDidDispose(
      () => {
        this.panel = undefined;
        this.current = undefined;
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

  private async handleMessage(msg: unknown): Promise<void> {
    if (!isMessage(msg)) return;
    if (msg.type === "openFullBody" && this.current) {
      const body = msg.body === "request"
        ? this.current.step.request?.body
        : this.current.step.response?.body;
      if (body === undefined) return;
      const content = stringifyBody(body);
      const doc = await vscode.workspace.openTextDocument({
        language: detectLanguage(content),
        content,
      });
      await vscode.window.showTextDocument(doc, { preview: false });
    }
  }

  private refresh(): void {
    if (!this.panel || !this.current) return;
    this.panel.title = `Tarn: ${this.current.stepName}`;
    this.panel.webview.html = renderHtml(this.current);
  }

  dispose(): void {
    for (const d of this.disposables) {
      d.dispose();
    }
    this.panel?.dispose();
  }
}

interface OpenFullBodyMessage {
  type: "openFullBody";
  body: "request" | "response";
}

type InspectorMessage = OpenFullBodyMessage;

function isMessage(msg: unknown): msg is InspectorMessage {
  if (!msg || typeof msg !== "object") return false;
  const obj = msg as Record<string, unknown>;
  return obj.type === "openFullBody" && (obj.body === "request" || obj.body === "response");
}

/** Serialize a body value for display. Objects are pretty-printed JSON. */
export function stringifyBody(body: unknown): string {
  if (body === undefined || body === null) return "";
  if (typeof body === "string") return body;
  try {
    return JSON.stringify(body, null, 2);
  } catch {
    return String(body);
  }
}

/** Best-effort language detection for the "Open full in editor" doc. */
export function detectLanguage(content: string): string {
  const trimmed = content.trim();
  if (!trimmed) return "plaintext";
  if (
    (trimmed.startsWith("{") && trimmed.endsWith("}")) ||
    (trimmed.startsWith("[") && trimmed.endsWith("]"))
  ) {
    return "json";
  }
  if (trimmed.startsWith("<?xml") || /^<[a-zA-Z!]/.test(trimmed)) {
    return "xml";
  }
  return "plaintext";
}

/** Truncate a body to `MAX_BODY_BYTES`, returning the display text and
 * a flag indicating whether the body was trimmed. */
export function truncateBody(content: string): {
  display: string;
  truncated: boolean;
} {
  if (content.length <= MAX_BODY_BYTES) {
    return { display: content, truncated: false };
  }
  return {
    display: content.slice(0, MAX_BODY_BYTES),
    truncated: true,
  };
}

function renderHtml(snapshot: StepSnapshot): string {
  const step = snapshot.step;
  const nonce = String(Date.now()) + Math.random().toString(36).slice(2);

  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8" />
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src 'unsafe-inline'; script-src 'nonce-${nonce}';" />
<title>Tarn: Step Details</title>
<style>
  body {
    font-family: var(--vscode-font-family);
    font-size: var(--vscode-font-size);
    color: var(--vscode-foreground);
    background: var(--vscode-editor-background);
    padding: 0 16px 24px;
  }
  h1 {
    font-size: 1.2em;
    margin: 16px 0 4px;
  }
  .meta {
    color: var(--vscode-descriptionForeground);
    font-size: 0.9em;
    margin-bottom: 12px;
  }
  .status {
    display: inline-block;
    padding: 2px 8px;
    border-radius: 3px;
    font-weight: 600;
    margin-left: 8px;
  }
  .status.passed { background: var(--vscode-testing-iconPassed); color: var(--vscode-editor-background); }
  .status.failed { background: var(--vscode-testing-iconFailed); color: var(--vscode-editor-background); }
  .tabs {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--vscode-panel-border);
    margin-bottom: 12px;
  }
  .tab-button {
    background: transparent;
    border: none;
    color: var(--vscode-foreground);
    padding: 8px 16px;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    font-family: inherit;
    font-size: inherit;
  }
  .tab-button.active {
    border-bottom-color: var(--vscode-focusBorder);
    color: var(--vscode-textLink-activeForeground);
  }
  .tab-panel { display: none; }
  .tab-panel.active { display: block; }
  table.headers {
    width: 100%;
    border-collapse: collapse;
    margin: 8px 0 16px;
  }
  table.headers td {
    padding: 4px 8px;
    border-bottom: 1px solid var(--vscode-panel-border);
    vertical-align: top;
    font-family: var(--vscode-editor-font-family);
    font-size: var(--vscode-editor-font-size);
  }
  table.headers td.name {
    color: var(--vscode-symbolIcon-fieldForeground);
    width: 30%;
    font-weight: 600;
  }
  pre {
    background: var(--vscode-textCodeBlock-background);
    padding: 12px;
    border-radius: 4px;
    overflow: auto;
    max-height: 520px;
    font-family: var(--vscode-editor-font-family);
    font-size: var(--vscode-editor-font-size);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .truncation {
    background: var(--vscode-inputValidation-warningBackground);
    border: 1px solid var(--vscode-inputValidation-warningBorder);
    color: var(--vscode-inputValidation-warningForeground);
    padding: 6px 10px;
    margin: 8px 0;
    border-radius: 4px;
    font-size: 0.9em;
  }
  .truncation button {
    margin-left: 8px;
    background: var(--vscode-button-background);
    color: var(--vscode-button-foreground);
    border: none;
    padding: 4px 10px;
    border-radius: 2px;
    cursor: pointer;
    font-family: inherit;
    font-size: inherit;
  }
  .assertion-row {
    border-left: 3px solid var(--vscode-panel-border);
    padding: 8px 12px;
    margin-bottom: 12px;
    background: var(--vscode-textCodeBlock-background);
    border-radius: 0 4px 4px 0;
  }
  .assertion-row.failed { border-left-color: var(--vscode-testing-iconFailed); }
  .assertion-row.passed { border-left-color: var(--vscode-testing-iconPassed); }
  .assertion-label {
    font-weight: 600;
    margin-bottom: 4px;
  }
  .assertion-detail {
    color: var(--vscode-descriptionForeground);
    font-family: var(--vscode-editor-font-family);
    font-size: 0.9em;
  }
  .diff {
    margin-top: 8px;
    padding: 8px;
    background: var(--vscode-editor-background);
    border-radius: 3px;
    font-family: var(--vscode-editor-font-family);
    font-size: 0.9em;
    white-space: pre;
    overflow: auto;
  }
  .diff .add { color: var(--vscode-diffEditor-insertedTextBackground, #99c794); }
  .diff .del { color: var(--vscode-diffEditor-removedTextBackground, #ec5f67); }
  .empty { color: var(--vscode-descriptionForeground); font-style: italic; }
</style>
</head>
<body>
  <h1>${escapeHtml(step.name)} <span class="status ${step.status.toLowerCase()}">${step.status}</span></h1>
  <div class="meta">
    ${escapeHtml(snapshot.fileName)} · ${snapshot.phase} · ${step.duration_ms}ms
    ${snapshot.phase === "test" ? `· test <code>${escapeHtml(snapshot.key.test)}</code>` : ""}
  </div>

  <div class="tabs" role="tablist">
    <button class="tab-button active" data-tab="request" role="tab">Request</button>
    <button class="tab-button" data-tab="response" role="tab">Response</button>
    <button class="tab-button" data-tab="assertions" role="tab">Assertions</button>
  </div>

  <div class="tab-panel active" id="tab-request">
    ${renderRequestPanel(step)}
  </div>

  <div class="tab-panel" id="tab-response">
    ${renderResponsePanel(step)}
  </div>

  <div class="tab-panel" id="tab-assertions">
    ${renderAssertionsPanel(step)}
  </div>

<script nonce="${nonce}">
  const vscode = acquireVsCodeApi();
  const buttons = document.querySelectorAll('.tab-button');
  const panels = document.querySelectorAll('.tab-panel');
  buttons.forEach((btn) => {
    btn.addEventListener('click', () => {
      const target = btn.getAttribute('data-tab');
      buttons.forEach((b) => b.classList.toggle('active', b === btn));
      panels.forEach((p) => p.classList.toggle('active', p.id === 'tab-' + target));
    });
  });
  document.querySelectorAll('[data-open-full]').forEach((el) => {
    el.addEventListener('click', () => {
      vscode.postMessage({ type: 'openFullBody', body: el.getAttribute('data-open-full') });
    });
  });
</script>
</body>
</html>`;
}

function renderRequestPanel(step: StepResult): string {
  if (!step.request) {
    return `<p class="empty">No request captured for this step.</p>`;
  }
  const req = step.request;
  const headersHtml = renderHeadersTable(req.headers);
  const { display, truncated } = truncateBody(stringifyBody(req.body));
  const bodyHtml = req.body !== undefined
    ? `<h3>Body</h3>
       ${truncated ? renderTruncationBanner("request") : ""}
       <pre>${escapeHtml(display)}</pre>`
    : "";
  return `
    <table class="headers">
      <tr><td class="name">Method</td><td>${escapeHtml(req.method)}</td></tr>
      <tr><td class="name">URL</td><td>${escapeHtml(req.url)}</td></tr>
    </table>
    <h3>Headers</h3>
    ${headersHtml}
    ${bodyHtml}
  `;
}

function renderResponsePanel(step: StepResult): string {
  if (!step.response) {
    if (step.response_status !== undefined) {
      return `<table class="headers"><tr><td class="name">Status</td><td>${step.response_status}</td></tr></table>
              <p class="empty">Full response not captured (step passed). Response bodies are only included for failed steps.</p>`;
    }
    return `<p class="empty">No response captured.</p>`;
  }
  const res = step.response;
  const headersHtml = renderHeadersTable(res.headers);
  const { display, truncated } = truncateBody(stringifyBody(res.body));
  const bodyHtml = res.body !== undefined
    ? `<h3>Body</h3>
       ${truncated ? renderTruncationBanner("response") : ""}
       <pre>${escapeHtml(display)}</pre>`
    : "";
  return `
    <table class="headers">
      <tr><td class="name">Status</td><td>${res.status}</td></tr>
    </table>
    <h3>Headers</h3>
    ${headersHtml}
    ${bodyHtml}
  `;
}

function renderAssertionsPanel(step: StepResult): string {
  const assertions = step.assertions;
  if (!assertions) {
    return `<p class="empty">No assertions recorded for this step.</p>`;
  }

  const rows: string[] = [];

  const allDetails = assertions.details ?? [];
  const failuresOnly = assertions.failures ?? [];

  // Prefer `details` (which includes passed+failed) if it's available;
  // otherwise fall back to failures only.
  const source = allDetails.length > 0 ? allDetails : failuresOnly;

  if (source.length === 0) {
    rows.push(`<p class="empty">No assertion details recorded.</p>`);
  } else {
    for (const a of source) {
      const status = a.passed ? "passed" : "failed";
      const statusLabel = a.passed ? "PASSED" : "FAILED";
      const expectedHtml = a.expected !== undefined
        ? `<div class="assertion-detail">Expected: <code>${escapeHtml(String(a.expected))}</code></div>`
        : "";
      const actualHtml = a.actual !== undefined
        ? `<div class="assertion-detail">Actual: <code>${escapeHtml(String(a.actual))}</code></div>`
        : "";
      const messageHtml = a.message
        ? `<div class="assertion-detail">${escapeHtml(a.message)}</div>`
        : "";
      const diffHtml = a.diff
        ? `<pre class="diff">${renderDiff(a.diff)}</pre>`
        : "";
      rows.push(`
        <div class="assertion-row ${status}">
          <div class="assertion-label">${escapeHtml(a.assertion)} <span class="status ${status}">${statusLabel}</span></div>
          ${expectedHtml}
          ${actualHtml}
          ${messageHtml}
          ${diffHtml}
        </div>
      `);
    }
  }

  return `
    <p class="meta">
      ${assertions.passed} passed · ${assertions.failed} failed · ${assertions.total} total
    </p>
    ${rows.join("")}
  `;
}

function renderHeadersTable(headers: Record<string, string> | undefined): string {
  if (!headers || Object.keys(headers).length === 0) {
    return `<p class="empty">No headers.</p>`;
  }
  const rows = Object.entries(headers)
    .map(
      ([name, value]) =>
        `<tr><td class="name">${escapeHtml(name)}</td><td>${escapeHtml(value)}</td></tr>`,
    )
    .join("");
  return `<table class="headers">${rows}</table>`;
}

function renderTruncationBanner(body: "request" | "response"): string {
  return `
    <div class="truncation">
      Body truncated to 10 KB.
      <button data-open-full="${body}">Open full in new editor</button>
    </div>
  `;
}

function renderDiff(diff: string): string {
  return diff
    .split("\n")
    .map((line) => {
      const safe = escapeHtml(line);
      if (line.startsWith("+")) return `<span class="add">${safe}</span>`;
      if (line.startsWith("-")) return `<span class="del">${safe}</span>`;
      return safe;
    })
    .join("\n");
}

function escapeHtml(s: unknown): string {
  return String(s)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}
