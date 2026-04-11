import * as path from "path";
import * as vscode from "vscode";
import { parse as parseYaml } from "yaml";
import type { Report } from "../util/schemaGuards";

/**
 * Tree data provider for captured variables from the most recent run.
 *
 * Populated via `loadFromReport` after every completed run. Walks
 * `files[].tests[].captures` from the JSON report and groups values
 * under `file > test > key` nodes. Nested JSON expands into child
 * nodes so users can drill into object/array captures.
 *
 * Redaction: capture keys listed under `redaction.captures` in
 * `tarn.config.yaml` render as `***`. The "Hide all capture values"
 * toggle masks every value regardless of the redaction list — useful
 * for demos and screen sharing.
 */
export class CapturesInspector
  implements vscode.TreeDataProvider<CaptureNode>, vscode.Disposable
{
  private readonly emitter = new vscode.EventEmitter<CaptureNode | undefined>();
  readonly onDidChangeTreeData = this.emitter.event;

  private readonly disposables: vscode.Disposable[] = [];
  private files: FileCaptures[] = [];
  private redactedKeys = new Set<string>();
  private hideAllValues = false;

  constructor() {
    const folder = vscode.workspace.workspaceFolders?.[0];
    if (folder) {
      const watcher = vscode.workspace.createFileSystemWatcher(
        new vscode.RelativePattern(folder, "tarn.config.yaml"),
      );
      watcher.onDidCreate(() => void this.reloadRedaction());
      watcher.onDidChange(() => void this.reloadRedaction());
      watcher.onDidDelete(() => void this.reloadRedaction());
      this.disposables.push(watcher);
    }
    void this.reloadRedaction();
  }

  /** Replace the view state with the captures pulled from a new report. */
  loadFromReport(report: Report): void {
    const files: FileCaptures[] = [];
    for (const file of report.files) {
      const tests: TestCaptures[] = [];
      for (const test of file.tests) {
        const captures = test.captures;
        if (!captures || Object.keys(captures).length === 0) {
          continue;
        }
        tests.push({
          name: test.name,
          description: test.description ?? undefined,
          captures,
        });
      }
      if (tests.length > 0) {
        files.push({
          file: file.file,
          displayName: file.name || path.basename(file.file),
          tests,
        });
      }
    }
    this.files = files;
    this.emitter.fire(undefined);
  }

  /** Total number of captured values across every test in the last run. */
  totalCaptureCount(): number {
    let n = 0;
    for (const file of this.files) {
      for (const test of file.tests) {
        n += Object.keys(test.captures).length;
      }
    }
    return n;
  }

  /** Toggle the global "hide all values" mode. */
  toggleHideAllValues(): void {
    this.hideAllValues = !this.hideAllValues;
    this.emitter.fire(undefined);
  }

  isHidingAllValues(): boolean {
    return this.hideAllValues;
  }

  /** Exposed for tests — returns whether a key would be redacted. */
  isKeyRedacted(key: string): boolean {
    return this.redactedKeys.has(key);
  }

  refresh(): void {
    this.emitter.fire(undefined);
  }

  clear(): void {
    this.files = [];
    this.emitter.fire(undefined);
  }

  getTreeItem(element: CaptureNode): vscode.TreeItem {
    switch (element.kind) {
      case "file": {
        const item = new vscode.TreeItem(
          element.displayName,
          vscode.TreeItemCollapsibleState.Expanded,
        );
        const testCount = element.tests.length;
        item.description =
          testCount === 1
            ? vscode.l10n.t("{0} test", testCount)
            : vscode.l10n.t("{0} tests", testCount);
        item.tooltip = element.file;
        item.resourceUri = vscode.Uri.file(element.file);
        item.iconPath = new vscode.ThemeIcon("file-code");
        item.contextValue = "tarnCapturesFile";
        return item;
      }
      case "test": {
        const item = new vscode.TreeItem(
          element.name,
          vscode.TreeItemCollapsibleState.Expanded,
        );
        const count = Object.keys(element.captures).length;
        item.description =
          count === 1
            ? vscode.l10n.t("{0} capture", count)
            : vscode.l10n.t("{0} captures", count);
        if (element.description) {
          item.tooltip = element.description;
        }
        item.iconPath = new vscode.ThemeIcon("beaker");
        item.contextValue = "tarnCapturesTest";
        return item;
      }
      case "value": {
        return this.renderValueNode(element);
      }
      case "placeholder": {
        const item = new vscode.TreeItem(
          element.message,
          vscode.TreeItemCollapsibleState.None,
        );
        item.contextValue = "tarnCapturesPlaceholder";
        return item;
      }
    }
  }

  getChildren(element?: CaptureNode): vscode.ProviderResult<CaptureNode[]> {
    if (!element) {
      if (this.files.length === 0) {
        return [
          {
            kind: "placeholder",
            message: vscode.l10n.t(
              "No captures from the last run. Run a test that uses `capture:` to populate this view.",
            ),
          },
        ];
      }
      return this.files.map<CaptureNode>((file) => ({
        kind: "file",
        file: file.file,
        displayName: file.displayName,
        tests: file.tests,
      }));
    }

    if (element.kind === "file") {
      return element.tests.map<CaptureNode>((test) => ({
        kind: "test",
        name: test.name,
        description: test.description,
        captures: test.captures,
      }));
    }

    if (element.kind === "test") {
      return Object.entries(element.captures).map<CaptureNode>(
        ([key, value]) => ({
          kind: "value",
          key,
          value,
          path: [key],
          topKey: key,
        }),
      );
    }

    if (element.kind === "value") {
      return this.childNodesFor(element);
    }

    return [];
  }

  private childNodesFor(node: CaptureValueNode): CaptureNode[] {
    // If the top-level capture key is on the redaction list or the
    // user has hidden all values, do not expose nested children — we
    // mustn't let users drill past the `***` mask.
    if (this.isNodeRedacted(node)) {
      return [];
    }
    const value = node.value;
    if (Array.isArray(value)) {
      return value.map<CaptureNode>((child, index) => ({
        kind: "value",
        key: String(index),
        value: child,
        path: [...node.path, String(index)],
        topKey: node.topKey,
      }));
    }
    if (value !== null && typeof value === "object") {
      return Object.entries(value as Record<string, unknown>).map<CaptureNode>(
        ([childKey, childValue]) => ({
          kind: "value",
          key: childKey,
          value: childValue,
          path: [...node.path, childKey],
          topKey: node.topKey,
        }),
      );
    }
    return [];
  }

  private renderValueNode(element: CaptureValueNode): vscode.TreeItem {
    const value = element.value;
    const hasChildren = !this.isNodeRedacted(element) && isExpandable(value);
    const collapsibleState = hasChildren
      ? vscode.TreeItemCollapsibleState.Collapsed
      : vscode.TreeItemCollapsibleState.None;

    const display = this.renderValueText(element);
    const item = new vscode.TreeItem(element.key, collapsibleState);
    item.description = display.description;
    item.tooltip = new vscode.MarkdownString(
      `**${element.path.join(".")}** = \`${escapeMarkdown(display.full)}\``,
    );
    item.iconPath = new vscode.ThemeIcon(display.icon);
    item.contextValue = "tarnCapturesValue";
    // Clicking copies the rendered (redaction-aware) value so demos
    // stay safe even when the user interacts with the tree.
    item.command = {
      command: "tarn.copyCaptureValue",
      title: vscode.l10n.t("Copy Capture Value"),
      arguments: [display.clipboard, element.path.join(".")],
    };
    return item;
  }

  private renderValueText(element: CaptureValueNode): RenderedValue {
    if (this.isNodeRedacted(element)) {
      return {
        description: "***",
        full: "***",
        clipboard: "***",
        icon: "lock",
      };
    }
    return renderRawValue(element.value);
  }

  private isNodeRedacted(element: CaptureValueNode): boolean {
    if (this.hideAllValues) {
      return true;
    }
    return this.redactedKeys.has(element.topKey);
  }

  private async reloadRedaction(): Promise<void> {
    const folder = vscode.workspace.workspaceFolders?.[0];
    if (!folder) {
      this.redactedKeys = new Set();
      return;
    }
    const configUri = vscode.Uri.joinPath(folder.uri, "tarn.config.yaml");
    try {
      const raw = await vscode.workspace.fs.readFile(configUri);
      const parsed = parseYaml(new TextDecoder().decode(raw)) as unknown;
      const keys = extractRedactionCaptures(parsed);
      this.redactedKeys = new Set(keys);
    } catch {
      // Missing or unparseable config is a soft error — the view
      // still works without redaction.
      this.redactedKeys = new Set();
    }
    this.emitter.fire(undefined);
  }

  dispose(): void {
    for (const d of this.disposables) {
      d.dispose();
    }
    this.emitter.dispose();
  }
}

