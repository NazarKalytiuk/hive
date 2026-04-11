import * as vscode from "vscode";
import type { Report } from "./util/schemaGuards";

/** Setting value for `tarn.notifications.failure`. */
export type FailureNotificationMode = "always" | "focused" | "off";

/**
 * Decide whether a completed run should trigger a warning toast.
 *
 * Pure helper — no VS Code side effects — so unit tests can exercise
 * every branch of the mode × dryRun × failed × tarnVisible matrix.
 */
export function shouldNotifyOnFailure(args: {
  mode: FailureNotificationMode;
  dryRun: boolean;
  failedSteps: number;
  tarnViewVisible: boolean;
}): boolean {
  if (args.mode === "off") return false;
  if (args.dryRun) return false;
  if (args.failedSteps <= 0) return false;
  if (args.mode === "focused" && args.tarnViewVisible) {
    // User is already staring at the Tarn activity bar; they'll see
    // the failure in the tree/fix plan without a toast.
    return false;
  }
  return true;
}

/**
 * Build the message shown in the warning toast. Keeps the name list
 * short so the toast doesn't wrap aggressively in VS Code's tight
 * notification column.
 *
 * Every template variant is routed through `vscode.l10n.t` so the
 * English strings land in `l10n/bundle.l10n.json` and translators
 * can localize the singular/plural/scope variants independently.
 */
export function formatFailureMessage(report: Report): string {
  const failed = report.summary.steps.failed;
  const files = report.files
    .filter((f) => f.status === "FAILED")
    .map((f) => f.name || f.file);
  if (files.length === 0) {
    return failed === 1
      ? vscode.l10n.t("Tarn: {0} failed step", failed)
      : vscode.l10n.t("Tarn: {0} failed steps", failed);
  }
  if (files.length <= 3) {
    const joined = files.join(", ");
    return failed === 1
      ? vscode.l10n.t("Tarn: {0} failed step in {1}", failed, joined)
      : vscode.l10n.t("Tarn: {0} failed steps in {1}", failed, joined);
  }
  return failed === 1
    ? vscode.l10n.t("Tarn: {0} failed step across {1} files", failed, files.length)
    : vscode.l10n.t("Tarn: {0} failed steps across {1} files", failed, files.length);
}

/**
 * Shows failure notifications after a run completes. Wraps the
 * modal `showWarningMessage` call so the extension can inject a
 * custom tarn-focused detector and the commands it dispatches to.
 *
 * The default wiring hands "Show Fix Plan" / "Open Report" /
 * "Rerun Failed" actions to the commands shipped by NAZ-271 / 273
 * / 2-2 respectively. Tests swap in stubs via the constructor.
 */
export class FailureNotifier {
  constructor(
    private readonly isTarnViewFocused: () => boolean,
    private readonly handlers: FailureActionHandlers = defaultHandlers(),
  ) {}

  /**
   * Evaluate the decision without invoking `showWarningMessage`.
   * Exposed so integration tests can exercise the full config +
   * focused-signal path without deadlocking the headless host on
   * a modal toast that never gets dismissed.
   */
  wouldNotify(
    report: Report,
    options: { dryRun: boolean },
  ): boolean {
    return shouldNotifyOnFailure({
      mode: currentMode(),
      dryRun: options.dryRun,
      failedSteps: report.summary.steps.failed,
      tarnViewVisible: this.isTarnViewFocused(),
    });
  }

  async maybeNotify(
    report: Report,
    options: { dryRun: boolean; files: string[] },
  ): Promise<boolean> {
    const shouldShow = this.wouldNotify(report, options);
    if (!shouldShow) return false;

    const message = formatFailureMessage(report);
    const actionShowFixPlan = vscode.l10n.t("Show Fix Plan");
    const actionOpenReport = vscode.l10n.t("Open Report");
    const actionRerunFailed = vscode.l10n.t("Rerun Failed");
    const pick = await vscode.window.showWarningMessage(
      message,
      actionShowFixPlan,
      actionOpenReport,
      actionRerunFailed,
    );
    if (!pick) return true;
    try {
      if (pick === actionShowFixPlan) {
        await this.handlers.showFixPlan();
      } else if (pick === actionOpenReport) {
        await this.handlers.openReport(options.files);
      } else if (pick === actionRerunFailed) {
        await this.handlers.rerunFailed();
      }
    } catch {
      // Action wiring errors are best-effort: the user dismissed
      // the toast either way, we shouldn't crash the run handler.
    }
    return true;
  }
}

/**
 * Callback surface used by {@link FailureNotifier}. Exposed so
 * tests can inject spies without relying on the global command
 * registry.
 */
export interface FailureActionHandlers {
  showFixPlan(): Promise<void> | void;
  openReport(files: readonly string[]): Promise<void> | void;
  rerunFailed(): Promise<void> | void;
}

function defaultHandlers(): FailureActionHandlers {
  return {
    async showFixPlan() {
      // VS Code auto-registers `<treeId>.focus` for every contributed
      // tree view, so this reveals the Fix Plan tree without a bespoke
      // command.
      await vscode.commands.executeCommand("tarn.fixPlan.focus");
    },
    async openReport(files) {
      await vscode.commands.executeCommand(
        "tarn.openHtmlReport",
        files.length > 0 ? files : undefined,
      );
    },
    async rerunFailed() {
      await vscode.commands.executeCommand("tarn.runFailed");
    },
  };
}

function currentMode(): FailureNotificationMode {
  const cfg = vscode.workspace.getConfiguration("tarn");
  const raw = cfg.get<string>("notifications.failure", "focused");
  return raw === "always" || raw === "off" ? raw : "focused";
}
