import * as assert from "assert";
import * as vscode from "vscode";

const EXTENSION_ID = "nazarkalytiuk.tarn-vscode";

interface TarnExtensionApiShape {
  readonly commands: readonly string[];
  readonly testing: {
    readonly loadCapturesFromReport: (report: unknown) => void;
    readonly capturesTotalCount: () => number;
    readonly isCaptureKeyRedacted: (key: string) => boolean;
    readonly isHidingAllCaptures: () => boolean;
    readonly toggleHideCaptures: () => void;
  };
}

async function getApi(): Promise<TarnExtensionApiShape> {
  const ext = vscode.extensions.getExtension<TarnExtensionApiShape>(EXTENSION_ID);
  assert.ok(ext, `extension ${EXTENSION_ID} not found`);
  const api = await ext!.activate();
  assert.ok(api, "extension activated but returned no API");
  return api;
}

/**
 * Minimal report fixture with two tests, one of which defines the
 * redaction-listed `auth_token` capture. Primed into the inspector
 * directly so this test does not depend on tarn actually running.
 */
const sampleReport = {
  schema_version: 1,
  version: "1",
  timestamp: "2026-04-10T12:00:00Z",
  duration_ms: 42,
  files: [
    {
      file: "tests/captures.tarn.yaml",
      name: "Captures fixture",
      status: "PASSED",
      duration_ms: 42,
      summary: { total: 2, passed: 2, failed: 0 },
      setup: [],
      tests: [
        {
          name: "login",
          description: null,
          status: "PASSED",
          duration_ms: 10,
          steps: [{ name: "POST /login", status: "PASSED", duration_ms: 10 }],
          captures: {
            auth_token: "super-secret-jwt",
            user_id: 42,
            profile: { name: "Ada", roles: ["admin", "ops"] },
          },
        },
        {
          name: "logout",
          description: null,
          status: "PASSED",
          duration_ms: 5,
          steps: [{ name: "POST /logout", status: "PASSED", duration_ms: 5 }],
          captures: {
            goodbye: true,
          },
        },
      ],
      teardown: [],
    },
  ],
  summary: {
    files: 1,
    tests: 2,
    steps: { total: 2, passed: 2, failed: 0 },
    status: "PASSED",
  },
};

describe("CapturesInspector (tarn.captures view)", () => {
  let api: TarnExtensionApiShape;

  before(async function () {
    this.timeout(60000);
    api = await getApi();
    api.testing.loadCapturesFromReport(sampleReport);
  });

  it("registers the captures-related commands", async () => {
    const commands = await vscode.commands.getCommands(true);
    assert.ok(
      commands.includes("tarn.copyCaptureValue"),
      "tarn.copyCaptureValue should be registered",
    );
    assert.ok(
      commands.includes("tarn.toggleHideCaptures"),
      "tarn.toggleHideCaptures should be registered",
    );
  });

  it("counts every captured key across every test in the report", () => {
    // login → 3 captures, logout → 1 capture.
    assert.strictEqual(api.testing.capturesTotalCount(), 4);
  });

  it("treats keys in tarn.config.yaml redaction.captures as redacted", () => {
    assert.strictEqual(api.testing.isCaptureKeyRedacted("auth_token"), true);
    assert.strictEqual(api.testing.isCaptureKeyRedacted("user_id"), false);
    assert.strictEqual(api.testing.isCaptureKeyRedacted("profile"), false);
    assert.strictEqual(api.testing.isCaptureKeyRedacted("goodbye"), false);
  });

  it("toggleHideCaptures flips the hide-all flag", () => {
    assert.strictEqual(api.testing.isHidingAllCaptures(), false);
    api.testing.toggleHideCaptures();
    assert.strictEqual(api.testing.isHidingAllCaptures(), true);
    api.testing.toggleHideCaptures();
    assert.strictEqual(api.testing.isHidingAllCaptures(), false);
  });

  it("tarn.toggleHideCaptures command wires through to the view", async () => {
    assert.strictEqual(api.testing.isHidingAllCaptures(), false);
    await vscode.commands.executeCommand("tarn.toggleHideCaptures");
    assert.strictEqual(api.testing.isHidingAllCaptures(), true);
    await vscode.commands.executeCommand("tarn.toggleHideCaptures");
    assert.strictEqual(api.testing.isHidingAllCaptures(), false);
  });

  it("tarn.copyCaptureValue writes the passed value to the clipboard", async () => {
    await vscode.commands.executeCommand(
      "tarn.copyCaptureValue",
      "hello-world",
      "login.greeting",
    );
    const contents = await vscode.env.clipboard.readText();
    assert.strictEqual(contents, "hello-world");
  });

  it("reloading with a report containing no captures empties the view", () => {
    api.testing.loadCapturesFromReport({
      schema_version: 1,
      version: "1",
      duration_ms: 0,
      files: [],
      summary: {
        files: 0,
        tests: 0,
        steps: { total: 0, passed: 0, failed: 0 },
        status: "PASSED",
      },
    });
    assert.strictEqual(api.testing.capturesTotalCount(), 0);
    // Re-prime so later tests in the run see the original state.
    api.testing.loadCapturesFromReport(sampleReport);
    assert.strictEqual(api.testing.capturesTotalCount(), 4);
  });
});
