import type { CookieJarMode } from "../config";
import type { RunOptions } from "./TarnBackend";

/**
 * Pure helper that builds the argv for a `tarn run` invocation. Kept
 * separate from `TarnProcessRunner` so the argv is exercised by unit
 * tests without spawning a process.
 *
 * `ndjsonReportPath` is set when the runner is streaming NDJSON and
 * writing the final JSON report to disk, and left `undefined` when the
 * runner is collecting JSON from stdout instead.
 */
export function buildRunArgs(
  options: RunOptions,
  ndjsonReportPath: string | undefined,
  cookieJarMode: CookieJarMode,
): string[] {
  const args: string[] = ["run"];
  if (ndjsonReportPath) {
    args.push("--ndjson");
    args.push("--format", `json=${ndjsonReportPath}`);
    args.push("--json-mode", options.jsonMode ?? "verbose");
  } else {
    args.push("--format", "json");
    args.push("--json-mode", options.jsonMode ?? "verbose");
    args.push("--no-progress");
  }
  if (options.dryRun) {
    args.push("--dry-run");
  }
  if (options.parallel) {
    args.push("--parallel");
  }
  if (cookieJarMode === "per-test") {
    // Forces the CLI override regardless of the file's declared
    // `cookies:` mode (except `off`, which Tarn short-circuits on the
    // runner side so there is no jar to reset).
    args.push("--cookie-jar-per-test");
  }
  if (options.environment) {
    args.push("--env", options.environment);
  }
  if (options.tags && options.tags.length > 0) {
    args.push("--tag", options.tags.join(","));
  }
  if (options.selectors) {
    for (const selector of options.selectors) {
      args.push("--select", selector);
    }
  }
  if (options.vars) {
    for (const [key, value] of Object.entries(options.vars)) {
      args.push("--var", `${key}=${value}`);
    }
  }
  for (const file of options.files) {
    args.push(file);
  }
  return args;
}
