import * as assert from "assert";
import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

const EXTENSION_ID = "nazarkalytiuk.tarn-vscode";

interface TarnExtensionApiShape {
  readonly testing: {
    readonly validateDocument: (uri: vscode.Uri) => Promise<void>;
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

async function updateSetting<T>(
  key: string,
  value: T,
): Promise<void> {
  await vscode.workspace
    .getConfiguration()
    .update(key, value, vscode.ConfigurationTarget.Workspace);
}

describe("DiagnosticsProvider: validate-on-save", () => {
  let api: TarnExtensionApiShape;
  let createdFiles: vscode.Uri[] = [];

  before(async function () {
    this.timeout(60000);
    api = await getApi();
  });

  afterEach(async () => {
    for (const uri of createdFiles) {
      try {
        fs.unlinkSync(uri.fsPath);
      } catch {
        /* ignore */
      }
    }
    createdFiles = [];
    await updateSetting("tarn.validateOnSave", undefined);
  });

  it("publishes no diagnostics for a valid file", async function () {
    this.timeout(30000);
    const uri = writeFixture(
      "diag-ok.tarn.yaml",
      `version: "1"
name: Valid fixture
tests:
  smoke:
    steps:
      - name: ping
        request:
          method: GET
          url: "http://127.0.0.1:0/"
        assert:
          status: 200
`,
    );
    createdFiles.push(uri);

    await api.testing.validateDocument(uri);
    const diagnostics = vscode.languages.getDiagnostics(uri);
    assert.strictEqual(
      diagnostics.length,
      0,
      `expected no diagnostics, got: ${JSON.stringify(diagnostics)}`,
    );
  });

  it("publishes a diagnostic with line/column for a YAML syntax error", async function () {
    this.timeout(30000);
    const uri = writeFixture(
      "diag-bad.tarn.yaml",
      `name: "Broken
tests:
  t:
    steps:
      - name: unclosed
`,
    );
    createdFiles.push(uri);

    await api.testing.validateDocument(uri);
    const diagnostics = vscode.languages.getDiagnostics(uri);
    assert.ok(
      diagnostics.length >= 1,
      `expected at least one diagnostic, got: ${JSON.stringify(diagnostics)}`,
    );
    const diag = diagnostics[0];
    assert.strictEqual(diag.source, "tarn");
    assert.strictEqual(diag.severity, vscode.DiagnosticSeverity.Error);
    assert.ok(
      diag.range.start.line >= 0,
      `expected non-negative line, got: ${diag.range.start.line}`,
    );
    assert.ok(
      diag.message.toLowerCase().includes("quoted") ||
        diag.message.toLowerCase().includes("stream"),
      `unexpected message: ${diag.message}`,
    );
  });

  it("publishes a diagnostic for unknown field errors", async function () {
    this.timeout(30000);
    const uri = writeFixture(
      "diag-unknown-field.tarn.yaml",
      `name: Unknown field
tests:
  t:
    steps:
      - name: bad
        requestt:
          method: GET
          url: "http://127.0.0.1:0/"
`,
    );
    createdFiles.push(uri);

    await api.testing.validateDocument(uri);
    const diagnostics = vscode.languages.getDiagnostics(uri);
    assert.ok(diagnostics.length >= 1, "expected at least one diagnostic");
    assert.ok(
      diagnostics[0].message.toLowerCase().includes("unknown field"),
      `expected unknown-field message, got: ${diagnostics[0].message}`,
    );
    assert.ok(
      diagnostics[0].message.includes("requestt"),
      `expected offending field name in message, got: ${diagnostics[0].message}`,
    );
  });

  it("clears diagnostics after the file becomes valid", async function () {
    this.timeout(30000);
    const uri = writeFixture(
      "diag-fix-cycle.tarn.yaml",
      `name: "Broken again
tests:
  t:
    steps:
      - name: s
`,
    );
    createdFiles.push(uri);

    await api.testing.validateDocument(uri);
    assert.ok(
      vscode.languages.getDiagnostics(uri).length >= 1,
      "expected broken file to produce a diagnostic first",
    );

    fs.writeFileSync(
      uri.fsPath,
      `version: "1"
name: Now valid
tests:
  t:
    steps:
      - name: ping
        request:
          method: GET
          url: "http://127.0.0.1:0/"
        assert:
          status: 200
`,
    );

    await api.testing.validateDocument(uri);
    const diagnostics = vscode.languages.getDiagnostics(uri);
    assert.strictEqual(
      diagnostics.length,
      0,
      `expected diagnostics to clear after fix, got: ${JSON.stringify(diagnostics)}`,
    );
  });

  it("respects tarn.validateOnSave = false", async function () {
    this.timeout(30000);
    await updateSetting("tarn.validateOnSave", false);

    const uri = writeFixture(
      "diag-disabled.tarn.yaml",
      `name: "Still broken
tests:
  t:
    steps:
      - name: s
`,
    );
    createdFiles.push(uri);

    await api.testing.validateDocument(uri);
    const diagnostics = vscode.languages.getDiagnostics(uri);
    assert.strictEqual(
      diagnostics.length,
      0,
      `expected no diagnostics when validateOnSave is disabled, got: ${JSON.stringify(diagnostics)}`,
    );
  });
});
