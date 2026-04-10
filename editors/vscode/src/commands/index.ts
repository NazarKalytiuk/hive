import * as vscode from "vscode";
import type { TarnTestController } from "../testing/TestController";
import type { WorkspaceIndex } from "../workspace/WorkspaceIndex";
import type { TarnBackend } from "../backend/TarnBackend";
import { getOutputChannel } from "../outputChannel";
import { ids } from "../testing/discovery";
import type { RunHistoryStore } from "../views/RunHistoryView";

export interface CommandDeps {
  tarnController: TarnTestController;
  index: WorkspaceIndex;
  backend: TarnBackend;
  history: RunHistoryStore;
  refreshStatusBar: () => void;
  refreshHistoryView: () => void;
}

export function registerCommands(deps: CommandDeps): vscode.Disposable {
  const registrations: vscode.Disposable[] = [];

  registrations.push(
    vscode.commands.registerCommand("tarn.runAll", async () => {
      const request = new vscode.TestRunRequest(
        undefined,
        undefined,
        deps.tarnController.runProfile,
      );
      await runViaProfile(request, deps.tarnController.runProfile);
    }),
  );

  registrations.push(
    vscode.commands.registerCommand("tarn.runFile", async () => {
      await runActiveFile(false);
    }),
  );

  registrations.push(
    vscode.commands.registerCommand("tarn.dryRunFile", async () => {
      await runActiveFile(true);
    }),
  );

  registrations.push(
    vscode.commands.registerCommand("tarn.validateFile", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        return;
      }
      const folder = vscode.workspace.getWorkspaceFolder(editor.document.uri);
      if (!folder) {
        return;
      }
      const token = new vscode.CancellationTokenSource().token;
      const result = await deps.backend.validate(
        [editor.document.uri.fsPath],
        folder.uri.fsPath,
        token,
      );
      if (result.exitCode === 0) {
        vscode.window.showInformationMessage("Tarn: file is valid.");
      } else {
        const out = getOutputChannel();
        out.show(true);
        out.appendLine(result.stdout || result.stderr || "Tarn validation failed");
      }
    }),
  );

  registrations.push(
    vscode.commands.registerCommand("tarn.rerunLast", async () => {
      await deps.tarnController.rerunLast();
    }),
  );

  registrations.push(
    vscode.commands.registerCommand("tarn.selectEnvironment", async () => {
      const envs = await collectEnvironments();
      type Pick = vscode.QuickPickItem & { value: string | null };
      const items: Pick[] = [
        { label: "$(close) (none)", description: "clear active environment", value: null },
        ...envs.map<Pick>((e) => ({ label: e, description: "", value: e })),
      ];
      const picked = await vscode.window.showQuickPick<Pick>(items, {
        placeHolder: "Select Tarn environment",
      });
      if (!picked) {
        return;
      }
      deps.tarnController.state.activeEnvironment = picked.value;
      deps.refreshStatusBar();
    }),
  );

  registrations.push(
    vscode.commands.registerCommand("tarn.setTagFilter", async () => {
      const input = await vscode.window.showInputBox({
        prompt: "Comma-separated tag filter (leave empty to clear)",
        value: deps.tarnController.state.activeTags.join(","),
      });
      if (input === undefined) {
        return;
      }
      deps.tarnController.state.activeTags = input
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
      deps.refreshStatusBar();
    }),
  );

  registrations.push(
    vscode.commands.registerCommand("tarn.clearTagFilter", () => {
      deps.tarnController.state.activeTags = [];
      deps.refreshStatusBar();
    }),
  );

  registrations.push(
    vscode.commands.registerCommand("tarn.showOutput", () => {
      getOutputChannel().show(true);
    }),
  );

  registrations.push(
    vscode.commands.registerCommand("tarn.installTarn", async () => {
      await vscode.env.openExternal(
        vscode.Uri.parse("https://github.com/NazarKalytiuk/hive#install"),
      );
    }),
  );

  registrations.push(
    vscode.commands.registerCommand("tarn.exportCurl", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) {
        return;
      }
      const folder = vscode.workspace.getWorkspaceFolder(editor.document.uri);
      if (!folder) {
        return;
      }
      const mode = await vscode.window.showQuickPick(
        [
          { label: "All steps", description: "--format curl-all", value: "all" as const },
          {
            label: "Failed steps only",
            description: "--format curl",
            value: "failed" as const,
          },
        ],
        { placeHolder: "Export mode" },
      );
      if (!mode) {
        return;
      }
      const token = new vscode.CancellationTokenSource().token;
      const result = await deps.backend.exportCurl(
        [editor.document.uri.fsPath],
        folder.uri.fsPath,
        mode.value,
        token,
      );
      if (result.stdout.length === 0) {
        vscode.window.showInformationMessage("Tarn: nothing to export.");
        return;
      }
      const doc = await vscode.workspace.openTextDocument({
        language: "shellscript",
        content: result.stdout,
      });
      await vscode.window.showTextDocument(doc, { preview: false });
    }),
  );

  registrations.push(
    vscode.commands.registerCommand("tarn.clearHistory", async () => {
      await deps.history.clear();
      deps.refreshHistoryView();
    }),
  );

  registrations.push(
    vscode.commands.registerCommand("tarn.showWalkthrough", async () => {
      await vscode.commands.executeCommand(
        "workbench.action.openWalkthrough",
        "nazarkalytiuk.tarn-vscode#tarn.gettingStarted",
        false,
      );
    }),
  );

  registrations.push(
    vscode.commands.registerCommand(
      "tarn.runTestFromCodeLens",
      async (itemId: string, dryRun: boolean) => {
        const item = findItemById(deps.tarnController.controller, itemId);
        if (!item) {
          return;
        }
        const profile = dryRun
          ? deps.tarnController.dryRunProfile
          : deps.tarnController.runProfile;
        const request = new vscode.TestRunRequest([item], undefined, profile);
        await runViaProfile(request, profile);
      },
    ),
  );

  registrations.push(
    vscode.commands.registerCommand(
      "tarn.dryRunTestFromCodeLens",
      async (itemId: string) => {
        await vscode.commands.executeCommand("tarn.runTestFromCodeLens", itemId, true);
      },
    ),
  );

  return vscode.Disposable.from(...registrations);

  async function runActiveFile(dryRun: boolean): Promise<void> {
    const editor = vscode.window.activeTextEditor;
    if (!editor) {
      return;
    }
    const parsed = deps.index.get(editor.document.uri);
    if (!parsed) {
      vscode.window.showInformationMessage(
        "Tarn: current file is not indexed as a Tarn test file.",
      );
      return;
    }
    const item = deps.tarnController.controller.items.get(ids.file(parsed.uri));
    if (!item) {
      return;
    }
    const profile = dryRun
      ? deps.tarnController.dryRunProfile
      : deps.tarnController.runProfile;
    const request = new vscode.TestRunRequest([item], undefined, profile);
    await runViaProfile(request, profile);
  }
}

