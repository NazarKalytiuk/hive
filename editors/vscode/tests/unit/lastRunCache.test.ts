import { describe, it, expect } from "vitest";
import { LastRunCache, encode, decode } from "../../src/testing/LastRunCache";
import type { Report } from "../../src/util/schemaGuards";

const SAMPLE_REPORT: Report = {
  schema_version: 1,
  version: "1",
  timestamp: "2026-04-10T12:00:00Z",
  duration_ms: 500,
  files: [
    {
      file: "tests/users.tarn.yaml",
      name: "Users",
      status: "FAILED",
      duration_ms: 500,
      summary: { total: 3, passed: 2, failed: 1 },
      setup: [
        {
          name: "Authenticate",
          status: "PASSED",
          duration_ms: 50,
        },
      ],
      tests: [
        {
          name: "create_user",
          description: "Create then verify",
          status: "FAILED",
          duration_ms: 400,
          steps: [
            { name: "POST /users", status: "PASSED", duration_ms: 100 },
            {
              name: "GET /users/1",
              status: "FAILED",
              duration_ms: 300,
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
                  },
                ],
              },
              request: { method: "GET", url: "http://localhost/users/1" },
              response: { status: 500 },
            },
          ],
        },
      ],
      teardown: [
        {
          name: "Clean up",
          status: "PASSED",
          duration_ms: 50,
        },
      ],
    },
  ],
  summary: {
    files: 1,
    tests: 1,
    steps: { total: 3, passed: 2, failed: 1 },
    status: "FAILED",
  },
};

describe("LastRunCache", () => {
  it("is empty before a report is loaded", () => {
    const cache = new LastRunCache();
    expect(cache.size()).toBe(0);
    expect(cache.get({ file: "x", test: "y", stepIndex: 0 })).toBeUndefined();
  });

  it("indexes every step from setup, tests, and teardown", () => {
    const cache = new LastRunCache();
    cache.loadFromReport(SAMPLE_REPORT);
    // 1 setup + 2 test steps + 1 teardown = 4 snapshots
    expect(cache.size()).toBe(4);
  });

  it("resolves setup steps via a setup test name", () => {
    const cache = new LastRunCache();
    cache.loadFromReport(SAMPLE_REPORT);
    const snapshot = cache.get({
      file: "tests/users.tarn.yaml",
      test: "setup",
      stepIndex: 0,
    });
    expect(snapshot).toBeDefined();
    expect(snapshot!.phase).toBe("setup");
    expect(snapshot!.stepName).toBe("Authenticate");
  });

  it("resolves test steps via the containing test's name", () => {
    const cache = new LastRunCache();
    cache.loadFromReport(SAMPLE_REPORT);
    const first = cache.get({
      file: "tests/users.tarn.yaml",
      test: "create_user",
      stepIndex: 0,
    });
    expect(first).toBeDefined();
    expect(first!.phase).toBe("test");
    expect(first!.step.name).toBe("POST /users");

    const second = cache.get({
      file: "tests/users.tarn.yaml",
      test: "create_user",
      stepIndex: 1,
    });
    expect(second!.step.status).toBe("FAILED");
    expect(second!.step.assertions?.failures?.[0].expected).toBe("200");
  });

  it("loadFromReport replaces previous entries", () => {
    const cache = new LastRunCache();
    cache.loadFromReport(SAMPLE_REPORT);
    expect(cache.size()).toBe(4);

    const smaller: Report = {
      ...SAMPLE_REPORT,
      files: [
        {
          ...SAMPLE_REPORT.files[0],
          setup: undefined,
          teardown: undefined,
          tests: [
            {
              name: "single",
              description: null,
              status: "PASSED",
              duration_ms: 10,
              steps: [
                { name: "s", status: "PASSED", duration_ms: 10 },
              ],
            },
          ],
        },
      ],
    };
    cache.loadFromReport(smaller);
    expect(cache.size()).toBe(1);
  });

  it("clear() empties the cache", () => {
    const cache = new LastRunCache();
    cache.loadFromReport(SAMPLE_REPORT);
    cache.clear();
    expect(cache.size()).toBe(0);
  });
});

describe("StepKey encode/decode", () => {
  it("round-trips a simple key", () => {
    const key = { file: "tests/a.tarn.yaml", test: "login", stepIndex: 2 };
    const encoded = encode(key);
    expect(encoded).toBe("tests/a.tarn.yaml::login::2");
    expect(decode(encoded)).toEqual(key);
  });

  it("returns undefined for malformed encodings", () => {
    expect(decode("")).toBeUndefined();
    expect(decode("just-a-file")).toBeUndefined();
    expect(decode("file::test::notanumber")).toBeUndefined();
  });
});
