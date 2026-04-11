import { describe, it, expect } from "vitest";
import * as fs from "node:fs";
import * as path from "node:path";

// CI lint for the localization baseline (NAZ-286).
//
// Fails the build if any user-facing VS Code API call site in
// `src` is handed a hardcoded string literal instead of routing
// through `vscode.l10n.t(...)`. This is the enforcement gate for
// the "English catalog" acceptance criterion: if a new quickPick
// placeholder, notification message, TreeItem label, etc. slips
// in as a literal, this test catches it before the string can
// ship un-translatable.
//
// Rules:
//
//   - Scans every `.ts` under `src/`.
//   - For each line matching one of the flagged APIs, checks the
//     first argument. If the argument is a bare string literal
//     (single-, double-, or backtick-quoted with no substitutions)
//     the line is reported.
//   - A line containing the marker comment `// l10n-ignore` is
//     exempt. The marker must live on the same line as the call
//     or on the line immediately above, so exceptions stay local
//     and greppable. The test also honors a file-local marker
//     `l10n-ignore-file` (whole file exempt — used by the test
//     harness itself).
//   - Calls where the first argument is already `vscode.l10n.t(...)`
//     (or a variable/identifier) are not flagged.
//
// Flagged APIs (all user-facing):
//
//   - `vscode.window.showInformationMessage(`
//   - `vscode.window.showWarningMessage(`
//   - `vscode.window.showErrorMessage(`
//   - Object-literal keys that are known user-visible slots:
//     `placeHolder`, `prompt`, `saveLabel`, `openLabel`.
//
// Debug/engineer-facing log lines (`out.appendLine`,
// `console.log`, `console.error`, `console.warn`) are
// intentionally NOT flagged: they render inside the Tarn output
// channel for engineers tailing logs, and the ticket explicitly
// lists them as an exception.

interface Violation {
  file: string;
  line: number;
  snippet: string;
  reason: string;
}

const SRC_ROOT = path.resolve(__dirname, "../../src");

function collectTsFiles(dir: string, out: string[] = []): string[] {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const abs = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      collectTsFiles(abs, out);
    } else if (entry.isFile() && entry.name.endsWith(".ts")) {
      out.push(abs);
    }
  }
  return out;
}

/**
 * Does the call site's first argument look like a bare string
 * literal? We only flag literals the lint is 100% sure about:
 *
 *   - `"..."`, `'...'`, or `` `...` `` with no `${}` interpolation
 *
 * Anything else (an identifier, a conditional, a call to
 * `vscode.l10n.t`) is considered safe and left alone. This keeps
 * the lint from chasing dynamic strings we can't reason about
 * statically.
 */
function firstArgIsLiteral(argText: string): boolean {
  const trimmed = argText.trimStart();
  if (trimmed.length === 0) return false;
  const quote = trimmed[0];
  if (quote !== '"' && quote !== "'" && quote !== "`") return false;
  // Scan for the matching close quote, ignoring escaped quotes.
  let i = 1;
  while (i < trimmed.length) {
    const c = trimmed[i];
    if (c === "\\") {
      i += 2;
      continue;
    }
    if (quote === "`" && c === "$" && trimmed[i + 1] === "{") {
      // Template with an interpolation — we can't reliably flag
      // these because the author may be concatenating a literal
      // prefix with a dynamic value. Skip.
      return false;
    }
    if (c === quote) {
      return true;
    }
    i++;
  }
  return false;
}

/**
 * Extract the substring between the `(` immediately following
 * `needle` and its matching closing `)`. Handles nested parens,
 * strings, template literals, and block comments.
 */
