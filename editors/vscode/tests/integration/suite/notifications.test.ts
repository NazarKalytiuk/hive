import * as assert from "assert";
import * as vscode from "vscode";

const EXTENSION_ID = "nazarkalytiuk.tarn-vscode";

interface ReportShape {
  schema_version?: number;
  version?: string;
  duration_ms: number;
  files: Array<{
    file: string;
    name: string;
    status: "PASSED" | "FAILED";
    duration_ms: number;
    summary: { total: number; passed: number; failed: number };
    setup: unknown[];
    tests: unknown[];
    teardown: unknown[];
  }>;
  summary: {
    files: number;
    tests: number;
    steps: { total: number; passed: number; failed: number };
    status: "PASSED" | "FAILED";
  };
}

interface TarnExtensionApiShape {
  readonly commands: readonly string[];
  readonly testing: {
    readonly notifier: {
      readonly isTarnViewFocused: () => boolean;
      readonly wouldNotify: (
        report: ReportShape,
        options: { dryRun: boolean },
      ) => boolean;
      readonly maybeNotify: (
        report: ReportShape,
        options: { dryRun: boolean; files: string[] },
      ) => Promise<boolean>;
    };
  };
}

async function getApi(): Promise<TarnExtensionApiShape> {
  const ext = vscode.extensions.getExtension<TarnExtensionApiShape>(EXTENSION_ID);
  assert.ok(ext, `extension ${EXTENSION_ID} not found`);
  const api = await ext!.activate();
  assert.ok(api, "extension activated but returned no API");
  return api;
}

function failingReport(): ReportShape {
  return {
    schema_version: 1,
    version: "1",
    duration_ms: 50,
    files: [
      {
        file: "tests/login.tarn.yaml",
        name: "login",
        status: "FAILED",
        duration_ms: 50,
        summary: { total: 1, passed: 0, failed: 1 },
        setup: [],
        tests: [],
        teardown: [],
      },
    ],
    summary: {
      files: 1,
      tests: 1,
      steps: { total: 1, passed: 0, failed: 1 },
      status: "FAILED",
    },
  };
}

function passingReport(): ReportShape {
  return {
    schema_version: 1,
    version: "1",
    duration_ms: 10,
    files: [
      {
        file: "tests/ok.tarn.yaml",
        name: "ok",
        status: "PASSED",
        duration_ms: 10,
        summary: { total: 1, passed: 1, failed: 0 },
        setup: [],
        tests: [],
        teardown: [],
      },
    ],
    summary: {
      files: 1,
      tests: 1,
      steps: { total: 1, passed: 1, failed: 0 },
      status: "PASSED",
    },
  };
}

describe("FailureNotifier (tarn.notifications.failure)", () => {
  let api: TarnExtensionApiShape;
  let originalMode: string | undefined;

  before(async function () {
    this.timeout(60000);
    api = await getApi();
    const cfg = vscode.workspace.getConfiguration("tarn");
    originalMode = cfg.get<string>("notifications.failure");
    // Force "always" so the decision doesn't depend on whether
    // VS Code happens to have the Tarn view visible during the
    // test run.
    await cfg.update(
      "notifications.failure",
      "always",
      vscode.ConfigurationTarget.Workspace,
    );
  });

  after(async () => {
    const cfg = vscode.workspace.getConfiguration("tarn");
    await cfg.update(
      "notifications.failure",
      originalMode,
      vscode.ConfigurationTarget.Workspace,
    );
  });

  it("contributes the tarn.notifications.failure setting", async () => {
    const cfg = vscode.workspace.getConfiguration("tarn");
    const inspected = cfg.inspect<string>("notifications.failure");
    assert.ok(inspected, "expected the setting to be contributed");
    assert.strictEqual(inspected!.defaultValue, "focused");
  });

  it("exposes a tarn-view-focused signal", () => {
    // The signal returns a plain boolean, we just ensure it doesn't throw.
    const value = api.testing.notifier.isTarnViewFocused();
    assert.strictEqual(typeof value, "boolean");
  });

  it("wouldNotify returns false for a passing report", () => {
    assert.strictEqual(
      api.testing.notifier.wouldNotify(passingReport(), { dryRun: false }),
      false,
    );
  });

  it("wouldNotify returns false for dry runs even when failures exist", () => {
    assert.strictEqual(
      api.testing.notifier.wouldNotify(failingReport(), { dryRun: true }),
      false,
    );
  });

  it("wouldNotify returns true for a failing non-dry run in 'always' mode", () => {
    assert.strictEqual(
      api.testing.notifier.wouldNotify(failingReport(), { dryRun: false }),
      true,
    );
  });

  it("switching to 'off' suppresses the decision", async function () {
    this.timeout(5000);
    const cfg = vscode.workspace.getConfiguration("tarn");
    await cfg.update(
      "notifications.failure",
      "off",
      vscode.ConfigurationTarget.Workspace,
    );
    assert.strictEqual(
      api.testing.notifier.wouldNotify(failingReport(), { dryRun: false }),
      false,
    );
    await cfg.update(
      "notifications.failure",
      "always",
      vscode.ConfigurationTarget.Workspace,
    );
  });
});
