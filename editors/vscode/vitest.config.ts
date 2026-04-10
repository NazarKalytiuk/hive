import { defineConfig } from "vitest/config";
import * as path from "path";

export default defineConfig({
  test: {
    include: ["tests/unit/**/*.test.ts"],
    environment: "node",
    alias: {
      vscode: path.resolve(__dirname, "tests/unit/__mocks__/vscode.ts"),
    },
  },
});