async function runViaProfile(
  request: vscode.TestRunRequest,
  profile: vscode.TestRunProfile,
): Promise<void> {
  const cts = new vscode.CancellationTokenSource();
  try {
    await profile.runHandler(request, cts.token);
  } finally {
    cts.dispose();
  }
}

function findItemById(
  controller: vscode.TestController,
  id: string,
): vscode.TestItem | undefined {
  let found: vscode.TestItem | undefined;
  const visit = (item: vscode.TestItem) => {
    if (found) {
      return;
    }
    if (item.id === id) {
      found = item;
      return;
    }
    item.children.forEach(visit);
  };
  controller.items.forEach(visit);
  return found;
}

async function collectEnvironments(): Promise<string[]> {
  const folder = vscode.workspace.workspaceFolders?.[0];
  if (!folder) {
    return [];
  }
  const pattern = new vscode.RelativePattern(folder, "tarn.env.*.yaml");
  const uris = await vscode.workspace.findFiles(pattern);
  return uris
    .map((u) => {
      const base = u.path.split("/").pop() ?? "";
      const match = /^tarn\.env\.([A-Za-z0-9_\-]+)\.yaml$/.exec(base);
      return match?.[1] ?? "";
    })
    .filter((n) => n.length > 0 && n !== "local");
}
