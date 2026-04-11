import * as assert from "assert";
import * as cp from "child_process";
import * as fs from "fs";
import * as net from "net";
import * as path from "path";
import * as vscode from "vscode";

const EXTENSION_ID = "nazarkalytiuk.tarn-vscode";
const FIXTURE_FILE = "tests/cookie-jar.tarn.yaml";

// Redeclared minimal shapes — the integration suite is a closed
// compilation unit (rootDir = tests/integration/) and cannot import
// from src/. These types only need to cover the fields the test reads.

interface ReportTestSummary {
  name: string;
  status: "PASSED" | "FAILED";
}

interface ReportFile {
  file: string;
  status: "PASSED" | "FAILED";
  summary: { total: number; passed: number; failed: number };
  tests: ReportTestSummary[];
}

interface Report {
  summary: {
    files: number;
    tests: number;
    steps: { total: number; passed: number; failed: number };
    status: "PASSED" | "FAILED";
  };
  files: ReportFile[];
}

interface RunOptions {
  files: string[];
  cwd: string;
  vars?: Record<string, string>;
  token: vscode.CancellationToken;
}

interface RunOutcome {
  report: Report | undefined;
  exitCode: number | null;
  stdout: string;
  stderr: string;
  cancelled: boolean;
}

interface TarnBackendShape {
  run(options: RunOptions): Promise<RunOutcome>;
}

interface TarnExtensionApiShape {
  readonly testing: { readonly backend: TarnBackendShape };
}

function workspaceRoot(): string {
  const folder = vscode.workspace.workspaceFolders?.[0];
  if (!folder) {
    throw new Error("no workspace folder available in the test host");
  }
  return folder.uri.fsPath;
}

async function getApi(): Promise<TarnExtensionApiShape> {
  const ext = vscode.extensions.getExtension<TarnExtensionApiShape>(EXTENSION_ID);
  assert.ok(ext, `extension ${EXTENSION_ID} not found`);
  const api = await ext!.activate();
  assert.ok(api, "extension activated but returned no API");
  return api;
}

async function updateSetting<T>(
  key: string,
  value: T | undefined,
): Promise<void> {
  await vscode.workspace
    .getConfiguration()
    .update(key, value, vscode.ConfigurationTarget.Workspace);
}

/**
 * Ask the OS for a free TCP port by binding ephemeral and releasing.
 * There is a microscopic race window between release and demo-server
 * binding, which is acceptable for local test runs — CI only ever has
 * one integration run in flight.
 */
function allocateFreePort(): Promise<number> {
  return new Promise((resolve, reject) => {
    const server = net.createServer();
    server.unref();
    server.on("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const address = server.address();
      if (address === null || typeof address === "string") {
        server.close(() => reject(new Error("expected TCP address info")));
        return;
      }
      const port = address.port;
      server.close(() => resolve(port));
    });
  });
}

/**
 * Poll the demo server's /health endpoint until it responds or we
 * give up. Keeps the test tolerant of the few-hundred-millisecond
 * warm-up on slower CI boxes.
 */
async function waitForServerReady(port: number, timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  let lastErr: unknown;
  while (Date.now() < deadline) {
    try {
      await new Promise<void>((resolve, reject) => {
        const socket = net.createConnection({ port, host: "127.0.0.1" });
        socket.once("connect", () => {
          socket.end();
          resolve();
        });
        socket.once("error", (err) => {
          socket.destroy();
          reject(err);
        });
      });
      return;
    } catch (err) {
      lastErr = err;
      await new Promise((r) => setTimeout(r, 50));
    }
  }
  throw new Error(
    `demo-server on port ${port} did not become ready within ${timeoutMs}ms (last error: ${String(
      lastErr,
    )})`,
  );
}

/**
 * Resolve `target/debug/demo-server` relative to the compiled
 * integration runner. Compiled layout:
 * `editors/vscode/tests/integration/out/suite/cookieJarMode.test.js`.
 * Six `..` walks back to the repository root. runTest.ts lives one
 * level shallower (out/runTest.js) and uses five, so the asymmetry
 * is deliberate.
 */
function demoServerPath(): string {
  return path.resolve(__dirname, "../../../../../../target/debug/demo-server");
}

