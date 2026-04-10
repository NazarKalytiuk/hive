import * as vscode from "vscode";
import type { WorkspaceIndex } from "../workspace/WorkspaceIndex";

export class TarnDocumentSymbolProvider implements vscode.DocumentSymbolProvider {
  constructor(private readonly index: WorkspaceIndex) {}

  provideDocumentSymbols(
    document: vscode.TextDocument,
    _token: vscode.CancellationToken,
  ): vscode.ProviderResult<vscode.DocumentSymbol[]> {
    const parsed = this.index.get(document.uri);
    if (!parsed) {
      return [];
    }

    const result: vscode.DocumentSymbol[] = [];

    if (parsed.ranges.setup.length > 0) {
      result.push(this.buildGroup("setup", parsed.ranges.setup, vscode.SymbolKind.Event));
    }

    for (const test of parsed.ranges.tests) {
      const testSymbol = new vscode.DocumentSymbol(
        test.name,
        test.description ?? "",
        vscode.SymbolKind.Class,
        test.nameRange,
        test.nameRange,
      );
      for (const step of test.steps) {
        testSymbol.children.push(
          new vscode.DocumentSymbol(
            step.name,
            `step ${step.index + 1}`,
            vscode.SymbolKind.Method,
            step.nameRange,
            step.nameRange,
          ),
        );
      }
      result.push(testSymbol);
    }

    if (parsed.ranges.teardown.length > 0) {
      result.push(
        this.buildGroup("teardown", parsed.ranges.teardown, vscode.SymbolKind.Event),
      );
    }

    return result;
  }

  private buildGroup(
    label: string,
    steps: readonly { name: string; index: number; nameRange: vscode.Range }[],
    kind: vscode.SymbolKind,
  ): vscode.DocumentSymbol {
    const first = steps[0];
    const range = first?.nameRange ?? new vscode.Range(0, 0, 0, 0);
    const group = new vscode.DocumentSymbol(label, "", kind, range, range);
    for (const step of steps) {
      group.children.push(
        new vscode.DocumentSymbol(
          step.name,
          `step ${step.index + 1}`,
          vscode.SymbolKind.Method,
          step.nameRange,
          step.nameRange,
        ),
      );
    }
    return group;
  }
}
