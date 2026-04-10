import { describe, it, expect } from "vitest";
import {
  customizeEnvFile,
  scaffoldFilesToPrune,
} from "../../src/commands/initProject";

const DEFAULT_ENV = `base_url: "http://localhost:3000"

# Optional credentials used by the example templates in ./examples/
admin_email: "admin@example.com"
admin_password: "secret"
alice_email: "alice@example.com"
alice_password: "secret"
bob_email: "bob@example.com"
bob_password: "secret"
`;

describe("customizeEnvFile", () => {
  it("returns the input unchanged when overrides is empty", () => {
    expect(customizeEnvFile(DEFAULT_ENV, {})).toBe(DEFAULT_ENV);
  });

  it("replaces known top-level keys in place", () => {
    const out = customizeEnvFile(DEFAULT_ENV, {
      base_url: "https://staging.example.com",
      admin_email: "ops@acme.dev",
    });
    // URLs and email addresses contain characters that would be
    // ambiguous in bare YAML (colon, @), so the rewriter quotes
    // them. That's the right default.
    expect(out).toContain('base_url: "https://staging.example.com"');
    expect(out).toContain('admin_email: "ops@acme.dev"');
    // Untouched keys are preserved verbatim.
    expect(out).toContain('alice_email: "alice@example.com"');
    // Comments stay.
    expect(out).toContain("# Optional credentials");
  });

  it("quotes values that would be mis-parsed as non-strings", () => {
    const out = customizeEnvFile("base_url: placeholder\n", {
      base_url: "value with spaces",
    });
    expect(out).toContain('base_url: "value with spaces"');
  });

  it("appends unknown keys in an annotated block at the end", () => {
    const out = customizeEnvFile(DEFAULT_ENV, {
      new_token: "xyz",
    });
    expect(out).toContain("# Added by Tarn: Init Project Here");
    expect(out).toMatch(/new_token:\s+xyz/);
    // Doesn't touch the existing block.
    expect(out).toContain('base_url: "http://localhost:3000"');
  });

  it("doesn't touch non-matching lines (comments, blanks, indented)", () => {
    const input = "# top comment\n\nbase_url: old\n  nested: foo\n";
    const out = customizeEnvFile(input, { base_url: "new" });
    expect(out).toContain("# top comment");
    expect(out).toContain("base_url: new");
    expect(out).toContain("  nested: foo");
  });

  it("escapes quotes and backslashes inside quoted values", () => {
    const out = customizeEnvFile("base_url: x\n", {
      base_url: 'a "q" and \\ back',
    });
    expect(out).toContain('base_url: "a \\"q\\" and \\\\ back"');
  });
});

describe("scaffoldFilesToPrune", () => {
  it("returns no paths for the 'all' flavor", () => {
    expect(scaffoldFilesToPrune("all")).toEqual([]);
  });

  it("prunes examples and fixtures for the 'basic' flavor", () => {
    expect(scaffoldFilesToPrune("basic")).toEqual(["examples", "fixtures"]);
  });
});