interface FileCaptures {
  file: string;
  displayName: string;
  tests: TestCaptures[];
}

interface TestCaptures {
  name: string;
  description: string | undefined;
  captures: Record<string, unknown>;
}

export type CaptureNode =
  | ({ kind: "file" } & FileCaptures)
  | ({ kind: "test" } & TestCaptures)
  | CaptureValueNode
  | { kind: "placeholder"; message: string };

export interface CaptureValueNode {
  kind: "value";
  key: string;
  value: unknown;
  path: string[];
  /** The top-level capture key this node belongs to. Used for redaction. */
  topKey: string;
}

export interface RenderedValue {
  /** Short form used in TreeItem.description. */
  description: string;
  /** Full form used for tooltip. */
  full: string;
  /** Exact string copied to the clipboard on click. */
  clipboard: string;
  /** Codicon name that hints at the value type. */
  icon: string;
}

export function renderRawValue(value: unknown): RenderedValue {
  if (value === null) {
    return { description: "null", full: "null", clipboard: "null", icon: "circle-slash" };
  }
  if (typeof value === "string") {
    const full = JSON.stringify(value);
    return {
      description: truncate(full, 80),
      full,
      clipboard: value,
      icon: "symbol-string",
    };
  }
  if (typeof value === "number") {
    return {
      description: String(value),
      full: String(value),
      clipboard: String(value),
      icon: "symbol-number",
    };
  }
  if (typeof value === "boolean") {
    return {
      description: String(value),
      full: String(value),
      clipboard: String(value),
      icon: "symbol-boolean",
    };
  }
  if (Array.isArray(value)) {
    const full = JSON.stringify(value);
    return {
      description: `[${value.length}]`,
      full,
      clipboard: full,
      icon: "symbol-array",
    };
  }
  if (typeof value === "object") {
    const keys = Object.keys(value as Record<string, unknown>);
    const full = JSON.stringify(value);
    return {
      description: `{${keys.length}}`,
      full,
      clipboard: full,
      icon: "symbol-object",
    };
  }
  // Fallback for exotic types (symbols, functions) — shouldn't appear in JSON.
  const full = String(value);
  return { description: full, full, clipboard: full, icon: "symbol-misc" };
}

export function isExpandable(value: unknown): boolean {
  if (value === null) return false;
  if (Array.isArray(value)) return value.length > 0;
  if (typeof value === "object") {
    return Object.keys(value as Record<string, unknown>).length > 0;
  }
  return false;
}

function truncate(s: string, max: number): string {
  if (s.length <= max) return s;
  return `${s.slice(0, max - 1)}…`;
}

function escapeMarkdown(s: string): string {
  return s.replace(/`/g, "\\`");
}

/**
 * Pulls the capture redaction list out of a parsed `tarn.config.yaml`
 * tree. Accepts `redaction.captures: [key1, key2]`. Anything else
 * (missing, wrong type, unexpected shape) returns an empty list, and
 * `reloadRedaction` logs the exception.
 */
export function extractRedactionCaptures(parsed: unknown): string[] {
  if (!parsed || typeof parsed !== "object") return [];
  const root = parsed as Record<string, unknown>;
  const redaction = root.redaction;
  if (!redaction || typeof redaction !== "object") return [];
  const captures = (redaction as Record<string, unknown>).captures;
  if (!Array.isArray(captures)) return [];
  return captures.filter((c): c is string => typeof c === "string");
}
