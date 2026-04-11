import * as vscode from "vscode";
import { WorkspaceIndex } from "./workspace/WorkspaceIndex";
import { createTarnTestController } from "./testing/TestController";
import { TestCodeLensProvider } from "./codelens/TestCodeLensProvider";
import { TarnDocumentSymbolProvider } from "./language/DocumentSymbolProvider";
import { TarnDiagnosticsProvider } from "./language/DiagnosticsProvider";
import { TarnCompletionProvider } from "./language/CompletionProvider";
import { TarnHoverProvider } from "./language/HoverProvider";
import {
  TarnDefinitionProvider,
  TarnReferencesProvider,
  TarnRenameProvider,
} from "./language/SymbolProviders";
import { TarnFormatProvider } from "./language/FormatProvider";
import { LastRunCache } from "./testing/LastRunCache";
import { RequestResponsePanel } from "./views/RequestResponsePanel";
import { TarnStatusBar } from "./statusBar";
import { registerCommands } from "./commands";
import { TarnProcessRunner } from "./backend/TarnProcessRunner";
import { promptInstallIfMissing } from "./backend/binaryResolver";
import { getOutputChannel, disposeOutputChannel } from "./outputChannel";
import { readConfig } from "./config";
import { warnIfTarnOutdated } from "./version";
import {
  RunHistoryStore,
  RunHistoryTreeProvider,
} from "./views/RunHistoryView";
import { EnvironmentsView } from "./views/EnvironmentsView";
import { CapturesInspector } from "./views/CapturesInspector";
import { FixPlanView } from "./views/FixPlanView";
import { ReportWebview } from "./views/ReportWebview";
import { BenchRunnerPanel } from "./views/BenchRunnerPanel";
import { runImportHurl } from "./commands/importHurl";
import { runInitProject } from "./commands/initProject";
import { FailureNotifier } from "./notifications";
import { buildFailureMessages as buildFailureMessagesImpl } from "./testing/ResultMapper";
import type { TarnExtensionApi } from "./api";

export type { TarnExtensionApi } from "./api";

