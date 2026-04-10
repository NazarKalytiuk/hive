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

export default {
  Position,
  Range,
  Location,
  MarkdownString,
  TestMessage,
  Uri,
  workspace,
  window,
};