function extractCallArgs(src: string, callStart: number): string | undefined {
  let i = callStart;
  while (i < src.length && src[i] !== "(") i++;
  if (i >= src.length) return undefined;
  const start = i + 1;
  let depth = 1;
  i = start;
  while (i < src.length && depth > 0) {
    const c = src[i];
    if (c === "(" || c === "{" || c === "[") {
      depth++;
      i++;
      continue;
    }
    if (c === ")" || c === "}" || c === "]") {
      depth--;
      i++;
      continue;
    }
    if (c === '"' || c === "'") {
      i++;
      while (i < src.length) {
        if (src[i] === "\\") {
          i += 2;
          continue;
        }
        if (src[i] === c) {
          i++;
          break;
        }
        i++;
      }
      continue;
    }
    if (c === "`") {
      i++;
      while (i < src.length) {
        if (src[i] === "\\") {
          i += 2;
          continue;
        }
        if (src[i] === "`") {
          i++;
          break;
        }
        if (src[i] === "$" && src[i + 1] === "{") {
          // Skip the interpolation expression with another
          // balance pass so nested backticks in `${}` don't
          // terminate the outer template.
          i += 2;
          let tDepth = 1;
          while (i < src.length && tDepth > 0) {
            if (src[i] === "{") tDepth++;
            else if (src[i] === "}") tDepth--;
            i++;
          }
          continue;
        }
        i++;
      }
      continue;
    }
    if (c === "/" && src[i + 1] === "/") {
      while (i < src.length && src[i] !== "\n") i++;
      continue;
    }
    if (c === "/" && src[i + 1] === "*") {
      i += 2;
      while (i < src.length - 1 && !(src[i] === "*" && src[i + 1] === "/")) i++;
      i += 2;
      continue;
    }
    i++;
  }
  if (depth !== 0) return undefined;
  return src.slice(start, i - 1);
}

function firstTopLevelArg(argsText: string): string {
  let depth = 0;
  let i = 0;
  while (i < argsText.length) {
    const c = argsText[i];
    if (c === "(" || c === "{" || c === "[") {
      depth++;
      i++;
      continue;
    }
    if (c === ")" || c === "}" || c === "]") {
      depth--;
      i++;
      continue;
    }
    if (c === "," && depth === 0) {
      return argsText.slice(0, i);
    }
    if (c === '"' || c === "'") {
      i++;
      while (i < argsText.length) {
        if (argsText[i] === "\\") {
          i += 2;
          continue;
        }
        if (argsText[i] === c) {
          i++;
          break;
        }
        i++;
      }
      continue;
    }
    if (c === "`") {
      i++;
      while (i < argsText.length) {
        if (argsText[i] === "\\") {
          i += 2;
          continue;
        }
        if (argsText[i] === "`") {
          i++;
          break;
        }
        if (argsText[i] === "$" && argsText[i + 1] === "{") {
          i += 2;
          let tDepth = 1;
          while (i < argsText.length && tDepth > 0) {
            if (argsText[i] === "{") tDepth++;
            else if (argsText[i] === "}") tDepth--;
            i++;
          }
          continue;
        }
        i++;
      }
      continue;
    }
    i++;
  }
  return argsText;
}

function lineNumberAt(src: string, offset: number): number {
  let line = 1;
  for (let i = 0; i < offset; i++) if (src[i] === "\n") line++;
  return line;
}

function lineText(src: string, lineNumber: number): string {
  const lines = src.split("\n");
  return lines[lineNumber - 1] ?? "";
}

function hasIgnoreMarker(src: string, lineNumber: number): boolean {
  const lines = src.split("\n");
  const current = lines[lineNumber - 1] ?? "";
  if (current.includes("l10n-ignore")) return true;
  const prev = lines[lineNumber - 2] ?? "";
  if (prev.trim().startsWith("//") && prev.includes("l10n-ignore")) return true;
  return false;
}

/**
 * Flag call sites where the first positional argument is a bare
 * literal. Returns one violation per offending call.
 */
function lintPositionalCalls(
  src: string,
  file: string,
  needles: readonly string[],
  reasonPrefix: string,
  violations: Violation[],
): void {
  for (const needle of needles) {
    let idx = 0;
    while ((idx = src.indexOf(needle, idx)) !== -1) {
      // Skip matches inside comments or string literals by checking
      // the character right before the needle — if it's inside a
      // comment block we'd have seen a `//` on the same line.
      const lineNumber = lineNumberAt(src, idx);
      const text = lineText(src, lineNumber);
      const needleColInLine = text.indexOf(needle);
      if (needleColInLine >= 0) {
        const before = text.slice(0, needleColInLine);
        if (before.includes("//")) {
          idx += needle.length;
          continue;
        }
      }
      const args = extractCallArgs(src, idx + needle.length - 1);
      if (args === undefined) {
        idx += needle.length;
        continue;
      }
      const firstArg = firstTopLevelArg(args);
      if (firstArgIsLiteral(firstArg) && !hasIgnoreMarker(src, lineNumber)) {
        violations.push({
          file,
          line: lineNumber,
          snippet: text.trim(),
          reason: `${reasonPrefix} must wrap the first argument in vscode.l10n.t(...)`,
        });
      }
      idx += needle.length;
    }
  }
}

