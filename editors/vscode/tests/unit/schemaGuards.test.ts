import { describe, it, expect } from "vitest";
import { parseReport } from "../../src/util/schemaGuards";

const passingReport = {
  schema_version: 1,
  version: "1",
  timestamp: "2026-04-10T00:00:00Z",
  duration_ms: 42,
  files: [
    {
      file: "tests/health.tarn.yaml",
      name: "Health",
      status: "PASSED",
      duration_ms: 42,
      summary: { total: 1, passed: 1, failed: 0 },
      tests: [
        {
          name: "default",
          description: null,
          status: "PASSED",
          duration_ms: 42,
          steps: [
            {
              name: "GET /health",
              status: "PASSED",
              duration_ms: 42,
              assertions: { total: 1, passed: 1, failed: 0, details: [], failures: [] },
            },
          ],
        },
      ],
    },
  ],
  summary: {
    files: 1,
    tests: 1,
    steps: { total: 1, passed: 1, failed: 0 },
    status: "PASSED",
  },
};

const failingReport = {
  duration_ms: 100,
  files: [
    {
      file: "tests/users.tarn.yaml",
      name: "Users",
      status: "FAILED",
      duration_ms: 100,
      summary: { total: 2, passed: 1, failed: 1 },
      tests: [
        {
          name: "create_user",
          description: "Creates a user",
          status: "FAILED",
          duration_ms: 100,
          steps: [
            {
              name: "Create",
              status: "FAILED",
              duration_ms: 100,
              failure_category: "assertion_failed",
              error_code: "assertion_mismatch",
              remediation_hints: ["Check the expected status"],
              assertions: {
                total: 1,
                passed: 0,
                failed: 1,
                failures: [
                  {
                    assertion: "status",
                    passed: false,
                    expected: "201",
                    actual: "500",
                    message: "unexpected status",
                    diff: "--- expected\n+++ actual\n-201\n+500",
                  },
                ],
              },
              request: {
                method: "POST",
                url: "http://localhost:3000/users",
                headers: { "content-type": "application/json" },
                body: { name: "alice" },
              },
              response: {
                status: 500,
                headers: {},
                body: { error: "kaboom" },
              },
            },
          ],
        },
      ],
    },
  ],
  summary: {
    files: 1,
    tests: 1,
    steps: { total: 1, passed: 0, failed: 1 },
    status: "FAILED",
  },
};

describe("parseReport", () => {
  it("accepts a passing report", () => {
    const report = parseReport(JSON.stringify(passingReport));
    expect(report.summary.status).toBe("PASSED");
    expect(report.files[0].tests[0].steps[0].status).toBe("PASSED");
  });

  it("accepts a failing report with rich failure detail", () => {
    const report = parseReport(JSON.stringify(failingReport));
    const step = report.files[0].tests[0].steps[0];
    expect(step.status).toBe("FAILED");
    expect(step.failure_category).toBe("assertion_failed");
    expect(step.error_code).toBe("assertion_mismatch");
    expect(step.assertions?.failures?.[0].diff).toContain("+500");
    expect(step.request?.method).toBe("POST");
    expect(step.response?.status).toBe(500);
  });

  it("rejects reports with wrong enum values", () => {
    const bad = { ...passingReport, summary: { ...passingReport.summary, status: "SKIPPED" } };
    expect(() => parseReport(JSON.stringify(bad))).toThrow();
  });

  it("rejects reports missing required fields", () => {
    const bad = { duration_ms: 1 };
    expect(() => parseReport(JSON.stringify(bad))).toThrow();
  });
});
