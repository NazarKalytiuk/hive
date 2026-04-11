import * as vscode from "vscode";
import {
  parseYamlFile,
  FileRanges,
  StepRange,
  TestRange,
} from "./YamlAst";
import { readConfig, buildExcludeGlob } from "../config";
import type { ListFileOutcome, TarnBackend } from "../backend/TarnBackend";
import type { ScopedListFileStrict } from "../util/schemaGuards";
import { getOutputChannel } from "../outputChannel";

export interface ParsedFile {
  uri: vscode.Uri;
  ranges: FileRanges;
  /**
   * `true` if the `ranges` were derived from `tarn list --file`
   * (NAZ-261 / NAZ-282), with AST ranges overlaid for positioning;
   * `false` if they came solely from the client-side YAML AST path.
   * Exposed for observability and the scoped-discovery integration
   * test — production code should treat both paths as equivalent.
   */
  fromScopedList?: boolean;
}

type Listener = (uri: vscode.Uri, parsed: ParsedFile | undefined) => void;

export interface WorkspaceIndexOptions {
  /**
   * Backend used for incremental scoped discovery via Tarn T57
   * (`tarn list --file PATH --format json`). When omitted, the
   * index always falls back to the client-side YAML AST path —
   * this is the mode the unit tests use to avoid spawning Tarn.
   */
  backend?: TarnBackend;
  /**
   * CWD passed to the backend when invoking `tarn list --file`.
   * Usually the first workspace folder; may be omitted when no
   * folder is open, in which case the backend is skipped.
   */
  cwd?: string;
}

export class WorkspaceIndex implements vscode.Disposable {
  private readonly files = new Map<string, ParsedFile>();
  private readonly watchers: vscode.FileSystemWatcher[] = [];
  private readonly listeners = new Set<Listener>();
  private initialized = false;
  private readonly backend: TarnBackend | undefined;
  private readonly cwd: string | undefined;
  private scopedDiscoverySupported = true;

  constructor(options: WorkspaceIndexOptions = {}) {
    this.backend = options.backend;
    this.cwd = options.cwd;
  }

  async initialize(): Promise<void> {
    const config = readConfig();
    const excludes = buildExcludeGlob(config.excludeGlobs);
    const uris = await vscode.workspace.findFiles(config.testFileGlob, excludes ?? null);

    this.files.clear();
    // Re-enable scoped discovery on every explicit refresh: if the
    // user just installed a newer Tarn or pointed `tarn.binaryPath`
    // at a different binary, we want the next edit to try the
    // scoped path again instead of staying stuck on AST fallback.
    this.scopedDiscoverySupported = true;
    // Startup discovery deliberately stays on the client-side AST:
    // spawning Tarn once per file at activation would dominate the
    // extension's startup budget for workspaces with dozens of test
    // files. The scoped `tarn list --file` path only kicks in on
    // individual `onDidChange` / `onDidCreate` events where the
    // process cost is amortized across a long-lived editor session.
    await Promise.all(uris.map((uri) => this.reparseFromAst(uri)));

    if (!this.initialized) {
      this.startWatcher(config.testFileGlob);
      this.initialized = true;
    }
  }

  get all(): readonly ParsedFile[] {
    return Array.from(this.files.values());
  }

  get(uri: vscode.Uri): ParsedFile | undefined {
    return this.files.get(uri.toString());
  }

  onDidChange(listener: Listener): vscode.Disposable {
    this.listeners.add(listener);
    return new vscode.Disposable(() => this.listeners.delete(listener));
  }

  dispose(): void {
    for (const watcher of this.watchers) {
      watcher.dispose();
    }
    this.listeners.clear();
    this.files.clear();
  }

  /**
   * Incremental reparse triggered by the file-system watcher. Tries
   * the scoped `tarn list --file` path first (authoritative names,
   * post-`include:` expansion, matches what the runner will see) and
   * falls back to the client AST if the backend returns `undefined`
   * (older Tarn, process error, parse error). The new `ParsedFile`
   * is compared against the cached entry and listeners are only
   * notified when the structure (file name, tests, steps) actually
   * changed — edits that touch request bodies, URLs, assertions, or
   * comments do not churn the Test Explorer tree.
   */
  async refreshSingleFile(uri: vscode.Uri): Promise<void> {
    let parsed: ParsedFile | undefined;

    const scoped = await this.tryScopedList(uri);
    if (scoped) {
      const astRanges = await this.readAstRanges(uri);
      parsed = {
        uri,
        ranges: mergeScopedWithAst(scoped, astRanges),
        fromScopedList: true,
      };
    }
    if (!parsed) {
      parsed = await this.buildFromAst(uri);
    }

    const key = uri.toString();
    const previous = this.files.get(key);
    if (!parsed) {
      if (previous !== undefined) {
        this.files.delete(key);
        this.notify(uri, undefined);
      }
      return;
    }

    this.files.set(key, parsed);
    if (previous && rangesStructurallyEqual(previous.ranges, parsed.ranges)) {
      // Structure is unchanged (only request body / URL / assertion
      // content moved), so leave the TestItem tree alone. Without
      // this early-return the watcher would call `replace(children)`
      // on every keystroke-save, flashing the gutter icons.
      return;
    }
    this.notify(uri, parsed);
  }

