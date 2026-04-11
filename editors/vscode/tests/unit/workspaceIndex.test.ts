import { describe, it, expect } from "vitest";
import * as vscode from "vscode";
import {
  mergeScopedWithAst,
  rangesStructurallyEqual,
} from "../../src/workspace/WorkspaceIndex";
import type { FileRanges, TestRange } from "../../src/workspace/YamlAst";
import type { ScopedListFileStrict } from "../../src/util/schemaGuards";

const astRange = (startLine: number): vscode.Range =>
  new vscode.Range(
    new vscode.Position(startLine, 0),
    new vscode.Position(startLine, 10),
  );

const makeAst = (tests: TestRange[]): FileRanges => ({
  fileName: "Health check",
  fileNameRange: astRange(1),
  tests,
  setup: [],
  teardown: [],
});

describe("mergeScopedWithAst (NAZ-282)", () => {
  it("uses Tarn's tests list verbatim and overlays AST ranges by name", () => {
    // The AST reports two tests with real ranges; Tarn reports the
    // same two tests (authoritative list). The merge must carry
    // Tarn's names forward and attach the AST ranges at matching
    // indices so downstream CodeLens and TestItem gutter icons land
    // on the correct lines.
    const scoped: ScopedListFileStrict = {
      file: "/tmp/fixture.tarn.yaml",
      name: "Fixture: health check",
      setup: [],
      steps: [],
      teardown: [],
      tests: [
        {
          name: "service_is_up",
          description: "Pings /status/200",
          steps: [{ name: "GET /status/200" }],
        },
        {
          name: "service_is_down",
          description: null,
          steps: [
            { name: "GET /status/500" },
            { name: "GET /status/503" },
          ],
        },
      ],
    };
    const ast = makeAst([
      {
        name: "service_is_up",
        description: null,
        nameRange: astRange(5),
        steps: [{ index: 0, name: "GET /status/200", nameRange: astRange(7) }],
      },
      {
        name: "service_is_down",
        description: null,
        nameRange: astRange(10),
        steps: [
          { index: 0, name: "GET /status/500", nameRange: astRange(12) },
          { index: 1, name: "GET /status/503", nameRange: astRange(14) },
        ],
      },
    ]);

    const merged = mergeScopedWithAst(scoped, ast);

    expect(merged.fileName).toBe("Fixture: health check");
    expect(merged.fileNameRange).toBe(ast.fileNameRange);
    expect(merged.tests).toHaveLength(2);

    expect(merged.tests[0].name).toBe("service_is_up");
    expect(merged.tests[0].description).toBe("Pings /status/200");
    expect(merged.tests[0].nameRange).toBe(ast.tests[0].nameRange);
    expect(merged.tests[0].steps).toHaveLength(1);
    expect(merged.tests[0].steps[0].name).toBe("GET /status/200");
    expect(merged.tests[0].steps[0].nameRange).toBe(ast.tests[0].steps[0].nameRange);

    expect(merged.tests[1].name).toBe("service_is_down");
    expect(merged.tests[1].steps).toHaveLength(2);
    expect(merged.tests[1].steps[0].nameRange).toBe(ast.tests[1].steps[0].nameRange);
    expect(merged.tests[1].steps[1].nameRange).toBe(ast.tests[1].steps[1].nameRange);
  });

  it("falls back to a zero-width range for tests/steps only Tarn knows about", () => {
    // Tarn post-expands `include:` directives so the scoped list can
    // reference a step that does not appear in the raw YAML. The
    // merge must still expose that step (it is the authoritative
    // structure) with a zero-width range so CodeLens providers do
    // not crash on a missing `nameRange`.
    const scoped: ScopedListFileStrict = {
      file: "/tmp/fixture.tarn.yaml",
      name: "With include",
      setup: [],
      steps: [],
      teardown: [],
      tests: [
        {
          name: "hidden_from_ast",
          description: null,
          steps: [{ name: "synthetic step from include" }],
        },
      ],
    };
    const ast = makeAst([]);

    const merged = mergeScopedWithAst(scoped, ast);
    expect(merged.tests).toHaveLength(1);
    expect(merged.tests[0].name).toBe("hidden_from_ast");
    expect(merged.tests[0].nameRange.start.line).toBe(0);
    expect(merged.tests[0].nameRange.start.character).toBe(0);
    expect(merged.tests[0].steps[0].name).toBe("synthetic step from include");
    expect(merged.tests[0].steps[0].nameRange.start.line).toBe(0);
  });

  it("folds flat `steps:` into a synthetic `default` test with the AST range", () => {
    // Tarn reports the top-level `steps:` form on `files[0].steps`
    // and leaves `tests[]` empty. The merge must fold this into a
    // single `default` test so downstream consumers see a uniform
    // tests[] list regardless of the YAML variant the user wrote.
    const scoped: ScopedListFileStrict = {
      file: "/tmp/flat.tarn.yaml",
      name: "Flat",
      setup: [],
      steps: [{ name: "GET /health" }, { name: "GET /ready" }],
      teardown: [],
      tests: [],
    };
    const ast = makeAst([
      {
        name: "default",
        description: null,
        nameRange: astRange(3),
        steps: [
          { index: 0, name: "GET /health", nameRange: astRange(5) },
          { index: 1, name: "GET /ready", nameRange: astRange(7) },
        ],
      },
    ]);

    const merged = mergeScopedWithAst(scoped, ast);
    expect(merged.tests).toHaveLength(1);
    expect(merged.tests[0].name).toBe("default");
    expect(merged.tests[0].nameRange).toBe(ast.tests[0].nameRange);
    expect(merged.tests[0].steps.map((s) => s.name)).toEqual([
      "GET /health",
      "GET /ready",
    ]);
    expect(merged.tests[0].steps[0].nameRange).toBe(
      ast.tests[0].steps[0].nameRange,
    );
  });

  it("carries setup and teardown from Tarn with AST ranges overlaid", () => {
    const scoped: ScopedListFileStrict = {
      file: "/tmp/with-setup.tarn.yaml",
      name: "With setup",
      setup: [{ name: "seed db" }, { name: "warm cache" }],
      steps: [],
      teardown: [{ name: "truncate db" }],
      tests: [],
    };
    const ast: FileRanges = {
      fileName: "With setup",
      fileNameRange: astRange(1),
      tests: [],
      setup: [
        { index: 0, name: "seed db", nameRange: astRange(3) },
        { index: 1, name: "warm cache", nameRange: astRange(5) },
      ],
      teardown: [{ index: 0, name: "truncate db", nameRange: astRange(20) }],
    };

    const merged = mergeScopedWithAst(scoped, ast);
    expect(merged.setup).toHaveLength(2);
    expect(merged.setup[0].nameRange).toBe(ast.setup[0].nameRange);
    expect(merged.setup[1].nameRange).toBe(ast.setup[1].nameRange);
    expect(merged.teardown).toHaveLength(1);
    expect(merged.teardown[0].nameRange).toBe(ast.teardown[0].nameRange);
  });

  it("accepts an undefined AST (e.g., read error) and returns zero-width ranges", () => {
    // If the file was deleted or unreadable between the watcher
    // event and the AST read, merging against `undefined` must not
    // throw — the scoped list is still authoritative for structure.
    const scoped: ScopedListFileStrict = {
      file: "/tmp/fixture.tarn.yaml",
      name: "Orphan",
      setup: [],
      steps: [],
      teardown: [],
      tests: [
        {
          name: "smoke",
          description: null,
          steps: [{ name: "GET /" }],
        },
      ],
    };
    const merged = mergeScopedWithAst(scoped, undefined);
    expect(merged.fileName).toBe("Orphan");
    expect(merged.tests[0].name).toBe("smoke");
    expect(merged.tests[0].nameRange.start.line).toBe(0);
  });
});

