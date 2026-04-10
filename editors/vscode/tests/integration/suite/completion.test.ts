import * as assert from "assert";
import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

const EXTENSION_ID = "nazarkalytiuk.tarn-vscode";

async function ensureActivated(): Promise<void> {
  const ext = vscode.extensions.getExtension(EXTENSION_ID);
  assert.ok(ext, `extension ${EXTENSION_ID} not found`);
  await ext!.activate();
}

function workspaceRoot(): string {
  const folder = vscode.workspace.workspaceFolders?.[0];
  if (!folder) {
    throw new Error("no workspace folder available");
  }
  return folder.uri.fsPath;
}

function writeFixture(relativePath: string, content: string): vscode.Uri {
  const absolute = path.join(workspaceRoot(), relativePath);
  fs.mkdirSync(path.dirname(absolute), { recursive: true });
  fs.writeFileSync(absolute, content, "utf8");
  return vscode.Uri.file(absolute);
}

async function completionsAt(
  uri: vscode.Uri,
  line: number,
  character: number,
  trigger?: string,
): Promise<vscode.CompletionList> {
  const position = new vscode.Position(line, character);
  const list = (await vscode.commands.executeCommand(
    "vscode.executeCompletionItemProvider",
    uri,
    position,
    trigger,
  )) as vscode.CompletionList | undefined;
  assert.ok(list, "completion provider returned nothing");
  return list!;
}

/**
 * Filter a raw completion list down to items produced by
 * TarnCompletionProvider. VS Code merges results from every
 * completion provider (the built-in word completer, the YAML
 * extension, etc.), so tests must disambiguate via a stable marker.
 * Our provider always sets `detail` to one of: an env.KEY string,
 * "capture from ...", "Environment variable"/"Captured variable"/
 * "Built-in function" (for the top-level helpers), or a builtin
 * signature starting with "$".
 */
function tarnItems(
  list: vscode.CompletionList,
  kind: "env" | "capture" | "builtin",
): vscode.CompletionItem[] {
  return list.items.filter((item) => {
    const detail = typeof item.detail === "string" ? item.detail : "";
    if (kind === "env") {
      return detail.startsWith("env.");
    }
    if (kind === "capture") {
      return detail.startsWith("capture from");
    }
    // builtin
    return (
      detail.startsWith("$") &&
      (typeof item.label === "string" ? item.label : item.label.label).startsWith("$")
    );
  });
}

function labels(items: vscode.CompletionItem[]): string[] {
  return items
    .map((i) => (typeof i.label === "string" ? i.label : i.label.label))
    .sort();
}

