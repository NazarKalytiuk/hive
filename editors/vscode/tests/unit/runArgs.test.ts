import { describe, it, expect } from "vitest";
import type { CancellationToken } from "vscode";
import { buildRunArgs } from "../../src/backend/runArgs";
import { normalizeCookieJarMode } from "../../src/config";
import type { RunOptions } from "../../src/backend/TarnBackend";

// Pure-helper tests for the `tarn run` argv builder. Exercises every
// branch of the NDJSON/non-NDJSON forks and proves that
// `tarn.cookieJarMode: per-test` threads `--cookie-jar-per-test` into
// every `tarn run` the extension spawns.

function stubToken(): CancellationToken {
  // buildRunArgs never touches the token; the cast lets us construct a
  // minimal RunOptions without pulling in the real vscode namespace.
  return {
    isCancellationRequested: false,
    onCancellationRequested: () => ({ dispose: () => {} }),
  } as unknown as CancellationToken;
}

function baseOptions(overrides: Partial<RunOptions> = {}): RunOptions {
  return {
    files: ["tests/health.tarn.yaml"],
    cwd: "/workspace",
    token: stubToken(),
    ...overrides,
  };
}

describe("buildRunArgs", () => {
  describe("non-NDJSON (stdout JSON) form", () => {
    it("builds the default argv for a single file", () => {
      const args = buildRunArgs(baseOptions(), undefined, "default");
      expect(args).toEqual([
        "run",
        "--format",
        "json",
        "--json-mode",
        "verbose",
        "--no-progress",
        "tests/health.tarn.yaml",
      ]);
    });

    it("honors jsonMode=compact", () => {
      const args = buildRunArgs(
        baseOptions({ jsonMode: "compact" }),
        undefined,
        "default",
      );
      expect(args).toContain("--json-mode");
      expect(args[args.indexOf("--json-mode") + 1]).toBe("compact");
    });

    it("appends --dry-run when dryRun is true", () => {
      const args = buildRunArgs(
        baseOptions({ dryRun: true }),
        undefined,
        "default",
      );
      expect(args).toContain("--dry-run");
    });

    it("appends --parallel when parallel is true", () => {
      const args = buildRunArgs(
        baseOptions({ parallel: true }),
        undefined,
        "default",
      );
      expect(args).toContain("--parallel");
    });

    it("omits --parallel when parallel is falsy", () => {
      const args = buildRunArgs(
        baseOptions({ parallel: false }),
        undefined,
        "default",
      );
      expect(args).not.toContain("--parallel");
    });

    it("threads --env when environment is set", () => {
      const args = buildRunArgs(
        baseOptions({ environment: "staging" }),
        undefined,
        "default",
      );
      const envIdx = args.indexOf("--env");
      expect(envIdx).toBeGreaterThanOrEqual(0);
      expect(args[envIdx + 1]).toBe("staging");
    });

    it("joins tags on a single --tag flag", () => {
      const args = buildRunArgs(
        baseOptions({ tags: ["smoke", "auth"] }),
        undefined,
        "default",
      );
      const tagIdx = args.indexOf("--tag");
      expect(args[tagIdx + 1]).toBe("smoke,auth");
    });

    it("emits one --select per selector", () => {
      const args = buildRunArgs(
        baseOptions({
          selectors: [
            "tests/a.tarn.yaml::login",
            "tests/a.tarn.yaml::search::0",
          ],
        }),
        undefined,
        "default",
      );
      const selectFlags = args.filter((a) => a === "--select");
      expect(selectFlags.length).toBe(2);
      expect(args).toContain("tests/a.tarn.yaml::login");
      expect(args).toContain("tests/a.tarn.yaml::search::0");
    });

    it("emits one --var per entry as KEY=VALUE", () => {
      const args = buildRunArgs(
        baseOptions({ vars: { base_url: "https://example.com", token: "abc" } }),
        undefined,
        "default",
      );
      expect(args).toContain("--var");
      expect(args).toContain("base_url=https://example.com");
      expect(args).toContain("token=abc");
    });

    it("places file paths last", () => {
      const args = buildRunArgs(
        baseOptions({ files: ["a.tarn.yaml", "b.tarn.yaml"] }),
        undefined,
        "default",
      );
      expect(args[args.length - 2]).toBe("a.tarn.yaml");
      expect(args[args.length - 1]).toBe("b.tarn.yaml");
    });
  });

  describe("NDJSON form", () => {
    it("uses --ndjson and --format json=<path> when a report path is supplied", () => {
      const args = buildRunArgs(
        baseOptions(),
        "/tmp/report.json",
        "default",
      );
      expect(args).toContain("--ndjson");
      const fmtIdx = args.indexOf("--format");
      expect(args[fmtIdx + 1]).toBe("json=/tmp/report.json");
      // --no-progress is only for the stdout-JSON form — NDJSON stream
      // handles its own progress reporting.
      expect(args).not.toContain("--no-progress");
    });

    it("still forwards --json-mode to the NDJSON final report", () => {
      const args = buildRunArgs(
        baseOptions({ jsonMode: "compact" }),
        "/tmp/report.json",
        "default",
      );
      const modeIdx = args.indexOf("--json-mode");
      expect(args[modeIdx + 1]).toBe("compact");
    });
  });

  describe("tarn.cookieJarMode integration (NAZ-280)", () => {
    it("omits --cookie-jar-per-test when mode is default (stdout form)", () => {
      const args = buildRunArgs(baseOptions(), undefined, "default");
      expect(args).not.toContain("--cookie-jar-per-test");
    });

    it("appends --cookie-jar-per-test when mode is per-test (stdout form)", () => {
      const args = buildRunArgs(baseOptions(), undefined, "per-test");
      expect(args).toContain("--cookie-jar-per-test");
    });

    it("appends --cookie-jar-per-test when mode is per-test (NDJSON form)", () => {
      const args = buildRunArgs(
        baseOptions(),
        "/tmp/report.json",
        "per-test",
      );
      expect(args).toContain("--cookie-jar-per-test");
    });

    it("applies the flag regardless of dry-run / selectors / env combos", () => {
      const args = buildRunArgs(
        baseOptions({
          dryRun: true,
          environment: "staging",
          selectors: ["tests/a.tarn.yaml::login"],
          tags: ["auth"],
          vars: { token: "abc" },
        }),
        undefined,
        "per-test",
      );
      expect(args).toContain("--cookie-jar-per-test");
      expect(args).toContain("--dry-run");
      expect(args).toContain("--env");
      expect(args).toContain("--select");
      expect(args).toContain("--tag");
      expect(args).toContain("--var");
    });

    it("places --cookie-jar-per-test before file paths so CLI parses it as a flag", () => {
      const args = buildRunArgs(
        baseOptions({ files: ["tests/cookie.tarn.yaml"] }),
        undefined,
        "per-test",
      );
      const flagIdx = args.indexOf("--cookie-jar-per-test");
      const fileIdx = args.indexOf("tests/cookie.tarn.yaml");
      expect(flagIdx).toBeGreaterThan(-1);
      expect(fileIdx).toBeGreaterThan(flagIdx);
    });
  });
});

describe("normalizeCookieJarMode", () => {
  it("returns 'per-test' for the exact per-test string", () => {
    expect(normalizeCookieJarMode("per-test")).toBe("per-test");
  });

  it("returns 'default' for the explicit default string", () => {
    expect(normalizeCookieJarMode("default")).toBe("default");
  });

  it("falls back to 'default' on undefined", () => {
    expect(normalizeCookieJarMode(undefined)).toBe("default");
  });

  it("falls back to 'default' on typos / unknown values", () => {
    // Users will mistype this at least once. The runner should never
    // crash on a bad setting — it should silently behave like default.
    expect(normalizeCookieJarMode("perTest")).toBe("default");
    expect(normalizeCookieJarMode("PER-TEST")).toBe("default");
    expect(normalizeCookieJarMode("")).toBe("default");
    expect(normalizeCookieJarMode("off")).toBe("default");
  });
});
