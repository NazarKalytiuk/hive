import { LineCounter, parseDocument, isMap, isScalar, isSeq } from "yaml";
import type {
  Document as YAMLDocument,
  YAMLMap as YAMLMapType,
  YAMLSeq as YAMLSeqType,
} from "yaml";

/**
 * One capture variable declared by a prior step. `stepIndex` is 0-based
 * within the phase (setup / test). `phase` lets callers prefer setup
 * captures over same-test captures when naming collides, and adds
 * useful context to the completion UI.
 */
export interface VisibleCapture {
  name: string;
  stepIndex: number;
  phase: "setup" | "test";
  testName?: string;
  stepName: string;
}

/**
 * Compute which captures are visible at a given byte offset in a Tarn
 * test file. Rules (matching the Rust runner):
 *
 *   * Setup captures are visible from every step in every test.
 *   * Within a test, captures from strictly earlier steps are visible.
 *   * Captures from the same step, later steps, or other tests are not.
 *   * Teardown is not considered because templates in teardown can see
 *     both setup and test captures, so the full union is returned.
 *
 * Parses the YAML via the shared `yaml` library and walks the CST so
 * we do not need to keep a separate AST in sync with the workspace
 * index.
 */
export function collectVisibleCaptures(
  source: string,
  offset: number,
): VisibleCapture[] {
  const lineCounter = new LineCounter();
  let doc: YAMLDocument.Parsed;
  try {
    doc = parseDocument(source, { lineCounter, keepSourceTokens: false });
  } catch {
    return [];
  }
  if (doc.errors.length > 0) {
    return [];
  }
  const root = doc.contents;
  if (!isMap(root)) {
    return [];
  }

  const setupCaptures = collectStepSequence(root, "setup", "setup", undefined);
  const { testName, stepIndex, location } = locateCursor(root, offset);

  if (location === "outside") {
    return setupCaptures;
  }

  if (location === "setup") {
    return setupCaptures.filter((c) => c.stepIndex < stepIndex);
  }

  if (location === "flat-steps") {
    // Simple-format file: `steps:` at the root. The whole document is
    // one pseudo-test; setup captures plus prior steps are visible.
    const flatCaptures = collectStepSequence(
      root,
      "steps",
      "test",
      undefined,
    );
    return [
      ...setupCaptures,
      ...flatCaptures.filter((c) => c.stepIndex < stepIndex),
    ];
  }

  if (location === "test" && testName) {
    const testsNode = getMapValue(root, "tests");
    if (isMap(testsNode)) {
      const testValue = getMapValue(testsNode, testName);
      if (isMap(testValue)) {
        const testCaptures = collectStepSequence(
          testValue,
          "steps",
          "test",
          testName,
        );
        return [
          ...setupCaptures,
          ...testCaptures.filter((c) => c.stepIndex < stepIndex),
        ];
      }
    }
    return setupCaptures;
  }

  if (location === "teardown") {
    // Teardown runs after tests, so every previously declared capture
    // is in scope.
    const testsNode = getMapValue(root, "tests");
    const allTestCaptures: VisibleCapture[] = [];
    if (isMap(testsNode)) {
      for (const pair of testsNode.items) {
        if (!isScalar(pair.key)) continue;
        const name = String(pair.key.value);
        const value = pair.value;
        if (!isMap(value)) continue;
        allTestCaptures.push(
          ...collectStepSequence(value, "steps", "test", name),
        );
      }
    }
    return [...setupCaptures, ...allTestCaptures];
  }

  return setupCaptures;
}

type CursorLocation =
  | "outside"
  | "setup"
  | "flat-steps"
  | "test"
  | "teardown";

interface CursorInfo {
  location: CursorLocation;
  testName?: string;
  stepIndex: number;
}

