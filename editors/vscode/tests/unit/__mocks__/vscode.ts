export class Position {
  constructor(public readonly line: number, public readonly character: number) {}
}

export class Range {
  constructor(public readonly start: Position, public readonly end: Position) {}
}

export class Location {
  constructor(public readonly uri: unknown, public readonly range: Range) {}
}

export class MarkdownString {
  supportHtml = false;
  isTrusted: boolean | undefined = undefined;
  constructor(public readonly value: string = "") {}
}

export class TestMessage {
  expectedOutput: string | undefined;
  actualOutput: string | undefined;
  location: Location | undefined;
  constructor(public readonly message: string | MarkdownString) {}
}

export const Uri = {
  file(p: string) {
    return { fsPath: p, toString: () => `file://${p}`, path: p };
  },
  parse(s: string) {
    return { fsPath: s, toString: () => s, path: s };
  },
};

export const workspace = {
  getConfiguration(_section?: string): {
    get<T>(key: string, defaultValue?: T): T | undefined;
  } {
    return {
      get<T>(_key: string, defaultValue?: T): T | undefined {
        return defaultValue;
      },
    };
  },
};

export const window = {
  async showWarningMessage(
    _message: string,
    ..._items: string[]
  ): Promise<string | undefined> {
    return undefined;
  },
};

/**
 * Minimal `vscode.l10n` stub for unit tests.
 *
 * VS Code's real implementation looks up `message` in a bundle keyed
 * by locale and falls back to the English key when no translation is
 * available. Unit tests always see the EN fallback, so we faithfully
 * reproduce the fallback: return `message` with `{N}` placeholders
 * substituted by positional args.
 */
export const l10n = {
  t(message: string, ...args: Array<string | number | boolean>): string {
    if (args.length === 0) {
      return message;
    }
    return message.replace(/\{(\d+)\}/g, (match, indexStr) => {
      const index = Number(indexStr);
      if (Number.isInteger(index) && index >= 0 && index < args.length) {
        return String(args[index]);
      }
      return match;
    });
  },
  bundle: undefined as Record<string, string> | undefined,
  uri: undefined,
};

export default {
  Position,
  Range,
  Location,
  MarkdownString,
  TestMessage,
  Uri,
  workspace,
  window,
  l10n,
};
