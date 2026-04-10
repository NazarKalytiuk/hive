import * as vscode from "vscode";
import type { Report } from "../util/schemaGuards";

export interface RunHistoryEntry {
  id: string;
  timestamp: number;
  label: string;
  environment: string | null;
  tags: string[];
  status: "PASSED" | "FAILED" | "CANCELLED" | "ERRORED";
  passed: number;
  failed: number;
  total: number;
  durationMs: number;
  files: string[];
  dryRun: boolean;
}

const STORAGE_KEY = "tarn.runHistory";
const MAX_ENTRIES = 20;

export class RunHistoryStore {
  constructor(private readonly memento: vscode.Memento) {}

  all(): RunHistoryEntry[] {
    return this.memento.get<RunHistoryEntry[]>(STORAGE_KEY, []);
  }

  async add(entry: RunHistoryEntry): Promise<void> {
    const current = this.all();
    current.unshift(entry);
    while (current.length > MAX_ENTRIES) {
      current.pop();
    }
    await this.memento.update(STORAGE_KEY, current);
  }

  async clear(): Promise<void> {
    await this.memento.update(STORAGE_KEY, []);
  }

  static entryFromReport(
    report: Report,
    environment: string | null,
    tags: string[],
    dryRun: boolean,
  ): RunHistoryEntry {
    return {
      id: `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      timestamp: Date.now(),
      label: `${report.summary.steps.passed}/${report.summary.steps.total} steps`,
      environment,
      tags,
      status: report.summary.status,
      passed: report.summary.steps.passed,
      failed: report.summary.steps.failed,
      total: report.summary.steps.total,
      durationMs: report.duration_ms,
      files: report.files.map((f) => f.file),
      dryRun,
    };
  }
}

export class RunHistoryTreeProvider
  implements vscode.TreeDataProvider<RunHistoryEntry | FileNode>
{
  private readonly emitter = new vscode.EventEmitter<void>();
  readonly onDidChangeTreeData = this.emitter.event;

  constructor(private readonly store: RunHistoryStore) {}

  refresh(): void {
    this.emitter.fire();
  }

  getTreeItem(element: RunHistoryEntry | FileNode): vscode.TreeItem {
    if ("file" in element) {
      const item = new vscode.TreeItem(
        element.file,
        vscode.TreeItemCollapsibleState.None,
      );
      item.resourceUri = vscode.Uri.file(element.file);
      item.command = {
        command: "vscode.open",
        title: "Open",
        arguments: [item.resourceUri],
      };
      return item;
    }
    const entry = element;
    const icon =
      entry.status === "PASSED"
        ? "$(check)"
        : entry.status === "FAILED"
          ? "$(x)"
          : "$(alert)";
    const date = new Date(entry.timestamp).toLocaleTimeString();
    const label = `${icon} ${date} · ${entry.label}${entry.dryRun ? " (dry)" : ""}`;
    const item = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.Collapsed);
    item.tooltip = this.renderTooltip(entry);
    item.description = this.renderDescription(entry);
    item.id = entry.id;
    item.contextValue = "tarnRunEntry";
    return item;
  }

  getChildren(
    element?: RunHistoryEntry | FileNode,
  ): vscode.ProviderResult<(RunHistoryEntry | FileNode)[]> {
    if (!element) {
      return this.store.all();
    }
    if ("file" in element) {
      return [];
    }
    return element.files.map((file) => ({ file }));
  }

  private renderDescription(entry: RunHistoryEntry): string {
    const parts: string[] = [];
    if (entry.environment) {
      parts.push(entry.environment);
    }
    if (entry.tags.length > 0) {
      parts.push(entry.tags.join(","));
    }
    parts.push(`${(entry.durationMs / 1000).toFixed(1)}s`);
    return parts.join(" · ");
  }

  private renderTooltip(entry: RunHistoryEntry): string {
    const lines = [
      `Status: ${entry.status}`,
      `Passed: ${entry.passed}/${entry.total}`,
      `Duration: ${(entry.durationMs / 1000).toFixed(2)}s`,
    ];
    if (entry.environment) {
      lines.push(`Env: ${entry.environment}`);
    }
    if (entry.tags.length > 0) {
      lines.push(`Tags: ${entry.tags.join(", ")}`);
    }
    if (entry.dryRun) {
      lines.push("Dry run");
    }
    return lines.join("\n");
  }
}

interface FileNode {
  file: string;
}
