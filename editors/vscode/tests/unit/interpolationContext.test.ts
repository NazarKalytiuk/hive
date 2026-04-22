import { describe, it, expect } from "vitest";
import {
  BUILTIN_FUNCTIONS,
  detectInterpolationContext,
  mergeEnvKeys,
} from "../../src/language/CompletionProvider";

describe("detectInterpolationContext", () => {
  it("returns none when cursor is not inside an interpolation", () => {
    expect(detectInterpolationContext("name: ping")).toEqual({ kind: "none" });
    expect(detectInterpolationContext("")).toEqual({ kind: "none" });
  });

  it("returns none when the interpolation is already closed before the cursor", () => {
    expect(detectInterpolationContext('url: "{{ env.base_url }}/foo')).toEqual({
      kind: "none",
    });
  });

  it("detects empty interpolation when user just opened {{", () => {
    expect(detectInterpolationContext('url: "{{')).toEqual({
      kind: "empty",
      prefix: "",
    });
    expect(detectInterpolationContext('url: "{{ ')).toEqual({
      kind: "empty",
      prefix: "",
    });
  });

  it("detects env context", () => {
    expect(detectInterpolationContext('url: "{{ env.')).toEqual({
      kind: "env",
      prefix: "",
    });
    expect(detectInterpolationContext('url: "{{ env.base')).toEqual({
      kind: "env",
      prefix: "base",
    });
    expect(detectInterpolationContext('url: "{{env.base_url')).toEqual({
      kind: "env",
      prefix: "base_url",
    });
  });

  it("detects env context with only 'env' typed", () => {
    expect(detectInterpolationContext('url: "{{ env')).toEqual({
      kind: "env",
      prefix: "",
    });
  });

  it("detects capture context", () => {
    expect(detectInterpolationContext('Authorization: "Bearer {{ capture.')).toEqual({
      kind: "capture",
      prefix: "",
    });
    expect(detectInterpolationContext('Authorization: "Bearer {{ capture.tok')).toEqual({
      kind: "capture",
      prefix: "tok",
    });
    expect(detectInterpolationContext("value: {{ capture")).toEqual({
      kind: "capture",
      prefix: "",
    });
  });

  it("detects builtin context", () => {
    expect(detectInterpolationContext('id: "{{ $')).toEqual({
      kind: "builtin",
      prefix: "",
    });
    expect(detectInterpolationContext('id: "{{ $uu')).toEqual({
      kind: "builtin",
      prefix: "uu",
    });
    expect(detectInterpolationContext('id: "{{$random_hex')).toEqual({
      kind: "builtin",
      prefix: "random_hex",
    });
  });

  it("returns none for unknown prefixes inside the interpolation", () => {
    expect(detectInterpolationContext('url: "{{ xyz')).toEqual({ kind: "none" });
    expect(detectInterpolationContext('url: "{{ 42')).toEqual({ kind: "none" });
  });

  it("handles nested braces inside YAML strings", () => {
    expect(
      detectInterpolationContext('body: {"name": "{{ env.user'),
    ).toEqual({ kind: "env", prefix: "user" });
  });
});

describe("mergeEnvKeys", () => {
  it("returns an empty map for no entries", () => {
    expect(mergeEnvKeys([]).size).toBe(0);
  });

  it("tracks which environments declare each key", () => {
    const merged = mergeEnvKeys([
      {
        name: "staging",
        source_file: "tarn.env.staging.yaml",
        vars: { base_url: "https://staging", api_token: "s" },
      },
      {
        name: "production",
        source_file: "tarn.env.production.yaml",
        vars: { base_url: "https://prod", api_token: "p" },
      },
      {
        name: "local",
        source_file: "tarn.env.local.yaml",
        vars: { debug: "true" },
      },
    ]);
    expect(merged.get("base_url")).toEqual(["staging", "production"]);
    expect(merged.get("api_token")).toEqual(["staging", "production"]);
    expect(merged.get("debug")).toEqual(["local"]);
    expect(merged.size).toBe(3);
  });
});

describe("BUILTIN_FUNCTIONS", () => {
  it("matches the Tarn interpolation runtime list", () => {
    const names = BUILTIN_FUNCTIONS.map((b) => b.name).sort();
    expect(names).toEqual([
      "$alnum",
      "$alpha",
      "$bool",
      "$choice",
      "$email",
      "$first_name",
      "$ipv4",
      "$ipv6",
      "$last_name",
      "$name",
      "$now_iso",
      "$phone",
      "$random_hex",
      "$random_int",
      "$sentence",
      "$slug",
      "$timestamp",
      "$username",
      "$uuid",
      "$uuid_v4",
      "$uuid_v7",
      "$word",
      "$words",
    ]);
  });

  it("every builtin has a signature and docs", () => {
    for (const fn of BUILTIN_FUNCTIONS) {
      expect(fn.signature.length).toBeGreaterThan(0);
      expect(fn.doc.length).toBeGreaterThan(0);
      expect(fn.insertText.length).toBeGreaterThan(0);
    }
  });
});
