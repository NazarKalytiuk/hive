import * as vscode from "vscode";
import type { EnvEntry } from "../util/schemaGuards";
import type { EnvironmentsView } from "../views/EnvironmentsView";
import { BUILTIN_FUNCTIONS } from "./CompletionProvider";
import { collectVisibleCaptures, type VisibleCapture } from "./completion/captures";
import { findHoverToken } from "./completion/hoverToken";

/**
 * Hover provider for Tarn interpolation expressions.
 *
 * Renders a markdown tooltip when the cursor sits inside a
 * `{{ env.x }}`, `{{ capture.y }}`, or `{{ $builtin }}` expression.
 * Reuses the same env cache (`EnvironmentsView`) and capture walker
 * (`collectVisibleCaptures`) as `TarnCompletionProvider` so the two
 * providers stay in sync.
 *
 * Dry-run URL previews (resolving `url:` strings end-to-end via
 * `tarn run --dry-run`) are intentionally out of scope for v1. They
 * require spawning tarn per hover with a cache keyed by file + test +
 * step + env, and the extension's Phase 3 hover story is valuable
 * without that extra complexity. See NAZ-266 notes for the follow-up.
 */
export class TarnHoverProvider implements vscode.HoverProvider {
  constructor(private readonly environmentsView: EnvironmentsView) {}

  async provideHover(
    document: vscode.TextDocument,
    position: vscode.Position,
  ): Promise<vscode.Hover | undefined> {
    const lineText = document.lineAt(position.line).text;
    const token = findHoverToken(lineText, position.character);
    if (token.kind === "none") {
      return undefined;
    }

    const range = new vscode.Range(
      new vscode.Position(position.line, token.rangeStart),
      new vscode.Position(position.line, token.rangeEnd),
    );

    if (token.kind === "empty") {
      return new vscode.Hover(this.renderEmptyHelp(), range);
    }
    if (token.kind === "env") {
      const entries = await this.environmentsView.getEntries();
      return new vscode.Hover(this.renderEnvHover(token.identifier, entries), range);
    }
    if (token.kind === "capture") {
      const captures = collectVisibleCaptures(
        document.getText(),
        document.offsetAt(position),
      );
      return new vscode.Hover(this.renderCaptureHover(token.identifier, captures), range);
    }
    if (token.kind === "builtin") {
      return new vscode.Hover(this.renderBuiltinHover(token.identifier), range);
    }
    return undefined;
  }

  private renderEmptyHelp(): vscode.MarkdownString {
    const md = new vscode.MarkdownString();
    md.appendMarkdown("**Tarn interpolation**\n\n");
    md.appendMarkdown("- `{{ env.KEY }}` — value from the env resolution chain\n");
    md.appendMarkdown(
      "- `{{ capture.NAME }}` — value captured by a prior step in this test\n",
    );
    md.appendMarkdown("- `{{ $uuid }}` — built-in function (also `$timestamp`, `$now_iso`, `$random_hex(n)`, `$random_int(min, max)`)\n");
    return md;
  }

  private renderEnvHover(
    key: string,
    entries: readonly EnvEntry[],
  ): vscode.MarkdownString {
    const md = new vscode.MarkdownString();
    md.isTrusted = false;
    md.supportHtml = false;

    if (key === "") {
      md.appendMarkdown("**`{{ env.KEY }}`**\n\n");
      md.appendMarkdown("Resolves `KEY` from the env resolution chain:\n\n");
      md.appendMarkdown("1. `--var KEY=VALUE` on the CLI\n");
      md.appendMarkdown("2. `tarn.env.local.yaml`\n");
      md.appendMarkdown("3. `tarn.env.{active}.yaml`\n");
      md.appendMarkdown("4. shell environment\n");
      md.appendMarkdown("5. `tarn.env.yaml`\n");
      md.appendMarkdown("6. inline `env:` block in this test file\n");
      return md;
    }

    const declaringEntries = entries.filter((e) =>
      Object.prototype.hasOwnProperty.call(e.vars, key),
    );

    md.appendMarkdown(`**\`env.${key}\`**\n\n`);

    if (declaringEntries.length === 0) {
      md.appendMarkdown(
        `Not declared in any configured environment. Will resolve at runtime from \`tarn.env.yaml\`, a named env file, the shell, or an inline \`env:\` block — or fail with \`unresolved_template\` if none of those provide it.\n`,
      );
      return md;
    }

    md.appendMarkdown("Declared in:\n\n");
    for (const entry of declaringEntries) {
      const value = entry.vars[key];
      md.appendMarkdown(
        `- \`${entry.name}\` (\`${entry.source_file}\`): \`${value}\`\n`,
      );
    }
    if (declaringEntries.length > 1) {
      md.appendMarkdown(
        "\n_The effective value depends on which environment is active (`--env NAME`) plus the resolution chain above._",
      );
    }
    return md;
  }

  private renderCaptureHover(
    name: string,
    captures: VisibleCapture[],
  ): vscode.MarkdownString {
    const md = new vscode.MarkdownString();
    md.isTrusted = false;
    md.supportHtml = false;

    if (name === "") {
      md.appendMarkdown("**`{{ capture.NAME }}`**\n\n");
      md.appendMarkdown(
        "Resolves `NAME` from the captures accumulated by earlier steps in the same test (plus any setup captures).",
      );
      return md;
    }

    const matches = captures.filter((c) => c.name === name);
    md.appendMarkdown(`**\`capture.${name}\`**\n\n`);

    if (matches.length === 0) {
      md.appendMarkdown(
        `Not captured by any step visible from this position. Captures are only in scope within the same test (setup counts for every test). Check that an earlier step declares \`capture: { ${name}: ... }\`.`,
      );
      return md;
    }

    md.appendMarkdown("Captured by:\n\n");
    for (const cap of matches) {
      const scope =
        cap.phase === "setup"
          ? "setup"
          : cap.testName
            ? `test \`${cap.testName}\``
            : "this file";
      md.appendMarkdown(
        `- step \`${cap.stepName}\` (index ${cap.stepIndex}, ${scope})\n`,
      );
    }
    if (matches.length > 1) {
      md.appendMarkdown(
        "\n_Later declarations override earlier ones when the runner merges captures._",
      );
    }
    return md;
  }

  private renderBuiltinHover(name: string): vscode.MarkdownString {
    const md = new vscode.MarkdownString();
    md.isTrusted = false;
    md.supportHtml = false;

    if (name === "") {
      md.appendMarkdown("**`{{ $builtin }}`**\n\n");
      md.appendMarkdown("Tarn built-in functions:\n\n");
      for (const fn of BUILTIN_FUNCTIONS) {
        md.appendMarkdown(`- \`${fn.signature}\` — ${fn.doc}\n`);
      }
      return md;
    }

    const lookupName = `$${name}`;
    const fn = BUILTIN_FUNCTIONS.find((b) => b.name === lookupName);
    if (!fn) {
      md.appendMarkdown(`**\`${lookupName}\`**\n\n`);
      md.appendMarkdown(
        `Not a recognized Tarn built-in. Known functions: ${BUILTIN_FUNCTIONS.map((b) => `\`${b.name}\``).join(", ")}.`,
      );
      return md;
    }

    md.appendMarkdown(`**\`${fn.signature}\`**\n\n`);
    md.appendMarkdown(fn.doc);
    return md;
  }
}
