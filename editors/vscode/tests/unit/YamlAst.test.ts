import { describe, it, expect } from "vitest";
import { parseYamlFile } from "../../src/workspace/YamlAst";

const fullFile = `version: "1"
name: "User CRUD Operations"
description: "Tests complete user lifecycle"
tags: [crud, users]

setup:
  - name: Authenticate
    request:
      method: POST
      url: "http://localhost/auth"
    capture:
      token: "$.token"
    assert:
      status: 200

teardown:
  - name: Clean up
    request:
      method: POST
      url: "http://localhost/cleanup"

tests:
  create_and_verify_user:
    description: "Create then fetch"
    steps:
      - name: Create user
        request:
          method: POST
          url: "http://localhost/users"
      - name: Fetch user
        request:
          method: GET
          url: "http://localhost/users/1"
  delete_user:
    steps:
      - name: Delete user
        request:
          method: DELETE
          url: "http://localhost/users/1"
`;

const flatStepsFile = `name: "Flat file"
steps:
  - name: One
    request:
      method: GET
      url: "http://localhost/1"
  - name: Two
    request:
      method: GET
      url: "http://localhost/2"
`;

const brokenFile = `name: "Broken
steps:
  - name: Unclosed
`;

describe("parseYamlFile", () => {
  it("extracts file name and its range", () => {
    const r = parseYamlFile(fullFile);
    expect(r.fileName).toBe("User CRUD Operations");
    expect(r.fileNameRange).toBeDefined();
    expect(r.fileNameRange?.start.line).toBe(1);
  });

  it("collects all tests in order with their step counts", () => {
    const r = parseYamlFile(fullFile);
    expect(r.tests).toHaveLength(2);
    expect(r.tests[0].name).toBe("create_and_verify_user");
    expect(r.tests[0].description).toBe("Create then fetch");
    expect(r.tests[0].steps).toHaveLength(2);
    expect(r.tests[0].steps[0].name).toBe("Create user");
    expect(r.tests[0].steps[0].index).toBe(0);
    expect(r.tests[0].steps[1].name).toBe("Fetch user");
    expect(r.tests[0].steps[1].index).toBe(1);
    expect(r.tests[1].name).toBe("delete_user");
    expect(r.tests[1].steps).toHaveLength(1);
  });

  it("collects setup and teardown steps", () => {
    const r = parseYamlFile(fullFile);
    expect(r.setup).toHaveLength(1);
    expect(r.setup[0].name).toBe("Authenticate");
    expect(r.teardown).toHaveLength(1);
    expect(r.teardown[0].name).toBe("Clean up");
  });

  it("anchors step names to non-zero ranges", () => {
    const r = parseYamlFile(fullFile);
    const firstStep = r.tests[0].steps[0];
    expect(firstStep.nameRange.start.line).toBeGreaterThan(0);
  });

  it("handles a file with flat steps as a single default test", () => {
    const r = parseYamlFile(flatStepsFile);
    expect(r.fileName).toBe("Flat file");
    expect(r.tests).toHaveLength(1);
    expect(r.tests[0].name).toBe("default");
    expect(r.tests[0].steps).toHaveLength(2);
    expect(r.tests[0].steps[0].name).toBe("One");
    expect(r.tests[0].steps[1].name).toBe("Two");
  });

  it("returns a parse error for malformed YAML without throwing", () => {
    const r = parseYamlFile(brokenFile);
    expect(r.parseError).toBeDefined();
    expect(r.tests).toHaveLength(0);
  });
});
