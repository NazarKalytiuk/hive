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

async function hoverAt(
  uri: vscode.Uri,
  line: number,
  character: number,
): Promise<vscode.Hover[]> {
  const position = new vscode.Position(line, character);
  return (
    (await vscode.commands.executeCommand(
      "vscode.executeHoverProvider",
      uri,
      position,
    )) as vscode.Hover[] | undefined
  ) ?? [];
}

/** Concatenate every markdown-ish block returned by every provider on
 * a hover into a single string, lowercased, so tests can search it
 * without worrying about Tarn-vs-YAML provider ordering. */
function flattenHoverText(hovers: vscode.Hover[]): string {
  const parts: string[] = [];
  for (const hover of hovers) {
    for (const content of hover.contents) {
      if (typeof content === "string") {
        parts.push(content);
      } else if ("value" in content) {
        parts.push(content.value);
      }
    }
  }
  return parts.join("\n").toLowerCase();
}

describe("TarnHoverProvider", () => {
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

  it("shows declaring environments and values for {{ env.base_url }}", async function () {
    this.timeout(15000);
    const uri = writeFixture(
      "hover-env.tarn.yaml",
      `name: Hover env
tests:
  t:
    steps:
      - name: ping
        request:
          method: GET
          url: "{{ env.base_url }}/health"
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
      .findIndex((l) => l.includes("{{ env.base_url }}"));
    assert.ok(urlLine >= 0);
    const column = doc.lineAt(urlLine).text.indexOf("base_url") + 2;

    const hovers = await hoverAt(uri, urlLine, column);
    const text = flattenHoverText(hovers);
    assert.ok(
      text.includes("env.base_url"),
      `expected 'env.base_url' in hover, got: ${text}`,
    );
    assert.ok(
      text.includes("staging") && text.includes("production"),
      `expected both environments in hover, got: ${text}`,
    );
    assert.ok(
      text.includes("https://staging.example.com") ||
        text.includes("https://prod.example.com"),
      `expected a concrete base_url value in hover, got: ${text}`,
    );
  });

  it("shows the capturing step for {{ capture.x }}", async function () {
    this.timeout(15000);
    const uri = writeFixture(
      "hover-capture.tarn.yaml",
      `name: Hover capture
tests:
  crud:
    steps:
      - name: login
        request:
          method: POST
          url: "http://localhost/auth"
        capture:
          auth_token: "$.token"
      - name: fetch
        request:
          method: GET
          url: "http://localhost/users/{{ capture.auth_token }}"
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
      .findIndex((l) => l.includes("{{ capture.auth_token }}"));
    const column = doc.lineAt(urlLine).text.indexOf("auth_token") + 4;

    const hovers = await hoverAt(uri, urlLine, column);
    const text = flattenHoverText(hovers);
    assert.ok(
      text.includes("capture.auth_token"),
      `expected 'capture.auth_token' in hover, got: ${text}`,
    );
    assert.ok(
      text.includes("login"),
      `expected capturing step 'login' in hover, got: ${text}`,
    );
  });

  it("explains missing captures when the name is not in scope", async function () {
    this.timeout(15000);
    const uri = writeFixture(
      "hover-missing-capture.tarn.yaml",
      `name: Hover missing
tests:
  t:
    steps:
      - name: only_step
        request:
          method: GET
          url: "http://localhost/{{ capture.nonexistent }}"
`,
    );
    createdFiles.push(uri);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);

    const urlLine = doc
      .getText()
      .split("\n")
      .findIndex((l) => l.includes("{{ capture.nonexistent }}"));
    const column = doc.lineAt(urlLine).text.indexOf("nonexistent") + 2;

    const hovers = await hoverAt(uri, urlLine, column);
    const text = flattenHoverText(hovers);
    assert.ok(
      text.includes("not captured") || text.includes("not in scope"),
      `expected a 'not captured' warning, got: ${text}`,
    );
  });

  it("shows signature and doc for {{ $uuid }}", async function () {
    this.timeout(15000);
    const uri = writeFixture(
      "hover-builtin.tarn.yaml",
      `name: Hover builtin
tests:
  t:
    steps:
      - name: ping
        request:
          method: GET
          url: "http://localhost/?id={{ $uuid }}"
`,
    );
    createdFiles.push(uri);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);

    const urlLine = doc
      .getText()
      .split("\n")
      .findIndex((l) => l.includes("{{ $uuid }}"));
    const column = doc.lineAt(urlLine).text.indexOf("uuid") + 2;

    const hovers = await hoverAt(uri, urlLine, column);
    const text = flattenHoverText(hovers);
    assert.ok(text.includes("$uuid"), `expected '$uuid' in hover, got: ${text}`);
    assert.ok(
      text.includes("uuid v4") || text.includes("random uuid"),
      `expected UUID v4 doc in hover, got: ${text}`,
    );
  });

  it("shows signature for parameterized builtin $random_hex", async function () {
    this.timeout(15000);
    const uri = writeFixture(
      "hover-random-hex.tarn.yaml",
      `name: Hover random hex
tests:
  t:
    steps:
      - name: ping
        request:
          method: GET
          url: "http://localhost/?id={{ $random_hex(8) }}"
`,
    );
    createdFiles.push(uri);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);

    const urlLine = doc
      .getText()
      .split("\n")
      .findIndex((l) => l.includes("{{ $random_hex(8) }}"));
    const column = doc.lineAt(urlLine).text.indexOf("random_hex") + 3;

    const hovers = await hoverAt(uri, urlLine, column);
    const text = flattenHoverText(hovers);
    assert.ok(
      text.includes("$random_hex(n)"),
      `expected signature '$random_hex(n)' in hover, got: ${text}`,
    );
  });

  it("does not surface a Tarn hover outside any interpolation", async function () {
    this.timeout(15000);
    const uri = writeFixture(
      "hover-outside.tarn.yaml",
      `name: Hover outside
tests:
  t:
    steps:
      - name: plain
        request:
          method: GET
          url: "http://localhost/static"
`,
    );
    createdFiles.push(uri);
    const doc = await vscode.workspace.openTextDocument(uri);
    await vscode.window.showTextDocument(doc);

    const urlLine = doc
      .getText()
      .split("\n")
      .findIndex((l) => l.includes("http://localhost/static"));
    const column = doc.lineAt(urlLine).text.indexOf("static") + 2;

    const hovers = await hoverAt(uri, urlLine, column);
    const text = flattenHoverText(hovers);
    // Hovers from other providers (YAML schema, etc.) are OK; just
    // assert that nothing from our provider leaked in. Our provider
    // always contains the substring "tarn interpolation" for the
    // top-level help, `env.` for env hovers, `capture.` for capture
    // hovers, or the backtick-wrapped `$name` header for builtins.
    assert.ok(
      !text.includes("tarn interpolation"),
      `unexpected Tarn hover help outside interpolation: ${text}`,
    );
    assert.ok(
      !/`env\.[a-z_]/i.test(text),
      `unexpected env hover outside interpolation: ${text}`,
    );
    assert.ok(
      !/`capture\.[a-z_]/i.test(text),
      `unexpected capture hover outside interpolation: ${text}`,
    );
  });
});
