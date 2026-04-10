import * as assert from "assert";
import * as vscode from "vscode";

const EXTENSION_ID = "nazarkalytiuk.tarn-vscode";

async function waitUntil<T>(
  predicate: () => T | undefined,
  timeoutMs = 10000,
  stepMs = 100,
): Promise<T> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const value = predicate();
    if (value !== undefined) {
      return value;
    }
    await new Promise((r) => setTimeout(r, stepMs));
  }
  throw new Error("waitUntil timed out");
}

function findController(): vscode.TestController | undefined {
  const all = (vscode.tests as unknown as { testControllers?: vscode.TestController[] })
    .testControllers;
  if (!all) {
    return undefined;
  }
  return all.find((c) => c.id === "tarn");
}

function collectItemIds(controller: vscode.TestController): string[] {
  const ids: string[] = [];
  const visit = (item: vscode.TestItem) => {
    ids.push(item.id);
    item.children.forEach(visit);
  };
  controller.items.forEach(visit);
  return ids;
}

describe("Tarn extension: discovery", () => {
  it("activates in a workspace containing .tarn.yaml files", async function () {
    this.timeout(30000);
    const ext = vscode.extensions.getExtension(EXTENSION_ID);
    assert.ok(ext, `extension ${EXTENSION_ID} not found`);
    await ext!.activate();
    assert.strictEqual(ext!.isActive, true);
  });

  it("registers the Tarn test controller", async function () {
    this.timeout(15000);
    const controller = await waitUntil(() => findController());
    assert.ok(controller, "Tarn test controller not registered");
  });

  it("discovers the fixture .tarn.yaml file", async function () {
    this.timeout(15000);
    const controller = findController();
    if (!controller) {
      assert.fail("controller missing");
      return;
    }

    const ids = await waitUntil(() => {
      const current = collectItemIds(controller);
      return current.length > 0 ? current : undefined;
    });

    const fileItems = ids.filter((id) => id.startsWith("file:"));
    assert.ok(
      fileItems.length >= 1,
      `expected at least 1 discovered file, got ${fileItems.length}`,
    );

    const testItems = ids.filter((id) => id.startsWith("test:"));
    assert.ok(
      testItems.length >= 1,
      `expected at least 1 discovered test, got ${testItems.length}`,
    );
  });

  it("provides document symbols for a discovered file", async function () {
    this.timeout(15000);
    const uris = await vscode.workspace.findFiles("**/*.tarn.yaml");
    assert.ok(uris.length > 0, "fixture not found");
    const doc = await vscode.workspace.openTextDocument(uris[0]);
    await vscode.window.showTextDocument(doc);
    const symbols = (await vscode.commands.executeCommand(
      "vscode.executeDocumentSymbolProvider",
      doc.uri,
    )) as vscode.DocumentSymbol[] | undefined;
    assert.ok(symbols && symbols.length > 0, "no document symbols returned");
  });

  it("registers Tarn commands", async function () {
    this.timeout(5000);
    const commands = await vscode.commands.getCommands(true);
    for (const expected of [
      "tarn.runAll",
      "tarn.runFile",
      "tarn.dryRunFile",
      "tarn.selectEnvironment",
      "tarn.setTagFilter",
      "tarn.exportCurl",
      "tarn.clearHistory",
    ]) {
      assert.ok(commands.includes(expected), `missing command: ${expected}`);
    }
  });
});
