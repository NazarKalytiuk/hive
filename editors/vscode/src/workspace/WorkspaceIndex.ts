import * as vscode from "vscode";
import { parseYamlFile, FileRanges } from "./YamlAst";
import { readConfig, buildExcludeGlob } from "../config";

export interface ParsedFile {
  uri: vscode.Uri;
  ranges: FileRanges;
}

type Listener = (uri: vscode.Uri, parsed: ParsedFile | undefined) => void;

export class WorkspaceIndex implements vscode.Disposable {
  private readonly files = new Map<string, ParsedFile>();
  private readonly watchers: vscode.FileSystemWatcher[] = [];
  private readonly listeners = new Set<Listener>();
  private initialized = false;

  async initialize(): Promise<void> {
    const config = readConfig();
    const excludes = buildExcludeGlob(config.excludeGlobs);
    const uris = await vscode.workspace.findFiles(config.testFileGlob, excludes ?? null);

    this.files.clear();
    await Promise.all(uris.map((uri) => this.reparse(uri)));

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

  private async reparse(uri: vscode.Uri): Promise<void> {
    try {
      const bytes = await vscode.workspace.fs.readFile(uri);
      const text = Buffer.from(bytes).toString("utf8");
      const ranges = parseYamlFile(text);
      const parsed: ParsedFile = { uri, ranges };
      this.files.set(uri.toString(), parsed);
      this.notify(uri, parsed);
    } catch {
      this.files.delete(uri.toString());
      this.notify(uri, undefined);
    }
  }

  private startWatcher(glob: string): void {
    const watcher = vscode.workspace.createFileSystemWatcher(glob);
    watcher.onDidCreate((uri) => this.reparse(uri));
    watcher.onDidChange((uri) => this.reparse(uri));
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
