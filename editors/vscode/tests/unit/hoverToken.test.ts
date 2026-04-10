import { describe, it, expect } from "vitest";
import { findHoverToken } from "../../src/language/completion/hoverToken";

describe("findHoverToken", () => {
  it("returns none when the cursor is outside any interpolation", () => {
    expect(findHoverToken("url: http://localhost/ping", 5)).toEqual({ kind: "none" });
  });

  it("returns none for an empty line", () => {
    expect(findHoverToken("", 0)).toEqual({ kind: "none" });
  });

  it("returns none when the cursor is before a {{", () => {
    const line = '  url: "{{ env.base }}"';
    expect(findHoverToken(line, 0)).toEqual({ kind: "none" });
  });

  it("returns none when the cursor is after the closing }}", () => {
    const line = '  url: "{{ env.base }}/extra"';
    const after = line.indexOf("/extra") + 3;
    expect(findHoverToken(line, after)).toEqual({ kind: "none" });
  });

  it("identifies an env hover when the cursor is on the key", () => {
    const line = '  url: "{{ env.base_url }}/health"';
    const cursor = line.indexOf("base_url") + 4;
    const token = findHoverToken(line, cursor);
    expect(token.kind).toBe("env");
    if (token.kind !== "env") return;
    expect(token.identifier).toBe("base_url");
    expect(token.rangeStart).toBe(line.indexOf("{{"));
    expect(token.rangeEnd).toBe(line.indexOf("}}") + 2);
  });

  it("identifies an env hover when the cursor is on `env` itself", () => {
    const line = '"{{ env.base_url }}"';
    const cursor = line.indexOf("env") + 1;
    const token = findHoverToken(line, cursor);
    expect(token.kind).toBe("env");
    if (token.kind !== "env") return;
    expect(token.identifier).toBe("base_url");
  });

  it("identifies a capture hover", () => {
    const line = 'Authorization: "Bearer {{ capture.auth_token }}"';
    const cursor = line.indexOf("auth_token") + 3;
    const token = findHoverToken(line, cursor);
    expect(token.kind).toBe("capture");
    if (token.kind !== "capture") return;
    expect(token.identifier).toBe("auth_token");
  });

  it("identifies a builtin hover for $uuid", () => {
    const line = 'id: "{{ $uuid }}"';
    const cursor = line.indexOf("uuid") + 2;
    const token = findHoverToken(line, cursor);
    expect(token.kind).toBe("builtin");
    if (token.kind !== "builtin") return;
    expect(token.identifier).toBe("uuid");
  });

  it("identifies a builtin hover for $random_hex(n) and strips the arg list", () => {
    const line = 'id: "{{ $random_hex(8) }}"';
    const cursor = line.indexOf("random_hex") + 4;
    const token = findHoverToken(line, cursor);
    expect(token.kind).toBe("builtin");
    if (token.kind !== "builtin") return;
    expect(token.identifier).toBe("random_hex");
  });

  it("returns empty when the cursor is inside an empty interpolation", () => {
    const line = 'id: "{{ }}"';
    const cursor = line.indexOf("{{") + 2;
    const token = findHoverToken(line, cursor);
    expect(token.kind).toBe("empty");
  });

  it("handles multiple interpolations on one line", () => {
    const line = 'url: "{{ env.base }}/{{ capture.id }}/extra"';
    const captureCursor = line.indexOf("capture") + 4;
    const token = findHoverToken(line, captureCursor);
    expect(token.kind).toBe("capture");
    if (token.kind !== "capture") return;
    expect(token.identifier).toBe("id");
    // Range should be the capture interpolation, not the env one.
    expect(line.slice(token.rangeStart, token.rangeEnd)).toBe("{{ capture.id }}");
  });

  it("returns none for unknown expressions inside the interpolation", () => {
    const line = 'value: "{{ nonsense }}"';
    const cursor = line.indexOf("nonsense") + 2;
    expect(findHoverToken(line, cursor)).toEqual({ kind: "none" });
  });

  it("returns none when the {{ has no matching }}", () => {
    const line = 'url: "{{ env.base';
    const cursor = line.indexOf("env") + 1;
    expect(findHoverToken(line, cursor)).toEqual({ kind: "none" });
  });
});
