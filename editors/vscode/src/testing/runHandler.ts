import * as vscode from "vscode";
import * as path from "path";
import type { TarnBackend } from "../backend/TarnBackend";
import type { WorkspaceIndex, ParsedFile } from "../workspace/WorkspaceIndex";
import { applyReport } from "./ResultMapper";
import { readConfig } from "../config";
import { getOutputChannel } from "../outputChannel";
import { RunHistoryStore } from "../views/RunHistoryView";

export interface RunState {
  activeEnvironment: string | null;
  activeTags: string[];
  lastRequest: vscode.TestRunRequest | undefined;
  lastDryRun: boolean;
}

export interface HandlerDeps {
  controller: vscode.TestController;
  backend: TarnBackend;
  index: WorkspaceIndex;
  state: RunState;
  history: RunHistoryStore;
  onHistoryChanged: () => void;
}

export function createRunHandler(
  deps: HandlerDeps,
  dryRun: boolean,
): (request: vscode.TestRunRequest, token: vscode.CancellationToken) => Promise<void> {
  return async (request, token) => {
    deps.state.lastRequest = request;
    deps.state.lastDryRun = dryRun;

    const run = deps.controller.createTestRun(request, dryRun ? "Tarn Dry Run" : "Tarn Run", true);
    try {
      await executeRun(deps, request, run, token, dryRun);
    } catch (err) {
      getOutputChannel().appendLine(`[tarn] run failed: ${String(err)}`);
      vscode.window.showErrorMessage(`Tarn run failed: ${String(err)}`);
    } finally {
      run.end();
    }
  };
}

async function executeRun(
  deps: HandlerDeps,
  request: vscode.TestRunRequest,
  run: vscode.TestRun,
  token: vscode.CancellationToken,
  dryRun: boolean,
): Promise<void> {
  const filesToRun = collectFilesForRequest(deps, request);
  if (filesToRun.length === 0) {
    run.appendOutput("No Tarn test files matched this run.\r\n");
    return;
  }

  const itemsById = collectAllTestItems(deps.controller);
  const parsedByPath = new Map<string, ParsedFile>();
  for (const parsed of deps.index.all) {
    parsedByPath.set(parsed.uri.fsPath, parsed);
  }

  for (const file of filesToRun) {
    enqueueFileItems(file, run, itemsById);
  }

  const config = readConfig();
  const cwd = primaryWorkspaceFolder();
  if (!cwd) {
    run.appendOutput("No workspace folder found; cannot invoke tarn.\r\n");
    return;
  }

  run.appendOutput(
    `[tarn] Running ${filesToRun.length} file(s)${dryRun ? " (dry run)" : ""}\r\n`,
  );

  const outcome = await deps.backend.run({
    files: filesToRun.map((f) => path.relative(cwd, f.uri.fsPath)),
    cwd,
    environment: deps.state.activeEnvironment ?? config.defaultEnvironment,
    tags: deps.state.activeTags.length > 0 ? deps.state.activeTags : config.defaultTags,
    parallel: config.parallel,
    jsonMode: config.jsonMode,
    dryRun,
    token,
  });

  if (token.isCancellationRequested || outcome.cancelled) {
    run.appendOutput("[tarn] Run cancelled.\r\n");
    return;
  }

  if (!outcome.report) {
    run.appendOutput(
      `[tarn] Run did not produce a parseable JSON report (exit ${outcome.exitCode}).\r\n`,
    );
    if (outcome.stderr) {
      run.appendOutput(outcome.stderr);
    }
    markAllErrored(deps, filesToRun, itemsById, run, outcome.stderr || "tarn produced no JSON report");
    return;
  }

  applyReport(outcome.report, {
    run,
    parsedByPath,
    testItemsById: itemsById,
  });

  const summary = outcome.report.summary;
  run.appendOutput(
    `[tarn] Done. ${summary.steps.passed}/${summary.steps.total} steps passed across ${summary.files} file(s).\r\n`,
  );

  const entry = RunHistoryStore.entryFromReport(
    outcome.report,
    deps.state.activeEnvironment ?? config.defaultEnvironment,
    deps.state.activeTags.length > 0 ? deps.state.activeTags : config.defaultTags,
    dryRun,
  );
  await deps.history.add(entry);
  deps.onHistoryChanged();
}

function collectFilesForRequest(
  deps: HandlerDeps,
  request: vscode.TestRunRequest,
): ParsedFile[] {
  const selected = request.include;
  const excluded = new Set((request.exclude ?? []).map((i) => i.id));

  const all = deps.index.all.filter((parsed) => {
    return !excluded.has(fileId(parsed));
  });

  if (!selected || selected.length === 0) {
    return all;
  }

  const chosen = new Set<string>();
  for (const item of selected) {
    const parsed = resolveParsedFor(item, deps);
    if (parsed && !excluded.has(fileId(parsed))) {
      chosen.add(parsed.uri.toString());
    }
  }
  return all.filter((parsed) => chosen.has(parsed.uri.toString()));
}

function resolveParsedFor(
  item: vscode.TestItem,
  deps: HandlerDeps,
): ParsedFile | undefined {
  const uri = item.uri;
  if (!uri) {
    return undefined;
  }
  return deps.index.get(uri);
}

function fileId(parsed: ParsedFile): string {
  return `file:${parsed.uri.toString()}`;
}

function enqueueFileItems(
  parsed: ParsedFile,
  run: vscode.TestRun,
  itemsById: Map<string, vscode.TestItem>,
): void {
  const top = itemsById.get(fileId(parsed));
  if (!top) {
    return;
  }
  enqueueRecursive(top, run);
}

function enqueueRecursive(item: vscode.TestItem, run: vscode.TestRun): void {
  run.enqueued(item);
  item.children.forEach((child) => enqueueRecursive(child, run));
}

function collectAllTestItems(
  controller: vscode.TestController,
): Map<string, vscode.TestItem> {
  const map = new Map<string, vscode.TestItem>();
  const visit = (item: vscode.TestItem) => {
    map.set(item.id, item);
    item.children.forEach(visit);
  };
  controller.items.forEach(visit);
  return map;
}

function primaryWorkspaceFolder(): string | undefined {
  return vscode.workspace.workspaceFolders?.[0]?.uri.fsPath;
}

function markAllErrored(
  _deps: HandlerDeps,
  files: ParsedFile[],
  itemsById: Map<string, vscode.TestItem>,
  run: vscode.TestRun,
  message: string,
): void {
  for (const parsed of files) {
    const item = itemsById.get(fileId(parsed));
    if (item) {
      run.errored(item, new vscode.TestMessage(message));
    }
  }
}