describe("rangesStructurallyEqual (NAZ-282)", () => {
  const baseTests: TestRange[] = [
    {
      name: "smoke",
      description: "health",
      nameRange: astRange(5),
      steps: [{ index: 0, name: "GET /health", nameRange: astRange(7) }],
    },
  ];

  it("returns true for two identical structures", () => {
    const a = makeAst(baseTests);
    const b = makeAst([
      {
        name: "smoke",
        description: "health",
        nameRange: astRange(5),
        steps: [{ index: 0, name: "GET /health", nameRange: astRange(7) }],
      },
    ]);
    expect(rangesStructurallyEqual(a, b)).toBe(true);
  });

  it("ignores line-number shifts on otherwise identical structures", () => {
    // Adding a comment at the top of a file shifts every range by
    // one line but does not change the test tree. The watcher must
    // NOT fire a TestItem rebuild for a shift that only moves the
    // anchor — the structural comparison must look at names, arity,
    // and description only.
    const a = makeAst(baseTests);
    const shifted = makeAst([
      {
        name: "smoke",
        description: "health",
        nameRange: astRange(42),
        steps: [{ index: 0, name: "GET /health", nameRange: astRange(44) }],
      },
    ]);
    expect(rangesStructurallyEqual(a, shifted)).toBe(true);
  });

  it("detects a renamed test", () => {
    const renamed = makeAst([
      {
        name: "smoke_renamed",
        description: "health",
        nameRange: astRange(5),
        steps: [{ index: 0, name: "GET /health", nameRange: astRange(7) }],
      },
    ]);
    expect(rangesStructurallyEqual(makeAst(baseTests), renamed)).toBe(false);
  });

  it("detects an added step", () => {
    const withExtra = makeAst([
      {
        name: "smoke",
        description: "health",
        nameRange: astRange(5),
        steps: [
          { index: 0, name: "GET /health", nameRange: astRange(7) },
          { index: 1, name: "GET /ready", nameRange: astRange(9) },
        ],
      },
    ]);
    expect(rangesStructurallyEqual(makeAst(baseTests), withExtra)).toBe(false);
  });

  it("detects a description change", () => {
    const reDescribed = makeAst([
      {
        name: "smoke",
        description: "different",
        nameRange: astRange(5),
        steps: [{ index: 0, name: "GET /health", nameRange: astRange(7) }],
      },
    ]);
    expect(rangesStructurallyEqual(makeAst(baseTests), reDescribed)).toBe(
      false,
    );
  });

  it("detects a changed file-level name", () => {
    const a = makeAst(baseTests);
    const b = makeAst(baseTests);
    b.fileName = "Different";
    expect(rangesStructurallyEqual(a, b)).toBe(false);
  });

  it("detects a renamed step while the test name stays the same", () => {
    const renamedStep = makeAst([
      {
        name: "smoke",
        description: "health",
        nameRange: astRange(5),
        steps: [{ index: 0, name: "GET /healthz", nameRange: astRange(7) }],
      },
    ]);
    expect(rangesStructurallyEqual(makeAst(baseTests), renamedStep)).toBe(
      false,
    );
  });

  it("detects setup or teardown arity changes", () => {
    const a: FileRanges = {
      fileName: "x",
      fileNameRange: undefined,
      tests: [],
      setup: [{ index: 0, name: "seed", nameRange: astRange(2) }],
      teardown: [],
    };
    const b: FileRanges = {
      fileName: "x",
      fileNameRange: undefined,
      tests: [],
      setup: [],
      teardown: [],
    };
    expect(rangesStructurallyEqual(a, b)).toBe(false);
  });
});
