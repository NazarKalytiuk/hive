import { describe, it, expect } from "vitest";
import * as fs from "fs";
import * as path from "path";

const GRAMMAR_PATH = path.resolve(
  __dirname,
  "../../syntaxes/tarn.tmLanguage.json",
);

interface TmPattern {
  name?: string;
  match?: string;
  begin?: string;
  end?: string;
  patterns?: TmPattern[];
  captures?: Record<string, { name?: string }>;
  beginCaptures?: Record<string, { name?: string }>;
  endCaptures?: Record<string, { name?: string }>;
  include?: string;
}

interface TmGrammar {
  name: string;
  scopeName: string;
  patterns: TmPattern[];
  repository: Record<string, TmPattern>;
}

function loadGrammar(): TmGrammar {
  const raw = fs.readFileSync(GRAMMAR_PATH, "utf8");
  return JSON.parse(raw) as TmGrammar;
}

/**
 * Walk every scope name mentioned in a pattern (its own `name`, plus
 * all `captures` / `beginCaptures` / `endCaptures` entries). Returns a
 * flat list of all scope names found in the entire grammar.
 */
function collectScopeNames(grammar: TmGrammar): string[] {
  const scopes: string[] = [];
  const visit = (pattern: TmPattern): void => {
    if (pattern.name) scopes.push(pattern.name);
    for (const captures of [
      pattern.captures,
      pattern.beginCaptures,
      pattern.endCaptures,
    ]) {
      if (!captures) continue;
      for (const entry of Object.values(captures)) {
        if (entry?.name) scopes.push(entry.name);
      }
    }
    for (const child of pattern.patterns ?? []) {
      visit(child);
    }
  };
  for (const p of grammar.patterns) visit(p);
  for (const p of Object.values(grammar.repository)) visit(p);
  return scopes;
}

/** Flatten "foo.bar baz.qux" space-separated scopes into individual names. */
function flatten(scopes: string[]): string[] {
  const out: string[] = [];
  for (const s of scopes) {
    for (const name of s.split(/\s+/).filter((n) => n.length > 0)) {
      out.push(name);
    }
  }
  return out;
}

describe("tarn.tmLanguage.json grammar", () => {
  const grammar = loadGrammar();
  const scopes = flatten(collectScopeNames(grammar));
  const scopeSet = new Set(scopes);

  it("has the expected top-level metadata", () => {
    expect(grammar.name).toBe("Tarn");
    expect(grammar.scopeName).toBe("source.tarn");
  });

  it("declares meta.template.tarn as the interpolation scope", () => {
    expect(scopeSet.has("meta.template.tarn")).toBe(true);
  });

  it("declares keyword.control.template.begin/end.tarn on the delimiters", () => {
    expect(scopeSet.has("keyword.control.template.begin.tarn")).toBe(true);
    expect(scopeSet.has("keyword.control.template.end.tarn")).toBe(true);
  });

  it("keeps the standard punctuation.definition.template.* scopes for theme compatibility", () => {
    expect(scopeSet.has("punctuation.definition.template.begin.tarn")).toBe(true);
    expect(scopeSet.has("punctuation.definition.template.end.tarn")).toBe(true);
  });

  it("declares variable.other.env.tarn for env.x references", () => {
    expect(scopeSet.has("variable.other.env.tarn")).toBe(true);
  });

  it("declares variable.other.capture.tarn for capture.y references", () => {
    expect(scopeSet.has("variable.other.capture.tarn")).toBe(true);
  });

  it("declares support.function.builtin.tarn for built-in functions", () => {
    expect(scopeSet.has("support.function.builtin.tarn")).toBe(true);
  });

  function findRuleByCaptureScope(
    interpolation: TmPattern,
    scopeFragment: string,
  ): TmPattern | undefined {
    return interpolation.patterns![0].patterns!.find((p) => {
      if (!p.captures) return false;
      for (const entry of Object.values(p.captures)) {
        if (entry?.name?.includes(scopeFragment)) return true;
      }
      return false;
    });
  }

  it("the env rule matches typed env keys and captures them as two groups", () => {
    const interpolation = grammar.repository.interpolation;
    const envRule = findRuleByCaptureScope(interpolation, "variable.other.env.tarn");
    expect(envRule).toBeDefined();
    expect(envRule!.captures).toBeDefined();
    expect(envRule!.captures!["1"]?.name).toContain("variable.other.env.tarn");
    expect(envRule!.captures!["2"]?.name).toContain("variable.other.env.tarn");

    const regex = new RegExp(envRule!.match!);
    const match = regex.exec("env.base_url");
    expect(match).not.toBeNull();
    expect(match![1]).toBe("env");
    expect(match![2]).toBe("base_url");
  });

  it("the capture rule matches typed capture names", () => {
    const interpolation = grammar.repository.interpolation;
    const captureRule = findRuleByCaptureScope(
      interpolation,
      "variable.other.capture.tarn",
    );
    expect(captureRule).toBeDefined();
    const regex = new RegExp(captureRule!.match!);
    const match = regex.exec("capture.auth_token");
    expect(match).not.toBeNull();
    expect(match![1]).toBe("capture");
    expect(match![2]).toBe("auth_token");
  });

  it("the builtin rule matches every Tarn runtime function", () => {
    const interpolation = grammar.repository.interpolation;
    const builtinRule = interpolation.patterns![0].patterns!.find((p) =>
      p.match?.startsWith("\\$"),
    );
    expect(builtinRule).toBeDefined();
    const regex = new RegExp(builtinRule!.match!);
    for (const name of ["$uuid", "$timestamp", "$now_iso", "$random_hex", "$random_int"]) {
      expect(regex.exec(name), `expected ${name} to match`).not.toBeNull();
    }
    expect(regex.exec("$made_up")).toBeNull();
  });
});
