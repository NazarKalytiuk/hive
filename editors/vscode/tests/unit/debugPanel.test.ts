import { describe, it, expect } from "vitest";
import {
  controlToLspCommand,
  escapeHtml,
  isDebugWebviewMessage,
  renderPanelHtml,
  type CaptureStatePayload,
  type DebugWebviewMessage,
} from "../../src/debug/DebugPanel";
import {
  extractSessionId,
  isCaptureStatePayload,
  renderDiffSummary,
  unwrapEnvelope,
} from "../../src/debug";

describe("isDebugWebviewMessage", () => {
  it("accepts the ready message", () => {
    const msg: DebugWebviewMessage = { type: "ready" };
    expect(isDebugWebviewMessage(msg)).toBe(true);
  });

  it.each([
    ["continue"],
    ["stepOver"],
    ["rerunStep"],
    ["restart"],
    ["stop"],
  ] as const)("accepts control command %s", (cmd) => {
    expect(isDebugWebviewMessage({ type: "control", command: cmd })).toBe(true);
  });

  it("rejects unknown control commands", () => {
    expect(isDebugWebviewMessage({ type: "control", command: "skip" })).toBe(false);
  });

  it("rejects non-objects", () => {
    expect(isDebugWebviewMessage(null)).toBe(false);
    expect(isDebugWebviewMessage("control")).toBe(false);
    expect(isDebugWebviewMessage(42)).toBe(false);
    expect(isDebugWebviewMessage(undefined)).toBe(false);
  });

  it("rejects objects without a type", () => {
    expect(isDebugWebviewMessage({ command: "continue" })).toBe(false);
  });
});

describe("controlToLspCommand", () => {
  it("maps every control into the documented tarn.debug* id", () => {
    expect(controlToLspCommand("continue")).toBe("tarn.debugContinue");
    expect(controlToLspCommand("stepOver")).toBe("tarn.debugStepOver");
    expect(controlToLspCommand("rerunStep")).toBe("tarn.debugRerunStep");
    expect(controlToLspCommand("restart")).toBe("tarn.debugRestart");
    expect(controlToLspCommand("stop")).toBe("tarn.debugStop");
  });
});

describe("escapeHtml", () => {
  it("escapes every special character", () => {
    const input = `<a href="x" class='y'>A & B</a>`;
    const escaped = escapeHtml(input);
    expect(escaped).not.toContain("<a");
    expect(escaped).toContain("&lt;a href=&quot;x&quot;");
    expect(escaped).toContain("class=&#039;y&#039;");
    expect(escaped).toContain("A &amp; B");
  });
});

describe("renderPanelHtml", () => {
  it("renders a waiting message when no state is loaded", () => {
    const html = renderPanelHtml(undefined, "sess-1");
    expect(html).toContain("sess-1");
    expect(html).toContain("waiting for first step");
    // Controls must be disabled pre-state so the user cannot fire a
    // command before the first step has run.
    expect(html).toContain('id="btn-continue" disabled');
  });

  it("renders captures and last response when present", () => {
    const state: CaptureStatePayload = {
      sessionId: "sess-2",
      stepIndex: 1,
      phase: "test",
      captures: { user_id: 42 },
      lastResponse: { status: 200, headers: {}, body: { ok: true } },
      lastStep: {
        name: "GET /users/42",
        passed: true,
        duration_ms: 123,
      },
      done: false,
    };
    const html = renderPanelHtml(state, "sess-2");
    expect(html).toContain("user_id");
    expect(html).toContain("42");
    expect(html).toContain("GET /users/42");
    expect(html).toContain("PASS");
    // Controls enabled while the session is live.
    expect(html).not.toContain('id="btn-continue" disabled');
  });

  it("renders assertion failures when the step failed", () => {
    const state: CaptureStatePayload = {
      sessionId: "sess-3",
      stepIndex: 0,
      phase: "test",
      captures: {},
      lastResponse: null,
      lastStep: {
        name: "POST /login",
        passed: false,
        duration_ms: 500,
        assertion_failures: [
          {
            assertion: "status",
            expected: "200",
            actual: "500",
            message: "Expected 200 got 500",
          },
        ],
      },
      done: false,
    };
    const html = renderPanelHtml(state, "sess-3");
    expect(html).toContain("FAIL");
    expect(html).toContain("Expected 200 got 500");
  });

  it("disables controls when the session is done", () => {
    const state: CaptureStatePayload = {
      sessionId: "sess-4",
      stepIndex: 0,
      phase: "finished",
      captures: {},
      lastResponse: null,
      lastStep: null,
      done: true,
    };
    const html = renderPanelHtml(state, "sess-4");
    expect(html).toContain("finished");
    expect(html).toContain('id="btn-continue" disabled');
  });
});

