import { describe, it, expect } from "vitest";
import * as vscode from "vscode";
import { formatFailureMessage } from "../../src/notifications";
import type { Report } from "../../src/util/schemaGuards";

/**
 * EN-fallback acceptance test for NAZ-286.
 *
 * The `vscode.l10n.t(...)` API has a single, documented fallback
 * behavior: when the active locale has no translation for a key
 * (or no bundle is loaded at all), the API MUST return the
 * English source string verbatim. This is what makes the Phase 6
 * baseline load-bearing — translators can land new locales
 * incrementally, and any untranslated key just falls back to the
 * canonical English in the bundle instead of throwing or showing
 * an empty string.
 *
 * The unit-test harness here talks to the mocked `vscode` module
 * in `tests/unit/__mocks__/vscode.ts`, which faithfully
 * reproduces the production behavior: unknown keys are returned
 * unchanged, and `{N}` positional placeholders are substituted
 * from the remaining args. That mock is the test double the
 * runtime localization layer collapses to when no bundle is
 * available — exactly the shape Phase 6 relies on.
 *
 * We exercise the fallback end-to-end via
 * `formatFailureMessage`, which is the user-visible consumer
 * with the richest template matrix (singular/plural × three
 * file-count buckets), so drifting any branch of that switch
 * would trip both the direct `t()` assertions below and the
 * high-level formatter path.
 */
describe("vscode.l10n.t EN fallback (NAZ-286)", () => {
  it("returns the English key unchanged when no translation is available", () => {
    // Simulate a locale that has no translation entry for this
    // message — the production VS Code l10n layer returns the
    // English source verbatim in that case.
    expect(vscode.l10n.t("Tarn: file is valid.")).toBe("Tarn: file is valid.");
    expect(vscode.l10n.t("Select Tarn environment")).toBe(
      "Select Tarn environment",
    );
    expect(vscode.l10n.t("Run")).toBe("Run");
  });

  it("substitutes positional {N} placeholders when the key is English-only", () => {
    // Even without a translation, the `{0}`, `{1}`, ... arg
    // slots must be filled so numeric interpolation keeps working
    // on locales that happen not to be translated yet.
    expect(vscode.l10n.t("Tarn: {0} inline vars", 3)).toBe("Tarn: 3 inline vars");
    expect(
      vscode.l10n.t("Tarn: copied '--env {0}' to clipboard.", "staging"),
    ).toBe("Tarn: copied '--env staging' to clipboard.");
  });

  it("leaves keys with no matching placeholder untouched", () => {
    // If an arg is supplied but the key has no matching `{N}`
    // slot, the key text is returned as-is — same as the
    // production implementation.
    expect(vscode.l10n.t("Tarn: nothing to export.", "extra")).toBe(
      "Tarn: nothing to export.",
    );
  });

  it("formatFailureMessage EN fallback: singular, one file", () => {
    const report: Report = {
      schema_version: 1,
      version: "1",
      timestamp: "2026-04-10T12:00:00Z",
      duration_ms: 50,
      files: [
        {
          file: "tests/login.tarn.yaml",
          name: "login",
          status: "FAILED",
          duration_ms: 50,
          summary: { total: 1, passed: 0, failed: 1 },
          setup: [],
          tests: [],
          teardown: [],
        },
      ],
      summary: {
        files: 1,
        tests: 1,
        steps: { total: 1, passed: 0, failed: 1 },
        status: "FAILED",
      },
    };
    expect(formatFailureMessage(report)).toBe("Tarn: 1 failed step in login");
  });

  it("formatFailureMessage EN fallback: plural, no files", () => {
    const report: Report = {
      schema_version: 1,
      version: "1",
      timestamp: "2026-04-10T12:00:00Z",
      duration_ms: 50,
      files: [],
      summary: {
        files: 0,
        tests: 2,
        steps: { total: 2, passed: 0, failed: 2 },
        status: "FAILED",
      },
    };
    expect(formatFailureMessage(report)).toBe("Tarn: 2 failed steps");
  });

  it("formatFailureMessage EN fallback: plural, more than three files", () => {
    const report: Report = {
      schema_version: 1,
      version: "1",
      timestamp: "2026-04-10T12:00:00Z",
      duration_ms: 50,
      files: [
        {
          file: "a",
          name: "a",
          status: "FAILED",
          duration_ms: 1,
          summary: { total: 1, passed: 0, failed: 1 },
          setup: [],
          tests: [],
          teardown: [],
        },
        {
          file: "b",
          name: "b",
          status: "FAILED",
          duration_ms: 1,
          summary: { total: 1, passed: 0, failed: 1 },
          setup: [],
          tests: [],
          teardown: [],
        },
        {
          file: "c",
          name: "c",
          status: "FAILED",
          duration_ms: 1,
          summary: { total: 1, passed: 0, failed: 1 },
          setup: [],
          tests: [],
          teardown: [],
        },
        {
          file: "d",
          name: "d",
          status: "FAILED",
          duration_ms: 1,
          summary: { total: 1, passed: 0, failed: 1 },
          setup: [],
          tests: [],
          teardown: [],
        },
      ],
      summary: {
        files: 4,
        tests: 4,
        steps: { total: 4, passed: 0, failed: 4 },
        status: "FAILED",
      },
    };
    expect(formatFailureMessage(report)).toBe("Tarn: 4 failed steps across 4 files");
  });
});