export async function activate(
  context: vscode.ExtensionContext,
): Promise<TarnExtensionApi | undefined> {
  const output = getOutputChannel();
  // l10n-ignore: debug log with static prefix; not user-facing copy.
  output.appendLine("[tarn] activating");

  if (!vscode.workspace.isTrusted) {
    // l10n-ignore: debug log only, shown in Tarn output channel for diagnostics.
    output.appendLine("[tarn] workspace is untrusted; only passive features available");
    context.subscriptions.push(
      vscode.workspace.onDidGrantWorkspaceTrust(() => {
        vscode.commands.executeCommand("workbench.action.reloadWindow");
      }),
    );
    return;
  }

  const resolved = await promptInstallIfMissing();
  const binaryPath = resolved?.path ?? readConfig().binaryPath;
  const backend = new TarnProcessRunner(binaryPath);

  // Check that the resolved Tarn CLI is at or above the extension's
  // declared `tarn.minVersion`. Non-fatal: a mismatch shows a warning
  // with an install link but activation continues so the user can
  // still browse files, edit, and format.
  if (resolved) {
    void warnIfTarnOutdated(context, binaryPath);
  }

  const workspaceRoot = vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
  const index = new WorkspaceIndex({ backend, cwd: workspaceRoot });
  await index.initialize();
  context.subscriptions.push(index);

  const history = new RunHistoryStore(context.workspaceState);
  const historyTree = new RunHistoryTreeProvider(history);
  context.subscriptions.push(
    vscode.window.registerTreeDataProvider("tarn.runHistory", historyTree),
  );

  const lastRunCache = new LastRunCache();
  const stepDetailsPanel = new RequestResponsePanel(context.extensionUri);
  context.subscriptions.push(stepDetailsPanel);

  const capturesInspector = new CapturesInspector();
  context.subscriptions.push(
    capturesInspector,
    vscode.window.registerTreeDataProvider("tarn.captures", capturesInspector),
  );

  const fixPlanView = new FixPlanView(index);
  const fixPlanTree = vscode.window.createTreeView("tarn.fixPlan", {
    treeDataProvider: fixPlanView,
    showCollapseAll: true,
  });
  context.subscriptions.push(fixPlanView, fixPlanTree);

  // "Tarn view focused" = any of our activity-bar tree views is
  // currently visible. They all flip together when the user selects
  // the Tarn container, so checking one is enough — we use the Fix
  // Plan tree since it's the most relevant target for the
  // notification's "Show Fix Plan" action.
  const failureNotifier = new FailureNotifier(() => fixPlanTree.visible);

  const reportWebview = new ReportWebview(index);
  context.subscriptions.push(reportWebview);

  const benchRunnerPanel = new BenchRunnerPanel();
  context.subscriptions.push(benchRunnerPanel);

  const tarnController = createTarnTestController(
    index,
    backend,
    history,
    lastRunCache,
    capturesInspector,
    fixPlanView,
    failureNotifier,
    () => historyTree.refresh(),
  );
  context.subscriptions.push(tarnController);

  const codeLens = new TestCodeLensProvider(index);
  context.subscriptions.push(
    vscode.languages.registerCodeLensProvider({ language: "tarn" }, codeLens),
    codeLens,
  );

  context.subscriptions.push(
    vscode.languages.registerDocumentSymbolProvider(
      { language: "tarn" },
      new TarnDocumentSymbolProvider(index),
    ),
  );

  const diagnostics = new TarnDiagnosticsProvider(backend);
  context.subscriptions.push(diagnostics);

  const environmentsView = new EnvironmentsView(backend, tarnController.state);
  context.subscriptions.push(
    environmentsView,
    vscode.window.registerTreeDataProvider("tarn.environments", environmentsView),
  );

  const completionProvider = new TarnCompletionProvider(environmentsView);
  context.subscriptions.push(
    vscode.languages.registerCompletionItemProvider(
      { language: "tarn" },
      completionProvider,
      "{",
      ".",
      "$",
      " ",
    ),
  );

  const hoverProvider = new TarnHoverProvider(environmentsView);
  context.subscriptions.push(
    vscode.languages.registerHoverProvider({ language: "tarn" }, hoverProvider),
  );

  const formatProvider = new TarnFormatProvider(backend);
  context.subscriptions.push(
    vscode.languages.registerDefinitionProvider(
      { language: "tarn" },
      new TarnDefinitionProvider(environmentsView),
    ),
    vscode.languages.registerReferenceProvider(
      { language: "tarn" },
      new TarnReferencesProvider(),
    ),
    vscode.languages.registerRenameProvider(
      { language: "tarn" },
      new TarnRenameProvider(),
    ),
    vscode.languages.registerDocumentFormattingEditProvider(
      { language: "tarn" },
      formatProvider,
    ),
  );

  const statusBar = new TarnStatusBar(tarnController.state);
  context.subscriptions.push(statusBar);

  context.subscriptions.push(
    registerCommands({
      tarnController,
      index,
      backend,
      history,
      environmentsView,
      lastRunCache,
      stepDetailsPanel,
      capturesInspector,
      fixPlanView,
      reportWebview,
      benchRunnerPanel,
      workspaceState: context.workspaceState,
      historyTree,
      refreshStatusBar: () => statusBar.refresh(),
      refreshHistoryView: () => historyTree.refresh(),
      refreshEnvironmentsView: () => environmentsView.refresh(),
    }),
  );

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((event) => {
      if (event.affectsConfiguration("tarn")) {
        statusBar.refresh();
      }
    }),
  );

  // l10n-ignore: debug log with tarn prefix; engineers read this in the output channel.
  output.appendLine(
    `[tarn] ready (${index.all.length} test file(s) indexed)`,
  );

  return {
    testControllerId: tarnController.controller.id,
    indexedFileCount: index.all.length,
    commands: [
      "tarn.runAll",
      "tarn.runFile",
      "tarn.dryRunFile",
      "tarn.validateFile",
      "tarn.rerunLast",
      "tarn.runFailed",
      "tarn.selectEnvironment",
      "tarn.setTagFilter",
      "tarn.clearTagFilter",
      "tarn.showOutput",
      "tarn.installTarn",
      "tarn.exportCurl",
      "tarn.clearHistory",
      "tarn.showWalkthrough",
      "tarn.initProject",
      "tarn.refreshDiscovery",
      "tarn.reloadEnvironments",
      "tarn.showStepDetails",
      "tarn.copyCaptureValue",
      "tarn.toggleHideCaptures",
      "tarn.jumpToFailure",
      "tarn.openHtmlReport",
      "tarn.benchStep",
      "tarn.importHurl",
      "tarn.pinHistoryEntry",
      "tarn.unpinHistoryEntry",
      "tarn.filterHistory",
      "tarn.rerunFromHistory",
    ],
    testing: {
      backend,
      validateDocument: async (uri: vscode.Uri) => {
        const doc = await vscode.workspace.openTextDocument(uri);
        await diagnostics.validate(doc);
      },
      reloadEnvironments: async () => {
        await environmentsView.reload();
      },
      listEnvironments: async () => environmentsView.getEntries(),
      getActiveEnvironment: () => tarnController.state.activeEnvironment,
      formatDocument: async (uri: vscode.Uri) => {
        const doc = await vscode.workspace.openTextDocument(uri);
        const cts = new vscode.CancellationTokenSource();
        try {
          const result = await formatProvider.provideDocumentFormattingEdits(
            doc,
            { tabSize: 2, insertSpaces: true },
            cts.token,
          );
          return result ?? [];
        } finally {
          cts.dispose();
        }
      },
      lastRunCacheSize: () => lastRunCache.size(),
      loadLastRunFromReport: (report) => lastRunCache.loadFromReport(report),
      showStepDetails: (key) => {
        const snapshot = lastRunCache.get(key);
        if (!snapshot) return false;
        stepDetailsPanel.show(snapshot);
        return true;
      },
      loadCapturesFromReport: (report) => capturesInspector.loadFromReport(report),
      capturesTotalCount: () => capturesInspector.totalCaptureCount(),
      isCaptureKeyRedacted: (key) => capturesInspector.isKeyRedacted(key),
      isHidingAllCaptures: () => capturesInspector.isHidingAllValues(),
      toggleHideCaptures: () => capturesInspector.toggleHideAllValues(),
      loadFixPlanFromReport: (report) => fixPlanView.loadFromReport(report),
      fixPlanSnapshot: () => fixPlanView.snapshot(),
      showReportHtml: (html) => reportWebview.show(html),
      sendReportMessage: (message) => reportWebview.handleMessage(message),
      showBenchResult: (context) => benchRunnerPanel.show(context),
      lastBenchContext: () => benchRunnerPanel.lastContext(),
      importHurl: (source, dest, cwd) =>
        runImportHurl(backend, source, dest, cwd),
      initProject: (options) => runInitProject({ backend }, options),
      history: {
        add: (entry) => history.add(entry),
        all: () => history.all(),
        clear: () => history.clear(),
        setFilter: (filter) => historyTree.setFilter(filter),
        getFilter: () => historyTree.getFilter(),
      },
      notifier: {
        isTarnViewFocused: () => fixPlanTree.visible,
        wouldNotify: (report, options) =>
          failureNotifier.wouldNotify(report, options),
        maybeNotify: (report, options) =>
          failureNotifier.maybeNotify(report, options),
      },
      workspaceIndexSnapshot: () =>
        index.all.map((parsed) => ({
          uri: parsed.uri.toString(),
          fileName: parsed.ranges.fileName,
          tests: parsed.ranges.tests.map((t) => ({
            name: t.name,
            stepCount: t.steps.length,
          })),
          fromScopedList: parsed.fromScopedList === true,
        })),
      refreshSingleFile: (uri) => index.refreshSingleFile(uri),
      buildFailureMessagesForStep: (step, fileUri, astFallback) => {
        // Synthesize a minimal ParsedFile. We deliberately do not pull
        // the real WorkspaceIndex entry so the test can feed in a
        // specific URI (e.g., a fixture outside the indexed workspace).
        const parsed = {
          uri: fileUri,
          ranges: {
            fileName: "(integration-test synthetic)",
            fileNameRange: undefined,
            tests: [],
            setup: [],
            teardown: [],
          },
        };
        // Synthesize a minimal TestItem with just the fields
        // buildFailureMessages reads (`range`). Using a plain object
        // matches the unit-test pattern and avoids the TestController
        // tree.
        const stepItem = { range: astFallback ?? undefined };
        return buildFailureMessagesImpl(
          step,
          stepItem as unknown as vscode.TestItem,
          parsed as unknown as import("./workspace/WorkspaceIndex").ParsedFile,
        );
      },
    },
  };
}

export function deactivate(): void {
  disposeOutputChannel();
}