describe("isCaptureStatePayload", () => {
  it("accepts a well-formed payload", () => {
    const payload = {
      sessionId: "s",
      stepIndex: 0,
      phase: "test",
      captures: {},
      lastResponse: null,
      lastStep: null,
      done: false,
    };
    expect(isCaptureStatePayload(payload)).toBe(true);
  });

  it("rejects payloads missing required fields", () => {
    expect(isCaptureStatePayload({})).toBe(false);
    expect(isCaptureStatePayload({ sessionId: "s" })).toBe(false);
    expect(
      isCaptureStatePayload({ sessionId: "s", stepIndex: 0, phase: "test" }),
    ).toBe(false);
  });
});

describe("extractSessionId", () => {
  it("unwraps the schema envelope", () => {
    const raw = { schema_version: 1, data: { sessionId: "abc" } };
    expect(extractSessionId(raw)).toBe("abc");
  });

  it("supports a bare sessionId payload", () => {
    expect(extractSessionId({ sessionId: "bare" })).toBe("bare");
  });

  it("returns undefined for malformed payloads", () => {
    expect(extractSessionId(null)).toBeUndefined();
    expect(extractSessionId("")).toBeUndefined();
    expect(extractSessionId({ data: { notSession: true } })).toBeUndefined();
  });
});

describe("unwrapEnvelope", () => {
  it("returns the data field when present", () => {
    expect(unwrapEnvelope({ schema_version: 1, data: { x: 1 } })).toEqual({ x: 1 });
  });

  it("returns the raw value when there is no envelope", () => {
    expect(unwrapEnvelope({ x: 1 })).toEqual({ x: 1 });
  });

  it("returns undefined for non-objects", () => {
    expect(unwrapEnvelope("x")).toBeUndefined();
    expect(unwrapEnvelope(null)).toBeUndefined();
  });
});

describe("renderDiffSummary", () => {
  it("shows status shift with was/now values", () => {
    const md = renderDiffSummary({
      status: { was: 200, now: 500 },
    });
    expect(md).toContain("**Status:**");
    expect(md).toContain("`200`");
    expect(md).toContain("`500`");
  });

  it("lists added/removed/changed headers", () => {
    const md = renderDiffSummary({
      headers_added: ["x-new"],
      headers_removed: ["x-old"],
      headers_changed: [{ name: "content-type", was: "json", now: "xml" }],
    });
    expect(md).toContain("Headers added");
    expect(md).toContain("x-new");
    expect(md).toContain("Headers removed");
    expect(md).toContain("x-old");
    expect(md).toContain("Headers changed");
    expect(md).toContain("content-type");
  });

  it("lists body changes with JSONPath-style keys", () => {
    const md = renderDiffSummary({
      body_keys_added: ["$.new"],
      body_keys_removed: ["$.gone"],
      body_values_changed: [{ path: "$.user.name", was: "A", now: "B" }],
    });
    expect(md).toContain("$.new");
    expect(md).toContain("$.gone");
    expect(md).toContain("$.user.name");
  });

  it("renders a match message when the diff is empty", () => {
    const md = renderDiffSummary({});
    expect(md).toContain("Responses match");
  });
});
