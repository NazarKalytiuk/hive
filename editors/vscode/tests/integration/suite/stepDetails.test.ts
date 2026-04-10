import * as assert from "assert";
import * as vscode from "vscode";

const EXTENSION_ID = "nazarkalytiuk.tarn-vscode";

interface StepKey {
  file: string;
  test: string;
  stepIndex: number;
}

interface TarnExtensionApiShape {
  readonly commands: readonly string[];
  readonly testing: {
    readonly loadLastRunFromReport: (report: unknown) => void;
    readonly lastRunCacheSize: () => number;
    readonly showStepDetails: (key: StepKey) => boolean;
  };
}

async function getApi(): Promise<TarnExtensionApiShape> {
  const ext = vscode.extensions.getExtension<TarnExtensionApiShape>(EXTENSION_ID);
  assert.ok(ext, `extension ${EXTENSION_ID} not found`);
  const api = await ext!.activate();
  assert.ok(api, "extension activated but returned no API");
  return api;
}

/** Minimal report fixture used to prime the cache without a real run. */
const sampleReport = {
  schema_version: 1,
  version: "1",
  timestamp: "2026-04-10T12:00:00Z",
  duration_ms: 200,
  files: [
    {
      file: "tests/users.tarn.yaml",
      name: "Users",
      status: "FAILED",
      duration_ms: 200,
      summary: { total: 2, passed: 1, failed: 1 },
      setup: [
        { name: "Auth", status: "PASSED", duration_ms: 10 },
      ],
      tests: [
        {
          name: "create_user",
          description: null,
          status: "FAILED",
          duration_ms: 190,
          steps: [
            { name: "POST /users", status: "PASSED", duration_ms: 100 },
            {
              name: "GET /users/1",
              status: "FAILED",
              duration_ms: 90,
              failure_category: "assertion_failed",
              error_code: "assertion_mismatch",
              assertions: {
                total: 1,
                passed: 0,
                failed: 1,
                failures: [
                  {
                    assertion: "status",
                    passed: false,
                    expected: "200",
                    actual: "500",
                    message: "unexpected status",
                  },
                ],
              },
              request: {
                method: "GET",
                url: "http://localhost/users/1",
                headers: { accept: "application/json" },
                body: undefined,
              },
              response: {
                status: 500,
                headers: { "content-type": "application/json" },
                body: { error: "server blew up" },
              },
            },
          ],
        },
      ],
      teardown: [],
    },
  ],
  summary: {
    files: 1,
    tests: 1,
    steps: { total: 2, passed: 1, failed: 1 },
    status: "FAILED",
  },
};

describe("RequestResponsePanel (tarn.showStepDetails)", () => {
  let api: TarnExtensionApiShape;

  before(async function () {
    this.timeout(60000);
    api = await getApi();
    api.testing.loadLastRunFromReport(sampleReport);
  });

  it("registers the tarn.showStepDetails command", async () => {
    const commands = await vscode.commands.getCommands(true);
    assert.ok(
      commands.includes("tarn.showStepDetails"),
      "tarn.showStepDetails should be registered",
    );
  });

  it("primes the last-run cache with every step", () => {
    // setup (1) + test steps (2) + teardown (0) = 3
    assert.strictEqual(api.testing.lastRunCacheSize(), 3);
  });

  it("showStepDetails opens the panel for a known step key", () => {
    const opened = api.testing.showStepDetails({
      file: "tests/users.tarn.yaml",
      test: "create_user",
      stepIndex: 1,
    });
    assert.strictEqual(opened, true, "showStepDetails should return true for a known key");
  });

  it("showStepDetails returns false for unknown step keys", () => {
    const opened = api.testing.showStepDetails({
      file: "tests/does-not-exist.tarn.yaml",
      test: "nothing",
      stepIndex: 0,
    });
    assert.strictEqual(opened, false);
  });

  it("invoking the tarn.showStepDetails command via executeCommand does not throw", async function () {
    this.timeout(10000);
    // The command is registered with a `when: false` commandPalette
    // gate, but executeCommand ignores the gate, so we can still
    // drive it directly to verify end-to-end wiring. Argument is a
    // known-good StepKey primed into the cache above.
    await vscode.commands.executeCommand("tarn.showStepDetails", {
      file: "tests/users.tarn.yaml",
      test: "create_user",
      stepIndex: 1,
    });
  });
});
