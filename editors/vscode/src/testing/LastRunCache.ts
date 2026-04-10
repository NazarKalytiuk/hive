import type { Report, StepResult, TestResult, FileResult } from "../util/schemaGuards";

/**
 * Stable step key combining the file path, test name, and 0-based
 * step index. Used by both the LastRunCache and the external
 * `tarn.showStepDetails` command.
 */
export interface StepKey {
  file: string;
  test: string;
  stepIndex: number;
}

/**
 * Snapshot of a step result with enough context for the Request/
 * Response Inspector webview to render the step on its own.
 */
export interface StepSnapshot {
  key: StepKey;
  stepName: string;
  fileName: string;
  testDescription?: string;
  phase: "setup" | "test" | "teardown";
  step: StepResult;
}

/**
 * In-memory cache of the most recent run's step results, keyed by
 * a `file::test::index` string. The runHandler populates it after
 * every successful run (replacing the previous contents); the
 * Request/Response Inspector command reads from it.
 *
 * Kept deliberately tiny and memory-only: persisting full reports
 * across sessions would bloat workspaceState and the cache only
 * needs to survive for the lifetime of a single VS Code window.
 */
export class LastRunCache {
  private entries = new Map<string, StepSnapshot>();

  /** Replace every entry with the step results from a new report. */
  loadFromReport(report: Report): void {
    this.entries.clear();
    for (const file of report.files) {
      this.indexFile(file);
    }
  }

  get(key: StepKey): StepSnapshot | undefined {
    return this.entries.get(encode(key));
  }

  getByEncoded(encodedKey: string): StepSnapshot | undefined {
    return this.entries.get(encodedKey);
  }

  size(): number {
    return this.entries.size;
  }

  clear(): void {
    this.entries.clear();
  }

  private indexFile(file: FileResult): void {
    if (file.setup) {
      this.indexSteps(file, undefined, "setup", file.setup);
    }
    for (const test of file.tests) {
      this.indexSteps(file, test, "test", test.steps);
    }
    if (file.teardown) {
      this.indexSteps(file, undefined, "teardown", file.teardown);
    }
  }

  private indexSteps(
    file: FileResult,
    test: TestResult | undefined,
    phase: "setup" | "test" | "teardown",
    steps: readonly StepResult[],
  ): void {
    steps.forEach((step, index) => {
      const testName = test?.name ?? phase;
      const key: StepKey = {
        file: file.file,
        test: testName,
        stepIndex: index,
      };
      this.entries.set(encode(key), {
        key,
        stepName: step.name,
        fileName: file.name,
        testDescription: test?.description ?? undefined,
        phase,
        step,
      });
    });
  }
}

export function encode(key: StepKey): string {
  return `${key.file}::${key.test}::${key.stepIndex}`;
}

export function decode(raw: string): StepKey | undefined {
  const lastSep = raw.lastIndexOf("::");
  if (lastSep < 0) return undefined;
  const indexStr = raw.slice(lastSep + 2);
  const stepIndex = Number(indexStr);
  if (!Number.isFinite(stepIndex)) return undefined;
  const prefix = raw.slice(0, lastSep);
  const testSep = prefix.lastIndexOf("::");
  if (testSep < 0) return undefined;
  const file = prefix.slice(0, testSep);
  const test = prefix.slice(testSep + 2);
  if (!file || !test) return undefined;
  return { file, test, stepIndex };
}
