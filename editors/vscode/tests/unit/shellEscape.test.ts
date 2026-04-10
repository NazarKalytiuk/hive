import { describe, it, expect } from "vitest";
import { quoteArgForLog, formatCommandForLog } from "../../src/util/shellEscape";

describe("quoteArgForLog", () => {
  it("returns empty quotes for empty string", () => {
    expect(quoteArgForLog("")).toBe("''");
  });

  it("leaves safe identifiers unquoted", () => {
    expect(quoteArgForLog("tarn")).toBe("tarn");
    expect(quoteArgForLog("--format")).toBe("--format");
    expect(quoteArgForLog("tests/users.tarn.yaml")).toBe("tests/users.tarn.yaml");
    expect(quoteArgForLog("KEY=value")).toBe("KEY=value");
    expect(quoteArgForLog("http://localhost:3000")).toBe("http://localhost:3000");
  });

  it("quotes args with spaces", () => {
    expect(quoteArgForLog("hello world")).toBe("'hello world'");
  });

  it("escapes embedded single quotes", () => {
    expect(quoteArgForLog("it's ok")).toBe(`'it'\\''s ok'`);
  });

  it("quotes args with dollar signs and backticks", () => {
    expect(quoteArgForLog("$PATH")).toBe("'$PATH'");
    expect(quoteArgForLog("`whoami`")).toBe("'`whoami`'");
  });
});

describe("formatCommandForLog", () => {
  it("joins binary and args", () => {
    expect(formatCommandForLog("tarn", ["run", "--format", "json"])).toBe(
      "tarn run --format json",
    );
  });

  it("quotes arguments that need it", () => {
    expect(formatCommandForLog("tarn", ["run", "--var", "KEY=hello world"])).toBe(
      "tarn run --var 'KEY=hello world'",
    );
  });
});
