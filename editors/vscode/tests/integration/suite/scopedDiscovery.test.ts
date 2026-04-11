import * as assert from "assert";
import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

const EXTENSION_ID = "nazarkalytiuk.tarn-vscode";

interface IndexSnapshotEntry {
  readonly uri: string;
  readonly fileName: string;
  readonly tests: ReadonlyArray<{ readonly name: string; readonly stepCount: number }>;
  readonly fromScopedList: boolean;
}

interface TarnExtensionApiShape {
  readonly testing: {
    readonly workspaceIndexSnapshot: () => ReadonlyArray<IndexSnapshotEntry>;
    readonly refreshSingleFile: (uri: vscode.Uri) => Promise<void>;
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

function findEntry(
  snapshot: ReadonlyArray<IndexSnapshotEntry>,
  uri: vscode.Uri,
): IndexSnapshotEntry | undefined {
  return snapshot.find((entry) => entry.uri === uri.toString());
}

describe("Scoped discovery via tarn list --file (NAZ-282)", () => {
  // These tests exercise the incremental WorkspaceIndex path which
  // asks `tarn list --file <path>` for the authoritative structure
  // of a single file and compares it against the cached entry. The
  // goal is to prove that (a) the structure the extension caches
  // matches Tarn's post-parse view, and (b) a mutation on one file
  // never churns another file's TestItem tree.
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

  it("discovers a newly created file via the scoped tarn list path", async function () {
    this.timeout(20000);
    const uri = writeFixture(
      "scoped-add.tarn.yaml",
      [
        'version: "1"',
        'name: "Scoped add fixture"',
        "tests:",
        "  alpha:",
        "    description: first test",
        "    steps:",
        "      - name: GET /alpha",
        "        request:",
        "          method: GET",
        '          url: "https://example.invalid/alpha"',
        "        assert:",
        "          status: 200",
        "",
      ].join("\n"),
    );
    createdFiles.push(uri);

    await api.testing.refreshSingleFile(uri);
    const snapshot = api.testing.workspaceIndexSnapshot();
    const entry = findEntry(snapshot, uri);
    assert.ok(entry, `created file was not indexed: ${uri.fsPath}`);
    assert.strictEqual(
      entry!.fromScopedList,
      true,
      "created file must come from scoped tarn list, not AST fallback",
    );
    assert.strictEqual(entry!.fileName, "Scoped add fixture");
    assert.strictEqual(entry!.tests.length, 1);
    assert.strictEqual(entry!.tests[0].name, "alpha");
    assert.strictEqual(entry!.tests[0].stepCount, 1);
  });

  it("reflects edits to an existing file without touching other files", async function () {
    this.timeout(25000);
    const targetUri = writeFixture(
      "scoped-edit.tarn.yaml",
      [
        'version: "1"',
        'name: "Scoped edit fixture"',
        "tests:",
        "  alpha:",
        "    steps:",
        "      - name: GET /a",
        "        request:",
        "          method: GET",
        '          url: "https://example.invalid/a"',
        "        assert:",
        "          status: 200",
        "",
      ].join("\n"),
    );
    createdFiles.push(targetUri);
    const siblingUri = writeFixture(
      "scoped-edit-sibling.tarn.yaml",
      [
        'version: "1"',
        'name: "Sibling idle"',
        "tests:",
        "  sibling:",
        "    steps:",
        "      - name: GET /sibling",
        "        request:",
        "          method: GET",
        '          url: "https://example.invalid/sibling"',
        "        assert:",
        "          status: 200",
        "",
      ].join("\n"),
    );
    createdFiles.push(siblingUri);

    await api.testing.refreshSingleFile(targetUri);
    await api.testing.refreshSingleFile(siblingUri);
    const beforeSnapshot = api.testing.workspaceIndexSnapshot();
    const siblingBefore = findEntry(beforeSnapshot, siblingUri);
    assert.ok(siblingBefore, "sibling must be indexed before the edit");
    const siblingSnapshotBefore = {
      fileName: siblingBefore!.fileName,
      tests: siblingBefore!.tests.map((t) => ({
        name: t.name,
        stepCount: t.stepCount,
      })),
      fromScopedList: siblingBefore!.fromScopedList,
    };

    // Now rewrite the target: rename `alpha` to `alpha_renamed`
    // and add a second step. Only this file's entry should change.
    fs.writeFileSync(
      targetUri.fsPath,
      [
        'version: "1"',
        'name: "Scoped edit fixture"',
        "tests:",
        "  alpha_renamed:",
        "    steps:",
        "      - name: GET /a",
        "        request:",
        "          method: GET",
        '          url: "https://example.invalid/a"',
        "        assert:",
        "          status: 200",
        "      - name: GET /a2",
        "        request:",
        "          method: GET",
        '          url: "https://example.invalid/a2"',
        "        assert:",
        "          status: 200",
        "",
      ].join("\n"),
      "utf8",
    );
    await api.testing.refreshSingleFile(targetUri);

    const afterSnapshot = api.testing.workspaceIndexSnapshot();
    const targetAfter = findEntry(afterSnapshot, targetUri);
    assert.ok(targetAfter, "target must still be indexed after the edit");
    assert.strictEqual(targetAfter!.tests.length, 1);
    assert.strictEqual(targetAfter!.tests[0].name, "alpha_renamed");
    assert.strictEqual(targetAfter!.tests[0].stepCount, 2);
    assert.strictEqual(targetAfter!.fromScopedList, true);

    const siblingAfter = findEntry(afterSnapshot, siblingUri);
    assert.ok(siblingAfter, "sibling must still be indexed after the edit");
    assert.deepStrictEqual(
      {
        fileName: siblingAfter!.fileName,
        tests: siblingAfter!.tests.map((t) => ({
          name: t.name,
          stepCount: t.stepCount,
        })),
        fromScopedList: siblingAfter!.fromScopedList,
      },
      siblingSnapshotBefore,
      "sibling structure must be unchanged when another file is edited",
    );
  });

  it("drops a file from the index when it is deleted", async function () {
    this.timeout(20000);
    const uri = writeFixture(
      "scoped-delete.tarn.yaml",
      [
        'version: "1"',
        'name: "Scoped delete fixture"',
        "tests:",
        "  alpha:",
        "    steps:",
        "      - name: GET /a",
        "        request:",
        "          method: GET",
        '          url: "https://example.invalid/a"',
        "        assert:",
        "          status: 200",
        "",
      ].join("\n"),
    );
    createdFiles.push(uri);

    await api.testing.refreshSingleFile(uri);
    let snapshot = api.testing.workspaceIndexSnapshot();
    assert.ok(
      findEntry(snapshot, uri),
      "file must be indexed before deletion",
    );

    fs.unlinkSync(uri.fsPath);
    // Drop from createdFiles so afterEach does not double-unlink.
    const idx = createdFiles.findIndex((u) => u.fsPath === uri.fsPath);
    if (idx !== -1) {
      createdFiles.splice(idx, 1);
    }

    await api.testing.refreshSingleFile(uri);
    snapshot = api.testing.workspaceIndexSnapshot();
    assert.strictEqual(
      findEntry(snapshot, uri),
      undefined,
      "deleted file must be removed from the index",
    );
  });

  it("handles a rename as delete-old + add-new", async function () {
    this.timeout(25000);
    const oldUri = writeFixture(
      "scoped-rename-old.tarn.yaml",
      [
        'version: "1"',
        'name: "Before rename"',
        "tests:",
        "  alpha:",
        "    steps:",
        "      - name: GET /a",
        "        request:",
        "          method: GET",
        '          url: "https://example.invalid/a"',
        "        assert:",
        "          status: 200",
        "",
      ].join("\n"),
    );
    createdFiles.push(oldUri);
    await api.testing.refreshSingleFile(oldUri);

    // Native rename: move the file on disk, then notify the index
    // about both the old path (delete) and the new path (create).
    // The extension's FileSystemWatcher plumbs this to
    // `refreshSingleFile` in production — we invoke it directly
    // here to avoid racing the native watcher.
    const newPath = path.join(workspaceRoot(), "scoped-rename-new.tarn.yaml");
    fs.renameSync(oldUri.fsPath, newPath);
    const newUri = vscode.Uri.file(newPath);
    createdFiles.push(newUri);

    await api.testing.refreshSingleFile(oldUri);
    await api.testing.refreshSingleFile(newUri);

    // The old entry must have been dropped from the index.
    const snapshot = api.testing.workspaceIndexSnapshot();
    assert.strictEqual(
      findEntry(snapshot, oldUri),
      undefined,
      "renamed-away entry must be dropped from the index",
    );
    const newEntry = findEntry(snapshot, newUri);
    assert.ok(newEntry, "renamed-in entry must be indexed");
    assert.strictEqual(newEntry!.fileName, "Before rename");
    assert.strictEqual(newEntry!.tests.length, 1);
    assert.strictEqual(newEntry!.tests[0].name, "alpha");
    assert.strictEqual(newEntry!.fromScopedList, true);

    // Clean up: afterEach drops `newUri`; the old path is already
    // gone because of the rename, so drop it from createdFiles so
    // afterEach does not attempt to unlink a non-existent file.
    const oldIdx = createdFiles.findIndex((u) => u.fsPath === oldUri.fsPath);
    if (oldIdx !== -1) {
      createdFiles.splice(oldIdx, 1);
    }
  });

  it("keeps scoped discovery enabled across a per-file parse error", async function () {
    this.timeout(25000);
    // A YAML that Tarn rejects at parse time (for example, a file
    // with neither `steps:` nor `tests:`) should NOT disable
    // scoped discovery for the rest of the session — the binary
    // still works fine, Tarn just cannot interpret this specific
    // file. After the broken file is indexed via the AST
    // fallback, a subsequent well-formed file must still come
    // back via the scoped path.
    const brokenUri = writeFixture(
      "scoped-broken.tarn.yaml",
      [
        'version: "1"',
        'name: "Broken fixture (no steps and no tests)"',
        "setup:",
        "  - name: only_a_setup",
        "    request:",
        "      method: GET",
        '      url: "https://example.invalid/boot"',
        "    assert:",
        "      status: 200",
        "",
      ].join("\n"),
    );
    createdFiles.push(brokenUri);
    await api.testing.refreshSingleFile(brokenUri);
    const afterBroken = api.testing.workspaceIndexSnapshot();
    // The broken file must still be indexed via the AST fallback —
    // Tarn rejected it but it is a real file the user can still
    // open in the Test Explorer tree.
    assert.ok(
      findEntry(afterBroken, brokenUri),
      "broken file must still be indexed via the AST fallback",
    );

    const goodUri = writeFixture(
      "scoped-good-after-broken.tarn.yaml",
      [
        'version: "1"',
        'name: "Good after broken"',
        "tests:",
        "  alpha:",
        "    steps:",
        "      - name: GET /alpha",
        "        request:",
        "          method: GET",
        '          url: "https://example.invalid/alpha"',
        "        assert:",
        "          status: 200",
        "",
      ].join("\n"),
    );
    createdFiles.push(goodUri);
    await api.testing.refreshSingleFile(goodUri);
    const afterGood = api.testing.workspaceIndexSnapshot();
    const goodEntry = findEntry(afterGood, goodUri);
    assert.ok(goodEntry, "well-formed fixture must be indexed after a broken one");
    assert.strictEqual(
      goodEntry!.fromScopedList,
      true,
      "scoped discovery must remain enabled after a per-file parse error",
    );
    assert.strictEqual(goodEntry!.fileName, "Good after broken");
    assert.strictEqual(goodEntry!.tests.length, 1);
    assert.strictEqual(goodEntry!.tests[0].name, "alpha");
    assert.strictEqual(goodEntry!.tests[0].stepCount, 1);
  });
});
