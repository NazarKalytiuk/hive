import { describe, it, expect } from "vitest";
import {
  stringifyBody,
  detectLanguage,
  truncateBody,
} from "../../src/views/RequestResponsePanel";

describe("stringifyBody", () => {
  it("returns empty string for undefined/null", () => {
    expect(stringifyBody(undefined)).toBe("");
    expect(stringifyBody(null)).toBe("");
  });

  it("returns strings unchanged", () => {
    expect(stringifyBody("hello")).toBe("hello");
  });

  it("pretty-prints objects as indented JSON", () => {
    expect(stringifyBody({ a: 1, b: [2, 3] })).toBe(
      '{\n  "a": 1,\n  "b": [\n    2,\n    3\n  ]\n}',
    );
  });

  it("handles arrays", () => {
    expect(stringifyBody([1, 2, 3])).toBe("[\n  1,\n  2,\n  3\n]");
  });

  it("falls back to String() for circular objects", () => {
    const circular: { self?: unknown } = {};
    circular.self = circular;
    // Circular → JSON.stringify throws → falls back to String().
    expect(stringifyBody(circular)).toContain("[object Object]");
  });
});

describe("detectLanguage", () => {
  it("detects JSON objects", () => {
    expect(detectLanguage('{"a": 1}')).toBe("json");
    expect(detectLanguage('  { "spaced": true }\n')).toBe("json");
  });

  it("detects JSON arrays", () => {
    expect(detectLanguage("[1, 2, 3]")).toBe("json");
  });

  it("detects XML/HTML by the leading tag", () => {
    expect(detectLanguage('<?xml version="1.0"?><root/>')).toBe("xml");
    expect(detectLanguage("<html><body></body></html>")).toBe("xml");
  });

  it("falls back to plaintext for bare strings", () => {
    expect(detectLanguage("hello world")).toBe("plaintext");
    expect(detectLanguage("")).toBe("plaintext");
    expect(detectLanguage("   ")).toBe("plaintext");
  });
});

describe("truncateBody", () => {
  it("passes through content under the limit", () => {
    const result = truncateBody("short");
    expect(result).toEqual({ display: "short", truncated: false });
  });

  it("trims content over 10 KB", () => {
    const big = "x".repeat(20 * 1024);
    const result = truncateBody(big);
    expect(result.truncated).toBe(true);
    expect(result.display.length).toBe(10 * 1024);
  });

  it("exactly at the limit is not truncated", () => {
    const exact = "x".repeat(10 * 1024);
    const result = truncateBody(exact);
    expect(result.truncated).toBe(false);
  });
});
