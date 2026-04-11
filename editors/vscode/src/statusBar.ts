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
    this.envItem.tooltip = vscode.l10n.t("Tarn: select environment");

    this.summaryItem = vscode.window.createStatusBarItem(
      "tarn.summary",
      vscode.StatusBarAlignment.Right,
      100,
    );
    this.summaryItem.command = "tarn.showOutput";
    this.summaryItem.tooltip = vscode.l10n.t("Tarn: last run summary");

    this.refresh();
  }

  refresh(): void {
    if (!readConfig().statusBarEnabled) {
      this.envItem.hide();
      this.summaryItem.hide();
      return;
    }
    const env = this.state.activeEnvironment ?? vscode.l10n.t("default");
    this.envItem.text = vscode.l10n.t("$(beaker) Tarn: {0}", env);
    this.envItem.show();
    if (this.summaryText.length > 0) {
      this.summaryItem.text = this.summaryText;
      this.summaryItem.show();
    } else {
      this.summaryItem.hide();
    }
  }

  setRunning(progress: string): void {
    this.summaryText = vscode.l10n.t("$(sync~spin) Tarn {0}", progress);
    this.refresh();
  }

  setSummary(passed: number, failed: number, durationMs: number): void {
    const seconds = (durationMs / 1000).toFixed(1);
    const icon = failed === 0 ? "$(check)" : "$(x)";
    this.summaryText = vscode.l10n.t(
      "{0} Tarn {1}/{2} · {3}s",
      icon,
      passed,
      passed + failed,
      seconds,
    );
    this.refresh();
  }

  dispose(): void {
    this.envItem.dispose();
    this.summaryItem.dispose();
  }
}
