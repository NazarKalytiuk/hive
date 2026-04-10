import * as vscode from "vscode";
import {
  Document,
  LineCounter,
  parseDocument,
  YAMLMap,
  YAMLSeq,
  isMap,
  isSeq,
  isScalar,
  Scalar,
  Pair,
} from "yaml";

export interface StepRange {
  index: number;
  name: string;
  nameRange: vscode.Range;
}

export interface TestRange {
  name: string;
  description: string | null;
  nameRange: vscode.Range;
  steps: StepRange[];
}

export interface FileRanges {
  fileName: string;
  fileNameRange: vscode.Range | undefined;
  tests: TestRange[];
  setup: StepRange[];
  teardown: StepRange[];
  parseError?: string;
}

export function parseYamlFile(source: string): FileRanges {
  const lineCounter = new LineCounter();
  let doc: Document.Parsed;
  try {
    doc = parseDocument(source, { lineCounter, keepSourceTokens: false });
  } catch (err) {
    return emptyRanges(String(err));
  }

  if (doc.errors.length > 0) {
    const msg = doc.errors.map((e) => e.message).join("; ");
    return { ...emptyRanges(msg), fileName: fallbackFileName() };
  }

  const root = doc.contents;
  if (!isMap(root)) {
    return emptyRanges("root is not a map");
  }

  const fileName = getScalarString(root, "name") ?? fallbackFileName();
  const fileNameRange = rangeOfKey(root, "name", lineCounter);

  return {
    fileName,
    fileNameRange,
    tests: collectTests(root, lineCounter),
    setup: collectStepSequence(root, "setup", lineCounter),
    teardown: collectStepSequence(root, "teardown", lineCounter),
  };
}

function emptyRanges(parseError?: string): FileRanges {
  return {
    fileName: fallbackFileName(),
    fileNameRange: undefined,
    tests: [],
    setup: [],
    teardown: [],
    parseError,
  };
}

function fallbackFileName(): string {
  return "(unnamed)";
}

function collectTests(root: YAMLMap, lineCounter: LineCounter): TestRange[] {
  const flatSteps = collectStepSequence(root, "steps", lineCounter);
  if (flatSteps.length > 0) {
    return [
      {
        name: "default",
        description: null,
        nameRange: rangeOfKey(root, "steps", lineCounter) ?? defaultRange(),
        steps: flatSteps,
      },
    ];
  }

  const testsNode = getMapValue(root, "tests");
  if (!testsNode || !isMap(testsNode)) {
    return [];
  }

  const results: TestRange[] = [];
  for (const pair of testsNode.items) {
    if (!isScalar(pair.key)) {
      continue;
    }
    const name = String(pair.key.value);
    const nameRange = rangeOfPairKey(pair, lineCounter) ?? defaultRange();
    const value = pair.value;
    if (!isMap(value)) {
      results.push({ name, description: null, nameRange, steps: [] });
      continue;
    }
    const description = getScalarString(value, "description") ?? null;
    const steps = collectStepSequence(value, "steps", lineCounter);
    results.push({ name, description, nameRange, steps });
  }
  return results;
}

function collectStepSequence(
  parent: YAMLMap,
  key: string,
  lineCounter: LineCounter,
): StepRange[] {
  const value = getMapValue(parent, key);
  if (!value || !isSeq(value)) {
    return [];
  }
  const steps: StepRange[] = [];
  let index = 0;
  for (const item of value.items) {
    if (!isMap(item)) {
      index++;
      continue;
    }
    const name = getScalarString(item, "name") ?? `step ${index + 1}`;
    const nameRange = rangeOfKey(item, "name", lineCounter) ?? defaultRange();
    steps.push({ index, name, nameRange });
    index++;
  }
  return steps;
}

function getMapValue(map: YAMLMap, key: string): unknown {
  for (const pair of map.items) {
    if (isScalar(pair.key) && pair.key.value === key) {
      return pair.value;
    }
  }
  return undefined;
}

function getScalarString(map: YAMLMap, key: string): string | undefined {
  const value = getMapValue(map, key);
  if (isScalar(value)) {
    return typeof value.value === "string" ? value.value : String(value.value);
  }
  return undefined;
}

function rangeOfKey(
  map: YAMLMap,
  key: string,
  lineCounter: LineCounter,
): vscode.Range | undefined {
  for (const pair of map.items) {
    if (isScalar(pair.key) && pair.key.value === key) {
      return rangeOfPairKey(pair, lineCounter);
    }
  }
  return undefined;
}

function rangeOfPairKey(
  pair: Pair<unknown, unknown>,
  lineCounter: LineCounter,
): vscode.Range | undefined {
  const keyNode = pair.key;
  if (!isScalar(keyNode)) {
    return undefined;
  }
  const range = getNodeRange(keyNode);
  if (!range) {
    return undefined;
  }
  return toVsRange(range, lineCounter);
}

function getNodeRange(node: Scalar | YAMLMap | YAMLSeq): [number, number, number] | undefined {
  const r = (node as unknown as { range?: [number, number, number] }).range;
  if (!r) {
    return undefined;
  }
  return r;
}

function toVsRange(
  range: [number, number, number],
  lineCounter: LineCounter,
): vscode.Range {
  const [start, valueEnd] = range;
  const startPos = lineCounter.linePos(start);
  const endPos = lineCounter.linePos(valueEnd);
  return new vscode.Range(
    new vscode.Position(startPos.line - 1, startPos.col - 1),
    new vscode.Position(endPos.line - 1, Math.max(endPos.col - 1, 0)),
  );
}

function defaultRange(): vscode.Range {
  return new vscode.Range(new vscode.Position(0, 0), new vscode.Position(0, 0));
}
