import { describe, it, expect } from "vitest";
import {
  FailureNotifier,
  formatFailureMessage,
  shouldNotifyOnFailure,
} from "../../src/notifications";
import type { Report } from "../../src/util/schemaGuards";

function makeReport(overrides: {
  failed?: number;
  passed?: number;
  files?: Array<{ name: string; file: string; status: "PASSED" | "FAILED" }>;
}): Report {
  const failed = overrides.failed ?? 0;
  const passed = overrides.passed ?? 0;
  return {
    schema_version: 1,
    version: "1",
    timestamp: "2026-04-10T12:00:00Z",
    duration_ms: 100,
    files: (overrides.files ?? []).map((f) => ({
      file: f.file,
      name: f.name,
      status: f.status,
      duration_ms: 50,
      summary: {
        total: 1,
        passed: f.status === "PASSED" ? 1 : 0,
        failed: f.status === "FAILED" ? 1 : 0,
      },
      setup: [],
      tests: [],
      teardown: [],
    })),
    summary: {
      files: (overrides.files ?? []).length,
      tests: passed + failed,
      steps: { total: passed + failed, passed, failed },
      status: failed > 0 ? "FAILED" : "PASSED",
    },
  };
}

describe("shouldNotifyOnFailure", () => {
  const base = {
    mode: "focused" as const,
    dryRun: false,
    failedSteps: 1,
    tarnViewVisible: false,
  };

  it("never notifies when mode is off", () => {
    expect(shouldNotifyOnFailure({ ...base, mode: "off" })).toBe(false);
  });

  it("never notifies for dry runs", () => {
    expect(shouldNotifyOnFailure({ ...base, dryRun: true })).toBe(false);
  });

  it("never notifies when there are no failing steps", () => {
    expect(shouldNotifyOnFailure({ ...base, failedSteps: 0 })).toBe(false);
    expect(shouldNotifyOnFailure({ ...base, failedSteps: -1 })).toBe(false);
  });

  it('in "focused" mode suppresses the toast when the Tarn view is visible', () => {
    expect(
      shouldNotifyOnFailure({ ...base, mode: "focused", tarnViewVisible: true }),
    ).toBe(false);
  });

  it('in "focused" mode still notifies when the Tarn view is hidden', () => {
    expect(
      shouldNotifyOnFailure({ ...base, mode: "focused", tarnViewVisible: false }),
    ).toBe(true);
  });

  it('in "always" mode notifies even when the Tarn view is visible', () => {
    expect(
      shouldNotifyOnFailure({ ...base, mode: "always", tarnViewVisible: true }),
    ).toBe(true);
  });
});

describe("formatFailureMessage", () => {
  it("pluralizes the step count", () => {
    const one = makeReport({
      failed: 1,
      files: [{ name: "login", file: "tests/login.tarn.yaml", status: "FAILED" }],
    });
    expect(formatFailureMessage(one)).toBe("Tarn: 1 failed step in login");

    const many = makeReport({
      failed: 3,
      files: [
        { name: "login", file: "tests/login.tarn.yaml", status: "FAILED" },
      ],
    });
    expect(formatFailureMessage(many)).toBe("Tarn: 3 failed steps in login");
  });

  it("lists up to three failing file names inline", () => {
    const report = makeReport({
      failed: 3,
      files: [
        { name: "a", file: "tests/a.tarn.yaml", status: "FAILED" },
        { name: "b", file: "tests/b.tarn.yaml", status: "FAILED" },
        { name: "c", file: "tests/c.tarn.yaml", status: "FAILED" },
      ],
    });
    expect(formatFailureMessage(report)).toBe("Tarn: 3 failed steps in a, b, c");
  });

  it("collapses to a count when more than three files failed", () => {
    const report = makeReport({
      failed: 4,
      files: [
        { name: "a", file: "tests/a.tarn.yaml", status: "FAILED" },
        { name: "b", file: "tests/b.tarn.yaml", status: "FAILED" },
        { name: "c", file: "tests/c.tarn.yaml", status: "FAILED" },
        { name: "d", file: "tests/d.tarn.yaml", status: "FAILED" },
      ],
    });
    expect(formatFailureMessage(report)).toBe("Tarn: 4 failed steps across 4 files");
  });

  it("drops the file suffix entirely when no files failed", () => {
    const report = makeReport({ failed: 2, files: [] });
    expect(formatFailureMessage(report)).toBe("Tarn: 2 failed steps");
  });

  it("ignores passing files when building the name list", () => {
    const report = makeReport({
      failed: 1,
      passed: 1,
      files: [
        { name: "good", file: "tests/good.tarn.yaml", status: "PASSED" },
        { name: "bad", file: "tests/bad.tarn.yaml", status: "FAILED" },
      ],
    });
    expect(formatFailureMessage(report)).toBe("Tarn: 1 failed step in bad");
  });
});

describe("FailureNotifier.maybeNotify short-circuits without side effects", () => {
  it("returns false when dryRun is true and never invokes handlers", async () => {
    const calls: string[] = [];
    const notifier = new FailureNotifier(() => false, {
      showFixPlan: () => void calls.push("fix"),
      openReport: () => void calls.push("report"),
      rerunFailed: () => void calls.push("rerun"),
    });
    const report = makeReport({
      failed: 1,
      files: [{ name: "x", file: "tests/x.tarn.yaml", status: "FAILED" }],
    });
    const shown = await notifier.maybeNotify(report, {
      dryRun: true,
      files: ["tests/x.tarn.yaml"],
    });
    expect(shown).toBe(false);
    expect(calls).toEqual([]);
  });

  it("returns false when there are no failed steps", async () => {
    const notifier = new FailureNotifier(() => false, {
      showFixPlan: async () => undefined,
      openReport: async () => undefined,
      rerunFailed: async () => undefined,
    });
    const report = makeReport({ failed: 0, passed: 5, files: [] });
    const shown = await notifier.maybeNotify(report, {
      dryRun: false,
      files: [],
    });
    expect(shown).toBe(false);
  });
});
