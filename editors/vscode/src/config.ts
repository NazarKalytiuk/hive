import * as vscode from "vscode";

export type CookieJarMode = "default" | "per-test";

export interface TarnConfig {
  binaryPath: string;
  testFileGlob: string;
  excludeGlobs: string[];
  defaultEnvironment: string | null;
  defaultTags: string[];
  parallel: boolean;
  jsonMode: "verbose" | "compact";
  requestTimeoutMs: number;
  showCodeLens: boolean;
  statusBarEnabled: boolean;
  validateOnSave: boolean;
  notificationsFailure: "always" | "focused" | "off";
  cookieJarMode: CookieJarMode;
}

export function readConfig(scope?: vscode.Uri): TarnConfig {
  const cfg = vscode.workspace.getConfiguration("tarn", scope);
  return {
    binaryPath: cfg.get<string>("binaryPath", "tarn"),
    testFileGlob: cfg.get<string>("testFileGlob", "**/*.tarn.yaml"),
    excludeGlobs: cfg.get<string[]>("excludeGlobs", [
      "**/target/**",
      "**/node_modules/**",
      "**/.git/**",
    ]),
    defaultEnvironment: cfg.get<string | null>("defaultEnvironment", null),
    defaultTags: cfg.get<string[]>("defaultTags", []),
    parallel: cfg.get<boolean>("parallel", true),
    jsonMode: cfg.get<"verbose" | "compact">("jsonMode", "verbose"),
    requestTimeoutMs: cfg.get<number>("requestTimeoutMs", 120000),
    showCodeLens: cfg.get<boolean>("showCodeLens", true),
    statusBarEnabled: cfg.get<boolean>("statusBar.enabled", true),
    validateOnSave: cfg.get<boolean>("validateOnSave", true),
    notificationsFailure: cfg.get<"always" | "focused" | "off">(
      "notifications.failure",
      "focused",
    ),
    cookieJarMode: normalizeCookieJarMode(
      cfg.get<string>("cookieJarMode", "default"),
    ),
  };
}

/**
 * Narrow a raw `tarn.cookieJarMode` value to a known mode. Unknown or
 * malformed values fall back to `"default"` so a typo in user settings
 * never breaks the runner — the worst case is honoring the file's
 * declared `cookies:` mode, which is the safe default.
 */
export function normalizeCookieJarMode(raw: string | undefined): CookieJarMode {
  return raw === "per-test" ? "per-test" : "default";
}

export function buildExcludeGlob(globs: string[]): string | undefined {
  if (globs.length === 0) {
    return undefined;
  }
  if (globs.length === 1) {
    return globs[0];
  }
  return `{${globs.join(",")}}`;
}
