import * as assert from "assert";
import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as vscode from "vscode";

const EXTENSION_ID = "nazarkalytiuk.tarn-vscode";

interface InitProjectOptionsShape {
  folder: string;
  flavor: "basic" | "all";
  envOverrides?: Record<string, string>;
}

interface InitProjectOutcomeShape {
  success: boolean;
  folder: string;
  created: string[];
  deleted: string[];
  validationErrors: number;
  errorMessage?: string;
}

interface TarnExtensionApiShape {
  readonly commands: readonly string[];
  readonly testing: {
    readonly initProject: (
      options: InitProjectOptionsShape,
    ) => Promise<InitProjectOutcomeShape>;
  };
}

async function getApi(): Promise<TarnExtensionApiShape> {
  const ext = vscode.extensions.getExtension<TarnExtensionApiShape>(EXTENSION_ID);
  assert.ok(ext, `extension ${EXTENSION_ID} not found`);
  const api = await ext!.activate();
  assert.ok(api, "extension activated but returned no API");
  return api;
}

async function makeTempFolder(): Promise<string> {
  return fs.promises.mkdtemp(path.join(os.tmpdir(), "tarn-vscode-init-"));
}

describe("Init Project wizard (tarn.initProject)", () => {
  let api: TarnExtensionApiShape;
  const tmpDirs: string[] = [];

  before(async function () {
    this.timeout(60000);
    api = await getApi();
  });

  after(async () => {
    await Promise.all(
      tmpDirs.map((d) =>
        fs.promises.rm(d, { recursive: true, force: true }).catch(() => {}),
      ),
    );
  });

  it("registers the tarn.initProject command", async () => {
    const commands = await vscode.commands.getCommands(true);
    assert.ok(
      commands.includes("tarn.initProject"),
      "tarn.initProject should be registered",
    );
  });

  it("scaffolds an 'all templates' project and validates cleanly", async function () {
    this.timeout(30000);
    const dir = await makeTempFolder();
    tmpDirs.push(dir);
    const outcome = await api.testing.initProject({
      folder: dir,
      flavor: "all",
    });
    assert.strictEqual(outcome.success, true, outcome.errorMessage ?? "init failed");
    assert.strictEqual(outcome.validationErrors, 0);
    // 'all' flavor keeps every scaffold file.
    assert.deepStrictEqual(outcome.deleted, []);
    assert.ok(outcome.created.includes("tarn.config.yaml"));
    assert.ok(outcome.created.includes("tarn.env.yaml"));
    assert.ok(
      outcome.created.some((f) => f.endsWith("tests/health.tarn.yaml")),
      `expected tests/health.tarn.yaml among created: ${outcome.created.join(", ")}`,
    );
    assert.ok(
      outcome.created.some((f) => f.startsWith("examples")),
      "expected at least one file under examples/",
    );
  });

  it("scaffolds a 'basic' project and prunes examples + fixtures", async function () {
    this.timeout(30000);
    const dir = await makeTempFolder();
    tmpDirs.push(dir);
    const outcome = await api.testing.initProject({
      folder: dir,
      flavor: "basic",
    });
    assert.strictEqual(outcome.success, true);
    assert.strictEqual(outcome.validationErrors, 0);
    assert.deepStrictEqual(outcome.deleted.sort(), ["examples", "fixtures"].sort());
    // Health check survives.
    assert.ok(
      outcome.created.some((f) => f.endsWith("tests/health.tarn.yaml")),
    );
    // Examples are gone.
    assert.ok(
      !outcome.created.some((f) => f.startsWith("examples")),
      `unexpected example files: ${outcome.created
        .filter((f) => f.startsWith("examples"))
        .join(", ")}`,
    );
    const examplesExists = await fs.promises
      .stat(path.join(dir, "examples"))
      .then(() => true)
      .catch(() => false);
    assert.strictEqual(examplesExists, false, "examples/ should not exist after basic scaffold");
  });

  it("rewrites tarn.env.yaml with user-supplied overrides", async function () {
    this.timeout(30000);
    const dir = await makeTempFolder();
    tmpDirs.push(dir);
    const outcome = await api.testing.initProject({
      folder: dir,
      flavor: "basic",
      envOverrides: {
        base_url: "https://staging.example.com",
        admin_email: "ops@acme.dev",
        admin_password: "hunter2",
      },
    });
    assert.strictEqual(outcome.success, true);
    const env = await fs.promises.readFile(path.join(dir, "tarn.env.yaml"), "utf8");
    assert.ok(
      env.includes('base_url: "https://staging.example.com"'),
      `base_url not rewritten: ${env}`,
    );
    assert.ok(env.includes('admin_email: "ops@acme.dev"'));
    assert.ok(env.includes("admin_password: hunter2"));
  });
});
