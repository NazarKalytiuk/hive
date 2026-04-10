import { describe, it, expect } from "vitest";
import * as fs from "fs";
import * as path from "path";
import { parseYamlFile } from "../../src/workspace/YamlAst";

const EXAMPLES_ROOT = path.resolve(__dirname, "../../../../examples");

function collectYamlFiles(root: string): string[] {
  const out: string[] = [];
  const walk = (dir: string) => {
    if (!fs.existsSync(dir)) return;
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        walk(full);
      } else if (entry.isFile() && /\.tarn\.ya?ml$/.test(entry.name)) {
        out.push(full);
      }
    }
  };
  walk(root);
  return out;
}

describe("YamlAst fixture sweep (examples/*.tarn.yaml)", () => {
  const files = collectYamlFiles(EXAMPLES_ROOT);

  it("finds at least one example file", () => {
    expect(files.length).toBeGreaterThan(0);
  });

  it.each(files)("parses %s without throwing", (file) => {
    const source = fs.readFileSync(file, "utf8");
    const parsed = parseYamlFile(source);
    expect(parsed.parseError).toBeUndefined();
  });

  it.each(files)("returns a non-empty file name for %s", (file) => {
    const source = fs.readFileSync(file, "utf8");
    const parsed = parseYamlFile(source);
    expect(parsed.fileName.length).toBeGreaterThan(0);
  });

  it.each(files)("produces ranges with non-negative lines for %s", (file) => {
    const source = fs.readFileSync(file, "utf8");
    const parsed = parseYamlFile(source);
    for (const test of parsed.tests) {
      expect(test.nameRange.start.line).toBeGreaterThanOrEqual(0);
      for (const step of test.steps) {
        expect(step.nameRange.start.line).toBeGreaterThanOrEqual(0);
      }
    }
  });
});
