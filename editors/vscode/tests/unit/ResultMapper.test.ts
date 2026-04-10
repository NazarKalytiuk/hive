import { describe, it, expect } from "vitest";
import { buildFailureMessages } from "../../src/testing/ResultMapper";
import type { StepResult } from "../../src/util/schemaGuards";
import { Range, Position, Uri, MarkdownString } from "./__mocks__/vscode";

function fakeStepItem() {
  return {
    range: new Range(new Position(10, 2), new Position(10, 20)),
  };
}

function fakeParsed() {
  return {
    uri: Uri.file("/fake/tests/users.tarn.yaml"),
    ranges: { fileName: "users", tests: [], setup: [], teardown: [], fileNameRange: undefined },
  };
}

describe("buildFailureMessages", () => {
  it("renders assertion_mismatch with diff, expected, actual, and request/response", () => {
    const step: StepResult = {
      name: "Create user",
      status: "FAILED",
      duration_ms: 42,
      failure_category: "assertion_failed",
      error_code: "assertion_mismatch",
      remediation_hints: ["check server status"],
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
        url: "http://localhost/users",
        headers: { "content-type": "application/json" },
        body: { name: "alice" },
      },
      response: { status: 500, headers: {}, body: { error: "boom" } },
    };

    const msgs = buildFailureMessages(
      step,
      fakeStepItem() as never,
      fakeParsed() as never,
    );
    expect(msgs).toHaveLength(1);
    const m = msgs[0];
    expect(m.expectedOutput).toBe("201");
    expect(m.actualOutput).toBe("500");
    expect(m.location).toBeDefined();
    expect(m.message).toBeInstanceOf(MarkdownString);
    const text = (m.message as MarkdownString).value;
    expect(text).toContain("Create user");
    expect(text).toContain("assertion_failed");
    expect(text).toContain("assertion_mismatch");
    expect(text).toContain("check server status");
    expect(text).toContain("+500");
    expect(text).toContain("POST http://localhost/users");
    expect(text).toContain("HTTP 500");
    expect(text).toContain("alice");
    expect(text).toContain("boom");
  });

  it("emits one message per assertion failure", () => {
    const step: StepResult = {
      name: "Multi-assert",
      status: "FAILED",
      duration_ms: 1,
      assertions: {
        total: 2,
        passed: 0,
        failed: 2,
        failures: [
          { assertion: "status", passed: false, expected: "200", actual: "500" },
          { assertion: "body $.ok", passed: false, expected: "true", actual: "false" },
        ],
      },
    };
    const msgs = buildFailureMessages(
      step,
      fakeStepItem() as never,
      fakeParsed() as never,
    );
    expect(msgs).toHaveLength(2);
    expect(msgs[0].expectedOutput).toBe("200");
    expect(msgs[1].expectedOutput).toBe("true");
  });

  it("falls back to a generic message when no assertion failures are attached", () => {
    const step: StepResult = {
      name: "Connect",
      status: "FAILED",
      duration_ms: 1500,
      failure_category: "connection_error",
      error_code: "connection_refused",
      remediation_hints: ["is the server running?"],
    };
    const msgs = buildFailureMessages(
      step,
      fakeStepItem() as never,
      fakeParsed() as never,
    );
    expect(msgs).toHaveLength(1);
    const text = (msgs[0].message as MarkdownString).value;
    expect(text).toContain("Connect");
    expect(text).toContain("connection_error");
    expect(text).toContain("connection_refused");
    expect(text).toContain("is the server running?");
  });

  it("covers every documented failure category with a generic message", () => {
    const categories = [
      "assertion_failed",
      "connection_error",
      "timeout",
      "parse_error",
      "capture_error",
      "unresolved_template",
    ] as const;
    for (const category of categories) {
      const step: StepResult = {
        name: `${category} step`,
        status: "FAILED",
        duration_ms: 1,
        failure_category: category,
      };
      const msgs = buildFailureMessages(
        step,
        fakeStepItem() as never,
        fakeParsed() as never,
      );
      expect(msgs).toHaveLength(1);
      const text = (msgs[0].message as MarkdownString).value;
      expect(text).toContain(category);
    }
  });
});
