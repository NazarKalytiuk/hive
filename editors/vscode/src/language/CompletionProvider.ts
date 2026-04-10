import * as vscode from "vscode";
import type { EnvironmentsView } from "../views/EnvironmentsView";
import type { EnvEntry } from "../util/schemaGuards";
import { collectVisibleCaptures, type VisibleCapture } from "./completion/captures";

/**
 * Shape of the interpolation context at the cursor. The completion
 * provider uses this to pick which set of items to return.
 */
export type InterpolationContext =
  | { kind: "none" }
  | { kind: "empty"; prefix: "" }
  | { kind: "env"; prefix: string }
  | { kind: "capture"; prefix: string }
  | { kind: "builtin"; prefix: string };

export const BUILTIN_FUNCTIONS: ReadonlyArray<{
  name: string;
  insertText: string;
  signature: string;
  doc: string;
}> = [
  {
    name: "$uuid",
    insertText: "uuid",
    signature: "$uuid",
    doc: "Random UUID v4.",
  },
  {
    name: "$timestamp",
    insertText: "timestamp",
    signature: "$timestamp",
    doc: "Current Unix timestamp (seconds).",
  },
  {
    name: "$now_iso",
    insertText: "now_iso",
    signature: "$now_iso",
    doc: "Current timestamp in ISO 8601 (e.g. `2026-04-10T12:34:56Z`).",
  },
  {
    name: "$random_hex",
    insertText: "random_hex(${1:8})",
    signature: "$random_hex(n)",
    doc: "Random hex string of length `n`.",
  },
  {
    name: "$random_int",
    insertText: "random_int(${1:min}, ${2:max})",
    signature: "$random_int(min, max)",
    doc: "Random integer in `[min, max]` inclusive.",
  },
];

/**
 * Parse the text on the current line up to the cursor and decide what
 * sort of interpolation the user is inside, if any. Returns `none` when
 * the cursor is outside any `{{ ... }}` pair.
 *
 * Exported so unit tests can exercise the logic without spinning up the
 * whole extension host.
 */
export function detectInterpolationContext(
  linePrefix: string,
): InterpolationContext {
  // Find the last `{{` before the cursor that isn't already closed.
  const openIdx = linePrefix.lastIndexOf("{{");
  if (openIdx < 0) {
    return { kind: "none" };
  }
  const closeAfter = linePrefix.indexOf("}}", openIdx);
  if (closeAfter >= 0 && closeAfter < linePrefix.length) {
    return { kind: "none" };
  }
  // Everything between `{{` and the cursor is the expression-in-progress.
  const expr = linePrefix.slice(openIdx + 2).trimStart();

  if (expr === "") {
    return { kind: "empty", prefix: "" };
  }
  if (expr.startsWith("env.")) {
    return { kind: "env", prefix: expr.slice(4) };
  }
  if (expr === "env") {
    return { kind: "env", prefix: "" };
  }
  if (expr.startsWith("capture.")) {
    return { kind: "capture", prefix: expr.slice(8) };
  }
  if (expr === "capture") {
    return { kind: "capture", prefix: "" };
  }
  if (expr.startsWith("$")) {
    return { kind: "builtin", prefix: expr.slice(1) };
  }
  return { kind: "none" };
}

/**
 * Merge inline `vars` from every discovered environment into a single
 * `{ key: [source, source, ...] }` map so the completion UI can show
 * which envs declare each key.
 */
export function mergeEnvKeys(entries: readonly EnvEntry[]): Map<string, string[]> {
  const merged = new Map<string, string[]>();
  for (const entry of entries) {
    for (const key of Object.keys(entry.vars)) {
      const sources = merged.get(key) ?? [];
      sources.push(entry.name);
      merged.set(key, sources);
    }
  }
  return merged;
}

export class TarnCompletionProvider implements vscode.CompletionItemProvider {
  constructor(private readonly environmentsView: EnvironmentsView) {}