  private async tryScopedList(
    uri: vscode.Uri,
  ): Promise<ScopedListFileStrict | undefined> {
    if (!this.backend || !this.cwd) {
      return undefined;
    }
    if (!this.scopedDiscoverySupported) {
      return undefined;
    }
    const cts = new vscode.CancellationTokenSource();
    try {
      const outcome: ListFileOutcome = await this.backend.listFile(
        uri.fsPath,
        this.cwd,
        cts.token,
      );
      if (outcome.ok) {
        return outcome.file;
      }
      if (outcome.reason === "file_error") {
        // The binary still supports `--file`; Tarn just could not
        // parse this one YAML. Keep scoped discovery enabled for
        // future edits and fall back to the client AST path for
        // this specific call so the broken file still shows up in
        // the Test Explorer tree (the diagnostics provider will
        // surface the actual parse error separately).
        // l10n-ignore: debug log for engineers, shown with [tarn] prefix.
        getOutputChannel().appendLine(
          `[tarn] scoped list rejected ${uri.fsPath}, using AST: ${outcome.error}`,
        );
        return undefined;
      }
      // `unsupported` — spawn error, watchdog, missing binary, or
      // older Tarn without `--file`. Flip the capability flag off
      // so we do not spawn Tarn on every subsequent edit. A manual
      // `Tarn: Refresh Discovery` re-runs `initialize()`, which
      // resets the flag via `resetScopedDiscoverySupport`.
      this.scopedDiscoverySupported = false;
      // l10n-ignore: debug log for engineers, shown with [tarn] prefix.
      getOutputChannel().appendLine(
        "[tarn] scoped discovery disabled (tarn list --file unsupported); falling back to AST",
      );
      return undefined;
    } catch (err) {
      this.scopedDiscoverySupported = false;
      // l10n-ignore: debug log for engineers, shown with [tarn] prefix.
      getOutputChannel().appendLine(
        `[tarn] scoped discovery errored, falling back to AST: ${String(err)}`,
      );
      return undefined;
    } finally {
      cts.dispose();
    }
  }

  private async buildFromAst(uri: vscode.Uri): Promise<ParsedFile | undefined> {
    const ranges = await this.readAstRanges(uri);
    if (!ranges) {
      return undefined;
    }
    return { uri, ranges, fromScopedList: false };
  }

  private async readAstRanges(uri: vscode.Uri): Promise<FileRanges | undefined> {
    try {
      const bytes = await vscode.workspace.fs.readFile(uri);
      const text = Buffer.from(bytes).toString("utf8");
      return parseYamlFile(text);
    } catch {
      return undefined;
    }
  }

  private async reparseFromAst(uri: vscode.Uri): Promise<void> {
    const parsed = await this.buildFromAst(uri);
    const key = uri.toString();
    if (!parsed) {
      this.files.delete(key);
      this.notify(uri, undefined);
      return;
    }
    this.files.set(key, parsed);
    this.notify(uri, parsed);
  }

  private startWatcher(glob: string): void {
    const watcher = vscode.workspace.createFileSystemWatcher(glob);
    watcher.onDidCreate((uri) => {
      void this.refreshSingleFile(uri);
    });
    watcher.onDidChange((uri) => {
      void this.refreshSingleFile(uri);
    });
    watcher.onDidDelete((uri) => {
      this.files.delete(uri.toString());
      this.notify(uri, undefined);
    });
    this.watchers.push(watcher);
  }

  private notify(uri: vscode.Uri, parsed: ParsedFile | undefined): void {
    for (const listener of this.listeners) {
      listener(uri, parsed);
    }
  }
}