/**
 * Flag object-literal key assignments where the value is a bare
 * literal. This catches QuickPickItem labels, TreeItem tooltips,
 * InputBoxOptions prompts, etc.
 */
function lintObjectKeys(
  src: string,
  file: string,
  keys: readonly string[],
  violations: Violation[],
): void {
  const pattern = new RegExp(
    `(^|[\\s{,(])(${keys.join("|")})\\s*:\\s*("(?:[^"\\\\]|\\\\.)*"|'(?:[^'\\\\]|\\\\.)*'|\`[^\`$]*\`)(?=\\s*[,}\\n])`,
    "gm",
  );
  let match: RegExpExecArray | null;
  while ((match = pattern.exec(src)) !== null) {
    const valueStart = match.index + match[0].indexOf(match[3]);
    const lineNumber = lineNumberAt(src, valueStart);
    const text = lineText(src, lineNumber);
    // Skip type annotations (`placeHolder: string`) and comment
    // lines — the regex's positive lookbehind-ish guard already
    // caught leading separators, but comments slip through.
    if (text.trim().startsWith("//")) continue;
    if (text.trim().startsWith("*")) continue; // JSDoc line
    if (hasIgnoreMarker(src, lineNumber)) continue;
    violations.push({
      file,
      line: lineNumber,
      snippet: text.trim(),
      reason: `object key "${match[2]}:" must use vscode.l10n.t(...) instead of a literal`,
    });
  }
}

describe("l10n lint: user-facing strings must go through vscode.l10n.t", () => {
  const files = collectTsFiles(SRC_ROOT);

  it("flags hardcoded literals passed to user-visible VS Code APIs", () => {
    const violations: Violation[] = [];
    for (const file of files) {
      const src = fs.readFileSync(file, "utf8");
      if (src.includes("l10n-ignore-file")) continue;
      lintPositionalCalls(
        src,
        file,
        [
          "vscode.window.showInformationMessage(",
          "vscode.window.showWarningMessage(",
          "vscode.window.showErrorMessage(",
        ],
        "user-visible notification",
        violations,
      );
      lintObjectKeys(
        src,
        file,
        [
          "placeHolder",
          "prompt",
          "saveLabel",
          "openLabel",
        ],
        violations,
      );
    }

    if (violations.length > 0) {
      const rendered = violations
        .map(
          (v) =>
            `  ${path.relative(SRC_ROOT, v.file)}:${v.line} → ${v.reason}\n    ${v.snippet}`,
        )
        .join("\n");
      throw new Error(
        `Found ${violations.length} hardcoded user-visible string(s):\n${rendered}\n\n` +
          `Wrap each literal in vscode.l10n.t("...") so it can be localized, ` +
          `or add a // l10n-ignore comment on the same line if the string is ` +
          `intentionally engineer-facing.`,
      );
    }
    expect(violations).toEqual([]);
  });

  /**
   * Self-check: the lint helpers themselves must trip on a
   * minimal synthetic file. If `firstArgIsLiteral` or the key
   * regex silently drifts into returning false, this canary fails
   * and the suite breaks loudly instead of degrading into a
   * no-op that lets real violations slip through.
   */
  it("canary: a synthetic hardcoded call site is detected", () => {
    const fake = `
      import * as vscode from "vscode";
      export async function bad() {
        await vscode.window.showInformationMessage("literal");
        await vscode.window.showQuickPick([{ label: "x" }], { placeHolder: "pick one" });
      }
    `;
    const violations: Violation[] = [];
    lintPositionalCalls(
      fake,
      "canary.ts",
      ["vscode.window.showInformationMessage("],
      "notification",
      violations,
    );
    lintObjectKeys(fake, "canary.ts", ["placeHolder", "prompt"], violations);
    expect(violations.length).toBeGreaterThanOrEqual(2);
    expect(violations[0].reason).toMatch(/notification/);
    expect(violations.some((v) => /placeHolder/.test(v.reason))).toBe(true);
  });

  /**
   * Self-check: the `// l10n-ignore` escape hatch must take a
   * line out of the lint. Covers both same-line and previous-line
   * placement so authors can annotate either way.
   */
  it("canary: l10n-ignore comment suppresses a violation", () => {
    const fake = `
      import * as vscode from "vscode";
      export async function ok() {
        // l10n-ignore: debug toast only.
        await vscode.window.showInformationMessage("debug");
        await vscode.window.showInformationMessage("debug-inline"); // l10n-ignore
      }
    `;
    const violations: Violation[] = [];
    lintPositionalCalls(
      fake,
      "canary.ts",
      ["vscode.window.showInformationMessage("],
      "notification",
      violations,
    );
    expect(violations).toEqual([]);
  });

  /**
   * Self-check: calls that already go through `vscode.l10n.t`
   * are left alone. A vanilla literal wrapped in `l10n.t()` must
   * NOT trip the first-positional-arg lint.
   */
  it("canary: already-wrapped l10n.t calls are clean", () => {
    const fake = `
      import * as vscode from "vscode";
      export async function ok() {
        await vscode.window.showInformationMessage(vscode.l10n.t("hi"));
        await vscode.window.showQuickPick([], { placeHolder: vscode.l10n.t("pick") });
      }
    `;
    const violations: Violation[] = [];
    lintPositionalCalls(
      fake,
      "canary.ts",
      ["vscode.window.showInformationMessage("],
      "notification",
      violations,
    );
    lintObjectKeys(fake, "canary.ts", ["placeHolder"], violations);
    expect(violations).toEqual([]);
  });
});