  async provideCompletionItems(
    document: vscode.TextDocument,
    position: vscode.Position,
  ): Promise<vscode.CompletionItem[] | vscode.CompletionList | undefined> {
    const linePrefix = document
      .lineAt(position.line)
      .text.slice(0, position.character);
    const context = detectInterpolationContext(linePrefix);
    if (context.kind === "none") {
      return undefined;
    }

    if (context.kind === "empty") {
      return this.topLevelCompletions();
    }
    if (context.kind === "env") {
      const entries = await this.environmentsView.getEntries();
      return this.envCompletions(entries);
    }
    if (context.kind === "capture") {
      const source = document.getText();
      const offset = document.offsetAt(position);
      const captures = collectVisibleCaptures(source, offset);
      return this.captureCompletions(captures);
    }
    if (context.kind === "builtin") {
      return this.builtinCompletions();
    }
    return undefined;
  }

  private topLevelCompletions(): vscode.CompletionItem[] {
    return [
      completionItem({
        label: "env",
        insertText: "env.",
        kind: vscode.CompletionItemKind.Module,
        detail: "Environment variable",
        documentation: "Expands to a key from the merged env resolution chain.",
        triggerSuggest: true,
      }),
      completionItem({
        label: "capture",
        insertText: "capture.",
        kind: vscode.CompletionItemKind.Module,
        detail: "Captured variable",
        documentation: "Expands to a value captured by a prior step in the same test.",
        triggerSuggest: true,
      }),
      completionItem({
        label: "$uuid",
        insertText: "$uuid",
        kind: vscode.CompletionItemKind.Function,
        detail: "Built-in function",
        documentation: "Random UUID v4.",
      }),
    ];
  }

  private envCompletions(entries: readonly EnvEntry[]): vscode.CompletionItem[] {
    const merged = mergeEnvKeys(entries);
    const items: vscode.CompletionItem[] = [];
    for (const [key, sources] of merged) {
      items.push(
        completionItem({
          label: key,
          insertText: key,
          kind: vscode.CompletionItemKind.Variable,
          detail: `env.${key}`,
          documentation: `Declared in: ${sources.join(", ")}`,
        }),
      );
    }
    items.sort((a, b) => (a.label as string).localeCompare(b.label as string));
    return items;
  }

  private captureCompletions(captures: VisibleCapture[]): vscode.CompletionItem[] {
    const seen = new Map<string, VisibleCapture>();
    for (const cap of captures) {
      // Later declarations override earlier ones (same as Tarn's
      // runtime merge behavior).
      seen.set(cap.name, cap);
    }
    const items: vscode.CompletionItem[] = [];
    for (const [name, cap] of seen) {
      const scope =
        cap.phase === "setup"
          ? "setup"
          : cap.testName
            ? `test '${cap.testName}'`
            : "this file";
      items.push(
        completionItem({
          label: name,
          insertText: name,
          kind: vscode.CompletionItemKind.Variable,
          detail: `capture from ${scope}`,
          documentation: `Set by step '${cap.stepName}' (index ${cap.stepIndex}).`,
        }),
      );
    }
    items.sort((a, b) => (a.label as string).localeCompare(b.label as string));
    return items;
  }

  private builtinCompletions(): vscode.CompletionItem[] {
    return BUILTIN_FUNCTIONS.map((fn) => {
      const item = new vscode.CompletionItem(
        fn.name,
        vscode.CompletionItemKind.Function,
      );
      item.detail = fn.signature;
      item.documentation = new vscode.MarkdownString(fn.doc);
      const snippet = new vscode.SnippetString(fn.insertText);
      item.insertText = snippet;
      return item;
    });
  }
}

function completionItem(opts: {
  label: string;
  insertText: string;
  kind: vscode.CompletionItemKind;
  detail: string;
  documentation?: string;
  triggerSuggest?: boolean;
}): vscode.CompletionItem {
  const item = new vscode.CompletionItem(opts.label, opts.kind);
  item.insertText = opts.insertText;
  item.detail = opts.detail;
  if (opts.documentation) {
    item.documentation = new vscode.MarkdownString(opts.documentation);
  }
  if (opts.triggerSuggest) {
    item.command = {
      command: "editor.action.triggerSuggest",
      title: "Re-trigger completions",
    };
  }
  return item;
}
