import { describe, it, expect } from "vitest";
import {
  parseReport,
  parseScopedListResult,
} from "../../src/util/schemaGuards";

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

  it("accepts the real tarn JSON shape: diff=null, no passed on failures[]", () => {
    // Regression: the schema used to require `diff: string | undefined`
    // and `passed: bool` on every assertion entry, but the real tarn
    // binary emits `diff: null` and omits `passed` inside `failures[]`
    // because those entries are by definition failed. parseReport must
    // accept that shape so a run with even one failing step does not
    // collapse to `report: undefined` in the runner.
    const realShape = {
      duration_ms: 5,
      files: [
        {
          file: "tests/cookie.tarn.yaml",
          name: "Cookie",
          status: "FAILED",
          duration_ms: 5,
          summary: { total: 1, passed: 0, failed: 1 },
          tests: [
            {
              name: "needs_clean_jar",
              status: "FAILED",
              duration_ms: 5,
              steps: [
                {
                  name: "confirm no session",
                  status: "FAILED",
                  duration_ms: 5,
                  assertions: {
                    total: 2,
                    passed: 1,
                    failed: 1,
                    details: [
                      {
                        assertion: "body $.session",
                        passed: false,
                        expected: "null",
                        actual: "\"abc123\"",
                        message: "JSONPath $.session: expected null",
                        diff: null,
                      },
                    ],
                    failures: [
                      {
                        // note: no `passed` field — the real tarn
                        // binary omits it inside failures[]
                        assertion: "body $.session",
                        expected: "null",
                        actual: "\"abc123\"",
                        message: "JSONPath $.session: expected null",
                        diff: null,
                      },
                    ],
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
        status: "FAILED" as const,
      },
    };
    const report = parseReport(JSON.stringify(realShape));
    expect(report.summary.status).toBe("FAILED");
    expect(report.files[0].tests[0].steps[0].assertions?.failures?.[0].diff).toBeNull();
  });

  it("rejects reports with wrong enum values", () => {
    const bad = { ...passingReport, summary: { ...passingReport.summary, status: "SKIPPED" } };
    expect(() => parseReport(JSON.stringify(bad))).toThrow();
  });

  it("rejects reports missing required fields", () => {
    const bad = { duration_ms: 1 };
    expect(() => parseReport(JSON.stringify(bad))).toThrow();
  });

  it("accepts optional `location` on steps and on assertion details/failures (NAZ-281)", () => {
    // Tarn T55 (NAZ-260) attaches a 1-based `location: { file, line, column }`
    // to every step and to every assertion detail/failure that maps back to
    // a YAML operator key. The extension consumes this in ResultMapper, so
    // the zod schema must preserve it through parseReport without dropping
    // the field or rejecting the payload.
    const withLocations = {
      duration_ms: 7,
      files: [
        {
          file: "tests/health.tarn.yaml",
          name: "Health",
          status: "FAILED",
          duration_ms: 7,
          summary: { total: 1, passed: 0, failed: 1 },
          tests: [
            {
              name: "smoke",
              description: null,
              status: "FAILED",
              duration_ms: 7,
              steps: [
                {
                  name: "GET /status/500",
                  status: "FAILED",
                  duration_ms: 7,
                  location: {
                    file: "/ws/tests/health.tarn.yaml",
                    line: 9,
                    column: 9,
                  },
                  assertions: {
                    total: 1,
                    passed: 0,
                    failed: 1,
                    details: [
                      {
                        assertion: "status",
                        passed: false,
                        expected: "200",
                        actual: "500",
                        message: "Expected HTTP status 200, got 500",
                        diff: null,
                        location: {
                          file: "/ws/tests/health.tarn.yaml",
                          line: 14,
                          column: 11,
                        },
                      },
                    ],
                    failures: [
                      {
                        assertion: "status",
                        expected: "200",
                        actual: "500",
                        message: "Expected HTTP status 200, got 500",
                        diff: null,
                        location: {
                          file: "/ws/tests/health.tarn.yaml",
                          line: 14,
                          column: 11,
                        },
                      },
                    ],
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
        status: "FAILED" as const,
      },
    };
    const report = parseReport(JSON.stringify(withLocations));
    const step = report.files[0].tests[0].steps[0];
    expect(step.location).toEqual({
      file: "/ws/tests/health.tarn.yaml",
      line: 9,
      column: 9,
    });
    expect(step.assertions?.details?.[0].location).toEqual({
      file: "/ws/tests/health.tarn.yaml",
      line: 14,
      column: 11,
    });
    expect(step.assertions?.failures?.[0].location).toEqual({
      file: "/ws/tests/health.tarn.yaml",
      line: 14,
      column: 11,
    });
  });

  it("rejects a location with non-positive line (must be 1-based)", () => {
    // Tarn spec: line and column are 1-based >= 1. A line of 0 means
    // a producer bug and must not silently coerce to a valid Position.
    const bad = {
      ...passingReport,
      files: [
        {
          ...passingReport.files[0],
          tests: [
            {
              ...passingReport.files[0].tests[0],
              steps: [
                {
                  ...passingReport.files[0].tests[0].steps[0],
                  location: { file: "x.yaml", line: 0, column: 1 },
                },
              ],
            },
          ],
        },
      ],
    };
    expect(() => parseReport(JSON.stringify(bad))).toThrow();
  });
});

describe("parseScopedListResult (Tarn T57 / NAZ-282)", () => {
  // This fixture mirrors the exact shape emitted by
  // `./target/debug/tarn list --file <path> --format json`, verified
  // against the integration workspace's `health.tarn.yaml`. The
  // scoped variant wraps the per-file record in the same top-level
  // `{ files: [...] }` envelope as the unscoped list, which is why
  // the schema accepts the envelope instead of a bare object.
  const tarnScopedOutput = {
    files: [
      {
        file: "/tmp/fixture.tarn.yaml",
        name: "Fixture: health check",
        setup: [],
        steps: [],
        tags: [],
        teardown: [],
        tests: [
          {
            description: "Pings the public httpbin 200 endpoint",
            name: "service_is_up",
            steps: [{ name: "GET /status/200" }],
            tags: [],
          },
        ],
      },
    ],
  };

  it("parses a named-tests scoped list output", () => {
    const parsed = parseScopedListResult(JSON.stringify(tarnScopedOutput));
    expect(parsed.files).toHaveLength(1);
    const file = parsed.files[0];
    expect(file.file).toBe("/tmp/fixture.tarn.yaml");
    expect(file.name).toBe("Fixture: health check");
    expect(file.tests).toHaveLength(1);
    expect(file.tests[0].name).toBe("service_is_up");
    expect(file.tests[0].description).toBe(
      "Pings the public httpbin 200 endpoint",
    );
    expect(file.tests[0].steps).toEqual([{ name: "GET /status/200" }]);
    expect(file.steps).toEqual([]);
    expect(file.setup).toEqual([]);
    expect(file.teardown).toEqual([]);
  });

  it("parses a flat-steps scoped list output (top-level `steps:` form)", () => {
    // Files that use the legacy `steps:` block at the top level
    // instead of `tests:` must still parse — Tarn places the steps
    // on `files[0].steps` and leaves `tests[]` empty.
    const flatSteps = {
      files: [
        {
          file: "/tmp/flat.tarn.yaml",
          name: "Health check",
          setup: [],
          steps: [{ name: "GET /health" }],
          tags: [],
          teardown: [],
          tests: [],
        },
      ],
    };
    const parsed = parseScopedListResult(JSON.stringify(flatSteps));
    expect(parsed.files[0].steps).toEqual([{ name: "GET /health" }]);
    expect(parsed.files[0].tests).toEqual([]);
  });

  it("parses an empty files[] envelope (scoped call against a missing path)", () => {
    // Tarn prints `{ error, files: [] }` when the scoped path is
    // invalid; the schema must accept the error envelope so
    // `listFile` can unwrap and return `undefined` deterministically.
    const errorEnvelope = {
      error: "Config error: Path not found: /nonexistent.tarn.yaml",
      files: [],
    };
    const parsed = parseScopedListResult(JSON.stringify(errorEnvelope));
    expect(parsed.files).toEqual([]);
    expect(parsed.error).toContain("Path not found");
  });

  it("rejects a payload missing the top-level `files` array", () => {
    // The envelope shape is load-bearing: a bare per-file object
    // (without the `{ files: [...] }` wrapper) means either an old
    // Tarn binary or a completely unrelated JSON payload, both of
    // which should bounce at the schema gate so the caller falls
    // back to the AST path.
    const bare = {
      file: "/tmp/naked.tarn.yaml",
      name: "naked",
      setup: [],
      steps: [],
      tests: [],
      teardown: [],
    };
    expect(() => parseScopedListResult(JSON.stringify(bare))).toThrow();
  });

  it("parses Tarn's per-file error envelope (a file Tarn could not parse)", () => {
    // When Tarn cannot parse the YAML at the scoped path it still
    // emits a valid top-level envelope but replaces the per-file
    // shape with `{ file, error }`. parseScopedListResult must
    // accept this degraded shape so `listFile` can distinguish
    // "file-level parse error" from "binary is wrong / missing".
    const perFileError = {
      files: [
        {
          file: "/tmp/broken.tarn.yaml",
          error:
            "Parse error: /tmp/broken.tarn.yaml: Test file must have either 'steps' or 'tests'",
        },
      ],
    };
    const parsed = parseScopedListResult(JSON.stringify(perFileError));
    expect(parsed.files).toHaveLength(1);
    expect(parsed.files[0].file).toBe("/tmp/broken.tarn.yaml");
    expect(parsed.files[0].error).toContain("Parse error");
    expect(parsed.files[0].name).toBeUndefined();
    expect(parsed.files[0].tests).toBeUndefined();
  });

  it("rejects a payload whose `files` entries are not objects", () => {
    // Load-bearing: if a future Tarn release somehow emits a bare
    // string (say, the file path) in `files[]`, we must bounce at
    // the schema gate rather than silently dereferencing a string
    // as a file record inside the runner.
    const bad = {
      files: ["/tmp/x.tarn.yaml"],
    };
    expect(() => parseScopedListResult(JSON.stringify(bad))).toThrow();
  });
});
