import * as vscode from "vscode";
import type { RunState } from "./testing/runHandler";
import { readConfig } from "./config";

export class TarnStatusBar implements vscode.Disposable {
  private readonly envItem: vscode.StatusBarItem;
  private readonly summaryItem: vscode.StatusBarItem;
  private summaryText = "";

  constructor(private readonly state: RunState) {
    this.envItem = vscode.window.createStatusBarItem(
      "tarn.env",
      vscode.StatusBarAlignment.Left,
      100,
    );
    this.envItem.command = "tarn.selectEnvironment";
    this.envItem.tooltip = "Tarn: select environment";

    this.summaryItem = vscode.window.createStatusBarItem(
      "tarn.summary",
      vscode.StatusBarAlignment.Right,
      100,
    );
    this.summaryItem.command = "tarn.showOutput";
    this.summaryItem.tooltip = "Tarn: last run summary";

    this.refresh();
  }

  refresh(): void {
    if (!readConfig().statusBarEnabled) {
      this.envItem.hide();
      this.summaryItem.hide();
      return;
    }
    const env = this.state.activeEnvironment ?? "default";
    this.envItem.text = `$(beaker) Tarn: ${env}`;
    this.envItem.show();
    if (this.summaryText.length > 0) {
      this.summaryItem.text = this.summaryText;
      this.summaryItem.show();
    } else {
      this.summaryItem.hide();
    }
  }

  setRunning(progress: string): void {
    this.summaryText = `$(sync~spin) Tarn ${progress}`;
    this.refresh();
  }

  setSummary(passed: number, failed: number, durationMs: number): void {
    const seconds = (durationMs / 1000).toFixed(1);
    const icon = failed === 0 ? "$(check)" : "$(x)";
    this.summaryText = `${icon} Tarn ${passed}/${passed + failed} · ${seconds}s`;
    this.refresh();
  }

  dispose(): void {
    this.envItem.dispose();
    this.summaryItem.dispose();
  }
}