function locateCursor(root: YAMLMapType, offset: number): CursorInfo {
  const setupSeq = getMapValue(root, "setup");
  if (isSeq(setupSeq)) {
    const idx = indexOfStepContainingOffset(setupSeq, offset);
    if (idx !== undefined) {
      return { location: "setup", stepIndex: idx };
    }
  }

  const flatSteps = getMapValue(root, "steps");
  if (isSeq(flatSteps)) {
    const idx = indexOfStepContainingOffset(flatSteps, offset);
    if (idx !== undefined) {
      return { location: "flat-steps", stepIndex: idx };
    }
  }

  const testsNode = getMapValue(root, "tests");
  if (isMap(testsNode)) {
    for (const pair of testsNode.items) {
      if (!isScalar(pair.key)) continue;
      const name = String(pair.key.value);
      const value = pair.value;
      if (!isMap(value)) continue;
      const stepsSeq = getMapValue(value, "steps");
      if (!isSeq(stepsSeq)) continue;
      const idx = indexOfStepContainingOffset(stepsSeq, offset);
      if (idx !== undefined) {
        return { location: "test", testName: name, stepIndex: idx };
      }
    }
  }

  const teardownSeq = getMapValue(root, "teardown");
  if (isSeq(teardownSeq)) {
    const idx = indexOfStepContainingOffset(teardownSeq, offset);
    if (idx !== undefined) {
      return { location: "teardown", stepIndex: idx };
    }
  }

  return { location: "outside", stepIndex: 0 };
}

function indexOfStepContainingOffset(
  seq: YAMLSeqType,
  offset: number,
): number | undefined {
  const seqRange = getNodeRange(seq);
  if (!seqRange) {
    return undefined;
  }
  const [seqStart, , seqNodeEnd] = seqRange;
  const seqEnd = seqNodeEnd ?? seqRange[1];
  if (offset < seqStart || offset >= seqEnd) {
    // Offset is outside the whole sequence — not our responsibility.
    return undefined;
  }

  const items = seq.items;
  if (items.length === 0) {
    return undefined;
  }

  // For each item, claim the half-open range [item.start, nextItem.start)
  // so lines that belong to item i (like its request/assert/capture
  // sub-blocks) resolve to i even though nodeEnd may report an earlier
  // offset. The final item claims everything up to the sequence end.
  for (let i = 0; i < items.length; i++) {
    const range = getNodeRange(items[i]);
    if (!range) continue;
    const [start] = range;
    const next = items[i + 1];
    const nextRange = next ? getNodeRange(next) : undefined;
    const effectiveEnd = nextRange ? nextRange[0] : seqEnd;
    if (offset >= start && offset < effectiveEnd) {
      return i;
    }
    if (i === 0 && offset < start) {
      return undefined;
    }
  }
  return undefined;
}

function collectStepSequence(
  parent: YAMLMapType,
  key: string,
  phase: "setup" | "test",
  testName: string | undefined,
): VisibleCapture[] {
  const value = getMapValue(parent, key);
  if (!isSeq(value)) {
    return [];
  }
  const captures: VisibleCapture[] = [];
  value.items.forEach((item, index) => {
    if (!isMap(item)) return;
    const stepName = getScalarString(item, "name") ?? `step ${index + 1}`;
    const captureBlock = getMapValue(item, "capture");
    if (!isMap(captureBlock)) return;
    for (const pair of captureBlock.items) {
      if (!isScalar(pair.key)) continue;
      const name = String(pair.key.value);
      captures.push({
        name,
        stepIndex: index,
        phase,
        testName,
        stepName,
      });
    }
  });
  return captures;
}

function getMapValue(map: YAMLMapType, key: string): unknown {
  for (const pair of map.items) {
    if (isScalar(pair.key) && pair.key.value === key) {
      return pair.value;
    }
  }
  return undefined;
}

function getScalarString(map: YAMLMapType, key: string): string | undefined {
  const value = getMapValue(map, key);
  if (isScalar(value)) {
    return typeof value.value === "string" ? value.value : String(value.value);
  }
  return undefined;
}

function getNodeRange(node: unknown): [number, number, number | undefined] | undefined {
  if (!node || typeof node !== "object") return undefined;
  const r = (node as { range?: [number, number, number] }).range;
  if (!r) return undefined;
  return [r[0], r[1], r[2]];
}