describe("tarn.cookieJarMode (NAZ-280)", () => {
  let api: TarnExtensionApiShape;
  let serverProcess: cp.ChildProcess | undefined;
  let port: number;

  before(async function () {
    this.timeout(60000);
    api = await getApi();

    const binary = demoServerPath();
    if (!fs.existsSync(binary)) {
      throw new Error(
        `demo-server binary not found at ${binary}. Run \`cargo build -p demo-server\` from the repo root first.`,
      );
    }

    port = await allocateFreePort();
    serverProcess = cp.spawn(binary, [], {
      env: { ...process.env, PORT: String(port) },
      stdio: ["ignore", "pipe", "pipe"],
    });
    serverProcess.on("error", (err) => {
      // Surface the spawn failure loudly so the test does not time out
      // mysteriously when the demo-server binary is missing or broken.
      // eslint-disable-next-line no-console
      console.error(`[cookieJarMode] demo-server spawn error:`, err);
    });

    await waitForServerReady(port, 10000);
  });

  after(async function () {
    this.timeout(10000);
    await updateSetting("tarn.cookieJarMode", undefined);
    if (serverProcess && !serverProcess.killed) {
      serverProcess.kill("SIGTERM");
      await new Promise<void>((resolve) => {
        const timer = setTimeout(() => {
          serverProcess?.kill("SIGKILL");
          resolve();
        }, 3000);
        serverProcess?.once("exit", () => {
          clearTimeout(timer);
          resolve();
        });
      });
    }
  });

  async function runFixture(): Promise<RunOutcome> {
    const cts = new vscode.CancellationTokenSource();
    try {
      return await api.testing.backend.run({
        files: [FIXTURE_FILE],
        cwd: workspaceRoot(),
        vars: { base_url: `http://127.0.0.1:${port}` },
        token: cts.token,
      });
    } finally {
      cts.dispose();
    }
  }

  it("default mode inherits session across named tests (fixture fails)", async function () {
    this.timeout(30000);
    await updateSetting("tarn.cookieJarMode", "default");

    const outcome = await runFixture();
    assert.ok(
      outcome.report,
      `expected a JSON report, exit=${outcome.exitCode}, stderr=${outcome.stderr}, stdout=${outcome.stdout.slice(0, 500)}`,
    );
    const report = outcome.report!;

    assert.strictEqual(
      report.summary.status,
      "FAILED",
      `expected the fixture to fail in default mode, but summary.status = ${report.summary.status}`,
    );

    // login passes; the two "clean jar" tests must fail because the
    // session cookie from login leaked into them.
    const fileReport = report.files[0];
    assert.ok(fileReport, "expected exactly one file in the report");
    assert.strictEqual(fileReport.summary.passed, 1, "only login should pass");
    assert.strictEqual(fileReport.summary.failed, 2, "both clean-jar tests must fail");

    const byName = new Map(fileReport.tests.map((t) => [t.name, t.status]));
    assert.strictEqual(byName.get("login_sets_session"), "PASSED");
    assert.strictEqual(byName.get("first_expects_clean_jar"), "FAILED");
    assert.strictEqual(byName.get("second_expects_clean_jar"), "FAILED");
  });

  it("per-test mode resets the jar between named tests (fixture passes)", async function () {
    this.timeout(30000);
    await updateSetting("tarn.cookieJarMode", "per-test");

    const outcome = await runFixture();
    assert.ok(outcome.report, `expected a JSON report, got stderr: ${outcome.stderr}`);
    const report = outcome.report!;

    assert.strictEqual(
      report.summary.status,
      "PASSED",
      `expected the fixture to pass in per-test mode, but summary.status = ${report.summary.status}`,
    );

    const fileReport = report.files[0];
    assert.ok(fileReport, "expected exactly one file in the report");
    assert.strictEqual(fileReport.summary.passed, 3, "all three tests should pass");
    assert.strictEqual(fileReport.summary.failed, 0);

    for (const test of fileReport.tests) {
      assert.strictEqual(
        test.status,
        "PASSED",
        `test ${test.name} should pass in per-test mode`,
      );
    }
  });

  it("flipping back to default re-introduces the failure (setting is live)", async function () {
    this.timeout(30000);
    // Re-verify that the setting is honored on every run, not cached
    // at activation time. This is the whole point of the ticket —
    // subset runs flipping mid-session must pick up the new mode.
    await updateSetting("tarn.cookieJarMode", "default");
    const outcome = await runFixture();
    assert.ok(outcome.report);
    assert.strictEqual(outcome.report!.summary.status, "FAILED");
  });

  it("declares the tarn.cookieJarMode setting in package.json", () => {
    // Guards against accidentally dropping the contributes.configuration
    // entry during refactors — reading the inspect() result mirrors how
    // notifications.test.ts guards its own setting.
    const inspected = vscode.workspace
      .getConfiguration("tarn")
      .inspect<string>("cookieJarMode");
    assert.ok(inspected, "tarn.cookieJarMode should be declared");
    assert.strictEqual(inspected!.defaultValue, "default");
  });
});
