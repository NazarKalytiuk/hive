import * as path from "path";
import { runTests } from "@vscode/test-electron";

async function main(): Promise<void> {
  try {
    const extensionDevelopmentPath = path.resolve(__dirname, "../../..");
    const extensionTestsPath = path.resolve(__dirname, "./suite/index");
    const fixtureWorkspace = path.resolve(__dirname, "../fixtures/workspace");

    await runTests({
      extensionDevelopmentPath,
      extensionTestsPath,
      launchArgs: [fixtureWorkspace, "--disable-extensions"],
    });
  } catch (err) {
    console.error("Failed to run integration tests:", err);
    process.exit(1);
  }
}

main();