/**
 * Merge Tarn's scoped-list output (authoritative structure, correct
 * post-`include:` expansion) with the client AST's range metadata
 * (authoritative file positions). Tarn wins on "what tests/steps
 * exist"; the AST wins on "where on disk the name lives". When Tarn
 * reports a step/test that the raw YAML does not expose (e.g., an
 * `include:`-expanded step), we fall back to a zero-width range so
 * downstream consumers still have a valid anchor.
 */
export function mergeScopedWithAst(
  scoped: ScopedListFileStrict,
  ast: FileRanges | undefined,
): FileRanges {
  const astTestsByName = new Map<string, TestRange>();
  const astSetupByName = new Map<string, StepRange>();
  const astTeardownByName = new Map<string, StepRange>();
  if (ast) {
    for (const t of ast.tests) {
      astTestsByName.set(t.name, t);
    }
    for (const s of ast.setup) {
      astSetupByName.set(s.name, s);
    }
    for (const s of ast.teardown) {
      astTeardownByName.set(s.name, s);
    }
  }

  const zero = new vscode.Range(new vscode.Position(0, 0), new vscode.Position(0, 0));
  const mergeSteps = (
    steps: ReadonlyArray<{ readonly name: string }>,
    astStepsByName?: Map<string, StepRange>,
  ): StepRange[] =>
    steps.map((step, index) => {
      const hit = astStepsByName?.get(step.name);
      return {
        index,
        name: step.name,
        nameRange: hit?.nameRange ?? zero,
      };
    });

  const mergedSetup = mergeSteps(scoped.setup, astSetupByName);
  const mergedTeardown = mergeSteps(scoped.teardown, astTeardownByName);

  // Tarn folds a top-level `steps:` block into a synthetic `default`
  // test in its unscoped JSON output, but the scoped shape surfaces
  // it on `files[0].steps` alongside `files[0].tests[]`. Mirror the
  // AST behavior so downstream consumers see a uniform `tests[]`
  // list regardless of which YAML variant the user wrote.
  const mergedTests: TestRange[] = [];
  if (scoped.steps.length > 0) {
    const fallbackAstDefault = astTestsByName.get("default");
    const stepAstByName = new Map<string, StepRange>();
    if (fallbackAstDefault) {
      for (const s of fallbackAstDefault.steps) {
        stepAstByName.set(s.name, s);
      }
    }
    mergedTests.push({
      name: "default",
      description: null,
      nameRange: fallbackAstDefault?.nameRange ?? zero,
      steps: mergeSteps(scoped.steps, stepAstByName),
    });
  }
  for (const test of scoped.tests) {
    const astTest = astTestsByName.get(test.name);
    const stepAstByName = new Map<string, StepRange>();
    if (astTest) {
      for (const s of astTest.steps) {
        stepAstByName.set(s.name, s);
      }
    }
    mergedTests.push({
      name: test.name,
      description: test.description ?? null,
      nameRange: astTest?.nameRange ?? zero,
      steps: mergeSteps(test.steps, stepAstByName),
    });
  }

  return {
    fileName: scoped.name || ast?.fileName || "(unnamed)",
    fileNameRange: ast?.fileNameRange,
    tests: mergedTests,
    setup: mergedSetup,
    teardown: mergedTeardown,
    parseError: ast?.parseError,
  };
}

/**
 * Structural equality for `FileRanges`. Used to short-circuit the
 * watcher's listener notification when an edit does not alter the
 * test/step tree — the tree structure is compared by names and
 * arity only; range positions are deliberately ignored so a trivial
 * shift in line numbers does not count as a structural change.
 */
export function rangesStructurallyEqual(a: FileRanges, b: FileRanges): boolean {
  if (a.fileName !== b.fileName) {
    return false;
  }
  if (a.tests.length !== b.tests.length) {
    return false;
  }
  if (a.setup.length !== b.setup.length) {
    return false;
  }
  if (a.teardown.length !== b.teardown.length) {
    return false;
  }
  for (let i = 0; i < a.tests.length; i++) {
    const ta = a.tests[i];
    const tb = b.tests[i];
    if (ta.name !== tb.name) return false;
    if ((ta.description ?? null) !== (tb.description ?? null)) return false;
    if (ta.steps.length !== tb.steps.length) return false;
    for (let j = 0; j < ta.steps.length; j++) {
      if (ta.steps[j].name !== tb.steps[j].name) return false;
    }
  }
  for (let i = 0; i < a.setup.length; i++) {
    if (a.setup[i].name !== b.setup[i].name) return false;
  }
  for (let i = 0; i < a.teardown.length; i++) {
    if (a.teardown[i].name !== b.teardown[i].name) return false;
  }
  return true;
}