/**
 * Separate suite: the EN catalog has one entry per unique
 * literal passed to `vscode.l10n.t(...)`. Keeping the bundle in
 * sync is the other half of the Phase 6 contract — if a refactor
 * drops a string from the source but leaves the key in the
 * bundle (or vice versa), the catalog drifts and translators
 * see ghost keys. This test refuses both directions.
 */
describe("l10n bundle: every t() literal has a matching EN entry", () => {
  const bundlePath = path.resolve(__dirname, "../../l10n/bundle.l10n.json");
  const bundle = JSON.parse(fs.readFileSync(bundlePath, "utf8")) as Record<
    string,
    string
  >;
  const files = collectTsFiles(SRC_ROOT);

  function collectSourceKeys(): Set<string> {
    const keys = new Set<string>();
    for (const file of files) {
      const src = fs.readFileSync(file, "utf8");
      if (src.includes("l10n-ignore-file")) continue;
      let idx = 0;
      const needle = "vscode.l10n.t(";
      while ((idx = src.indexOf(needle, idx)) !== -1) {
        const args = extractCallArgs(src, idx + needle.length - 1);
        if (args === undefined) {
          idx += needle.length;
          continue;
        }
        const firstArg = firstTopLevelArg(args);
        const trimmed = firstArg.trimStart();
        const quote = trimmed[0];
        if (quote === '"' || quote === "'" || quote === "`") {
          const value = parseLiteral(trimmed);
          if (value !== null) keys.add(value);
        }
        idx += needle.length;
      }
    }
    return keys;
  }

  function parseLiteral(literal: string): string | null {
    const quote = literal[0];
    let out = "";
    let i = 1;
    while (i < literal.length) {
      const c = literal[i];
      if (c === "\\") {
        const next = literal[i + 1];
        if (next === undefined) return null;
        if (next === "n") out += "\n";
        else if (next === "t") out += "\t";
        else if (next === "r") out += "\r";
        else out += next;
        i += 2;
        continue;
      }
      if (c === quote) return out;
      if (quote === "`" && c === "$" && literal[i + 1] === "{") return null;
      out += c;
      i++;
    }
    return null;
  }

  it("every t() literal has an EN entry (no source → bundle drift)", () => {
    const sourceKeys = collectSourceKeys();
    const missing = [...sourceKeys].filter((k) => !(k in bundle)).sort();
    expect(missing).toEqual([]);
  });

  it("every EN entry is used by at least one t() call (no stale bundle entries)", () => {
    const sourceKeys = collectSourceKeys();
    const stale = Object.keys(bundle)
      .filter((k) => !sourceKeys.has(k))
      .sort();
    expect(stale).toEqual([]);
  });

  it("bundle keys match their EN values (identity baseline)", () => {
    for (const [key, value] of Object.entries(bundle)) {
      expect(value).toBe(key);
    }
  });
});
