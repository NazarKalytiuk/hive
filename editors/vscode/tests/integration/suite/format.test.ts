import * as assert from "assert";
import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

const EXTENSION_ID = "nazarkalytiuk.tarn-vscode";

interface TarnExtensionApiShape {
  readonly testing: {
    readonly formatDocument: (uri: vscode.Uri) => Promise<vscode.TextEdit[]>;
  };
}

async function getApi(): Promise<TarnExtensionApiShape> {
  const ext = vscode.extensions.getExtension<TarnExtensionApiShape>(EXTENSION_ID);
  assert.ok(ext, `extension ${EXTENSION_ID} not found`);
  const api = await ext!.activate();
  assert.ok(api, "extension activated but returned no API");
  return api;
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

describe("TarnFormatProvider (tarn fmt)", () => {
  const createdFiles: vscode.Uri[] = [];
  let api: TarnExtensionApiShape;

  before(async function () {
    this.timeout(60000);
    api = await getApi();
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

  it("normalizes non-canonical indentation and quoting", async function () {
    this.timeout(20000);
    const uri = writeFixture(
      "format-messy.tarn.yaml",
      `name:    "Messy fixture"
tests:
  t:
    steps:
      -    name:   "step"
           request:
               method:   GET
               url:  "http://localhost/"
`,
    );
    createdFiles.push(uri);

    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);

    const edits = await api.testing.formatDocument(uri);
    assert.strictEqual(
      edits.length,
      1,
      `expected a single full-document edit, got ${edits.length}`,
    );

    const wholeRange = new vscode.Range(
      doc.positionAt(0),
      doc.positionAt(doc.getText().length),
    );
    assert.ok(
      edits[0].range.isEqual(wholeRange),
      "edit should span the whole document",
    );
    // The replacement must be a valid canonical form: no 4-space
    // indents, no quotes around plain scalars.
    const formatted = edits[0].newText;
    assert.ok(
      formatted.includes("name: Messy fixture"),
      `expected canonical unquoted name, got:\n${formatted}`,
    );
    assert.ok(
      formatted.includes("    - name: step"),
      `expected canonical step indent, got:\n${formatted}`,
    );
    assert.ok(
      !formatted.includes("        method:   GET"),
      `non-canonical spacing should be removed, got:\n${formatted}`,
    );
  });

  it("returns no edits when the file is already canonical", async function () {
    this.timeout(20000);
    // Tarn's canonical form puts the list dash at the same indent as
    // `steps:` rather than 2 spaces further in, so this fixture is
    // already a fixed point for `tarn fmt`.
    const uri = writeFixture(
      "format-canonical.tarn.yaml",
      `name: Already canonical
tests:
  t:
    steps:
    - name: ping
      request:
        method: GET
        url: http://localhost/
`,
    );
    createdFiles.push(uri);

    await vscode.workspace.openTextDocument(uri);
    const edits = await api.testing.formatDocument(uri);
    assert.strictEqual(
      edits.length,
      0,
      `canonical file should produce no edits, got ${edits.length}`,
    );
  });

  it("leaves the buffer untouched when the file has YAML parse errors", async function () {
    this.timeout(20000);
    const uri = writeFixture(
      "format-broken.tarn.yaml",
      `name: "Unclosed
tests:
  t:
    steps:
      - name: s
`,
    );
    createdFiles.push(uri);

    await vscode.workspace.openTextDocument(uri);
    const edits = await api.testing.formatDocument(uri);
    assert.strictEqual(
      edits.length,
      0,
      `invalid file should produce no edits, got ${edits.length}`,
    );
    // Original content on disk unchanged.
    const onDisk = fs.readFileSync(uri.fsPath, "utf8");
    assert.ok(
      onDisk.startsWith(`name: "Unclosed`),
      `broken file should not have been rewritten, got:\n${onDisk}`,
    );
  });
});