describe("TarnCompletionProvider", () => {
  const createdFiles: vscode.Uri[] = [];

  before(async function () {
    this.timeout(60000);
    await ensureActivated();
  });

  afterEach(() => {
    for (const uri of createdFiles) {
      try {
        fs.unlinkSync(uri.fsPath);
      } catch {
        /* ignore */
      }
    }
    createdFiles.length = 0;
  });

  it("offers env keys from tarn.config.yaml inside `{{ env. }}`", async function () {
    this.timeout(15000);
    const uri = writeFixture(
      "completion-env.tarn.yaml",
      `name: Env completion
tests:
  t:
    steps:
      - name: ping
        request:
          method: GET
          url: "{{ env. }}/health"
        assert:
          status: 200
`,
    );
    createdFiles.push(uri);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);

    // Cursor goes right after the "." in "{{ env." so the prefix is
    // empty and every env key is offered.
    const urlLine = doc
      .getText()
      .split("\n")
      .findIndex((l) => l.includes("{{ env."));
    assert.ok(urlLine >= 0);
    const line = doc.lineAt(urlLine);
    const column = line.text.indexOf("{{ env.") + "{{ env.".length;

    const list = await completionsAt(uri, urlLine, column, ".");
    const envLabels = labels(tarnItems(list, "env"));
    assert.ok(
      envLabels.includes("base_url"),
      `expected base_url in env completions, got: ${envLabels.join(",")}`,
    );
    assert.ok(
      envLabels.includes("api_token"),
      `expected api_token in env completions, got: ${envLabels.join(",")}`,
    );
  });

  it("offers captures from earlier steps inside `{{ capture. }}`", async function () {
    this.timeout(15000);
    const uri = writeFixture(
      "completion-capture.tarn.yaml",
      `name: Capture completion
tests:
  crud:
    steps:
      - name: login
        request:
          method: POST
          url: "http://localhost/auth"
        capture:
          auth_token: "$.token"
      - name: create
        request:
          method: POST
          url: "http://localhost/users"
        capture:
          user_id: "$.id"
      - name: fetch
        request:
          method: GET
          url: "http://localhost/users/{{ capture. }}"
        assert:
          status: 200
`,
    );
    createdFiles.push(uri);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);

    const urlLine = doc
      .getText()
      .split("\n")
      .findIndex((l) => l.includes("{{ capture."));
    const column =
      doc.lineAt(urlLine).text.indexOf("{{ capture.") + "{{ capture.".length;

    const list = await completionsAt(uri, urlLine, column, ".");
    const captureLabels = labels(tarnItems(list, "capture"));
    assert.deepStrictEqual(
      captureLabels,
      ["auth_token", "user_id"],
      `expected auth_token and user_id, got: ${captureLabels.join(",")}`,
    );
  });

  it("does not offer captures from later steps or other tests", async function () {
    this.timeout(15000);
    const uri = writeFixture(
      "completion-capture-scope.tarn.yaml",
      `name: Capture scope
tests:
  first:
    steps:
      - name: only_step
        request:
          method: GET
          url: "http://localhost/{{ capture. }}"
        capture:
          late_capture: "$.id"
  second:
    steps:
      - name: other_step
        request:
          method: GET
          url: "http://localhost/other"
        capture:
          unrelated: "$.data"
`,
    );
    createdFiles.push(uri);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);

    const urlLine = doc
      .getText()
      .split("\n")
      .findIndex((l) => l.includes("{{ capture."));
    const column =
      doc.lineAt(urlLine).text.indexOf("{{ capture.") + "{{ capture.".length;

    const list = await completionsAt(uri, urlLine, column, ".");
    const captureLabels = labels(tarnItems(list, "capture"));
    assert.ok(
      !captureLabels.includes("late_capture"),
      `late_capture (same step) should not be offered: ${captureLabels.join(",")}`,
    );
    assert.ok(
      !captureLabels.includes("unrelated"),
      `unrelated (other test) should not be offered: ${captureLabels.join(",")}`,
    );
  });

  it("offers builtin functions inside `{{ $... }}`", async function () {
    this.timeout(15000);
    const uri = writeFixture(
      "completion-builtin.tarn.yaml",
      `name: Builtin completion
tests:
  t:
    steps:
      - name: ping
        request:
          method: GET
          url: "http://localhost/?id={{ $ }}"
        assert:
          status: 200
`,
    );
    createdFiles.push(uri);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);

    const urlLine = doc
      .getText()
      .split("\n")
      .findIndex((l) => l.includes("{{ $"));
    const column = doc.lineAt(urlLine).text.indexOf("{{ $") + "{{ $".length;

    const list = await completionsAt(uri, urlLine, column, "$");
    const builtinLabels = labels(tarnItems(list, "builtin"));
    for (const expected of [
      "$now_iso",
      "$random_hex",
      "$random_int",
      "$timestamp",
      "$uuid",
    ]) {
      assert.ok(
        builtinLabels.includes(expected),
        `expected ${expected} in builtin completions, got: ${builtinLabels.join(",")}`,
      );
    }
  });

  it("does not offer completions outside any interpolation", async function () {
    this.timeout(15000);
    const uri = writeFixture(
      "completion-outside.tarn.yaml",
      `name: Outside
tests:
  t:
    steps:
      - name: ping
        request:
          method: GET
          url: "http://localhost/plain"
`,
    );
    createdFiles.push(uri);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);

    const urlLine = doc
      .getText()
      .split("\n")
      .findIndex((l) => l.includes("http://localhost/plain"));
    const column = doc.lineAt(urlLine).text.indexOf("plain") + "plain".length;

    const list = await completionsAt(uri, urlLine, column);
    // Only items from other providers (YAML schema completions, etc.)
    // should appear — our provider returns nothing. Verify no item has
    // a detail string starting with "env." or "capture from" which
    // would indicate our provider fired.
    for (const item of list.items) {
      const detail = typeof item.detail === "string" ? item.detail : "";
      assert.ok(
        !detail.startsWith("env.") && !detail.startsWith("capture from"),
        `unexpected Tarn completion outside interpolation: ${detail}`,
      );
    }
  });
});
