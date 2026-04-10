import { describe, it, expect } from "vitest";
import {
  extractRedactionCaptures,
  isExpandable,
  renderRawValue,
} from "../../src/views/CapturesInspector";

describe("extractRedactionCaptures", () => {
  it("returns an empty list when the config is missing or non-object", () => {
    expect(extractRedactionCaptures(undefined)).toEqual([]);
    expect(extractRedactionCaptures(null)).toEqual([]);
    expect(extractRedactionCaptures(42)).toEqual([]);
    expect(extractRedactionCaptures("string")).toEqual([]);
  });

  it("returns an empty list when redaction is missing or malformed", () => {
    expect(extractRedactionCaptures({})).toEqual([]);
    expect(extractRedactionCaptures({ redaction: null })).toEqual([]);
    expect(extractRedactionCaptures({ redaction: "nope" })).toEqual([]);
    expect(extractRedactionCaptures({ redaction: { captures: "str" } })).toEqual([]);
  });

  it("extracts string keys from redaction.captures", () => {
    expect(
      extractRedactionCaptures({
        redaction: { captures: ["auth_token", "session_id"] },
      }),
    ).toEqual(["auth_token", "session_id"]);
  });

  it("skips non-string entries defensively", () => {
    expect(
      extractRedactionCaptures({
        redaction: { captures: ["ok", 123, null, { obj: true }] },
      }),
    ).toEqual(["ok"]);
  });
});

describe("isExpandable", () => {
  it("returns true for non-empty arrays and objects", () => {
    expect(isExpandable([1])).toBe(true);
    expect(isExpandable({ a: 1 })).toBe(true);
  });

  it("returns false for empty arrays and objects", () => {
    expect(isExpandable([])).toBe(false);
    expect(isExpandable({})).toBe(false);
  });

  it("returns false for scalars and null", () => {
    expect(isExpandable("abc")).toBe(false);
    expect(isExpandable(5)).toBe(false);
    expect(isExpandable(true)).toBe(false);
    expect(isExpandable(null)).toBe(false);
    expect(isExpandable(undefined)).toBe(false);
  });
});

describe("renderRawValue", () => {
  it("quotes strings and copies them unquoted to the clipboard", () => {
    const result = renderRawValue("hello");
    expect(result.description).toBe('"hello"');
    expect(result.full).toBe('"hello"');
    expect(result.clipboard).toBe("hello");
    expect(result.icon).toBe("symbol-string");
  });

  it("truncates long strings in the description", () => {
    const long = "x".repeat(200);
    const result = renderRawValue(long);
    expect(result.description.length).toBeLessThanOrEqual(80);
    expect(result.description.endsWith("…")).toBe(true);
    expect(result.clipboard).toBe(long);
  });

  it("renders numbers, booleans, and null cleanly", () => {
    expect(renderRawValue(42)).toMatchObject({ description: "42", clipboard: "42" });
    expect(renderRawValue(true)).toMatchObject({ description: "true", clipboard: "true" });
    expect(renderRawValue(null)).toMatchObject({ description: "null", clipboard: "null" });
  });

  it("renders arrays as [n] and copies JSON to the clipboard", () => {
    const result = renderRawValue([1, 2, 3]);
    expect(result.description).toBe("[3]");
    expect(result.clipboard).toBe("[1,2,3]");
    expect(result.icon).toBe("symbol-array");
  });

  it("renders objects as {n} and copies JSON to the clipboard", () => {
    const result = renderRawValue({ name: "Ada", age: 30 });
    expect(result.description).toBe("{2}");
    expect(result.clipboard).toBe('{"name":"Ada","age":30}');
    expect(result.icon).toBe("symbol-object");
  });
});
