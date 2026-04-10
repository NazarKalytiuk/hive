import * as vscode from "vscode";
import type { WorkspaceIndex } from "../workspace/WorkspaceIndex";
import { readConfig } from "../config";
import { ids } from "../testing/discovery";

export class TestCodeLensProvider implements vscode.CodeLensProvider {
  private readonly emitter = new vscode.EventEmitter<void>();
  readonly onDidChangeCodeLenses = this.emitter.event;

  constructor(private readonly index: WorkspaceIndex) {
    this.index.onDidChange(() => this.emitter.fire());
  }

  provideCodeLenses(
    document: vscode.TextDocument,
    _token: vscode.CancellationToken,
  ): vscode.ProviderResult<vscode.CodeLens[]> {
    if (!readConfig(document.uri).showCodeLens) {
      return [];
    }
    const parsed = this.index.get(document.uri);
    if (!parsed) {
      return [];
    }

    const lenses: vscode.CodeLens[] = [];
    for (const test of parsed.ranges.tests) {
      const testItemId = ids.test(document.uri, test.name);
      lenses.push(
        new vscode.CodeLens(test.nameRange, {
          title: "$(play) Run",
          command: "tarn.runTestFromCodeLens",
          arguments: [testItemId, false],
        }),
      );
      lenses.push(
        new vscode.CodeLens(test.nameRange, {
          title: "$(debug-alt-small) Dry Run",
          command: "tarn.dryRunTestFromCodeLens",
          arguments: [testItemId, true],
        }),
      );

      for (const step of test.steps) {
        const stepItemId = ids.step(document.uri, test.name, step.index);
        lenses.push(
          new vscode.CodeLens(step.nameRange, {
            title: "$(play) Run step",
            command: "tarn.runTestFromCodeLens",
            arguments: [stepItemId, false],
          }),
        );
      }
    }

    return lenses;
  }

  dispose(): void {
    this.emitter.dispose();
  }
}
