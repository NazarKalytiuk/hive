import { describe, it, expect } from "vitest";
import { collectVisibleCaptures } from "../../src/language/completion/captures";

const SOURCE = `name: Fixture
setup:
  - name: Authenticate
    request:
      method: POST
      url: "http://localhost/auth"
    capture:
      auth_token: "$.token"
      session_id: "$.sid"

tests:
  create_flow:
    steps:
      - name: Create user
        request:
          method: POST
          url: "http://localhost/users"
        capture:
          user_id: "$.id"
          created_at: "$.createdAt"
      - name: Verify user
        request:
          method: GET
          url: "http://localhost/users/1"
        capture:
          display_name: "$.name"
      - name: Delete user
        request:
          method: DELETE
          url: "http://localhost/users/1"
  other_flow:
    steps:
      - name: Unrelated
        request:
          method: GET
          url: "http://localhost/ping"
        capture:
          pong: "$.pong"

teardown:
  - name: Clean up
    request:
      method: POST
      url: "http://localhost/cleanup"
`;

function offsetOf(needle: string): number {
  const idx = SOURCE.indexOf(needle);
  if (idx < 0) {
    throw new Error(`needle not found in source: ${needle}`);
  }
  return idx;
}

describe("collectVisibleCaptures", () => {
  it("returns setup captures when cursor is outside any step", () => {
    // offset at the top of the document — before setup runs
    const captures = collectVisibleCaptures(SOURCE, 0);
    const names = captures.map((c) => c.name).sort();
    expect(names).toEqual(["auth_token", "session_id"]);
  });

  it("earlier setup steps are visible from later setup steps", () => {
    // Setup has only one step, so no earlier steps. Returns empty.
    const offset = offsetOf("http://localhost/auth");
    const captures = collectVisibleCaptures(SOURCE, offset);
    const names = captures.map((c) => c.name);
    expect(names).toEqual([]);
  });

  it("inside the first test step, only setup captures are visible", () => {
    const offset = offsetOf("http://localhost/users\"");
    const captures = collectVisibleCaptures(SOURCE, offset);
    const names = captures.map((c) => c.name).sort();
    expect(names).toEqual(["auth_token", "session_id"]);
  });

  it("inside the second test step, captures from step 0 are visible plus setup", () => {
    const offset = offsetOf("http://localhost/users/1\"");
    const captures = collectVisibleCaptures(SOURCE, offset);
    const names = captures.map((c) => c.name).sort();
    expect(names).toEqual(["auth_token", "created_at", "session_id", "user_id"]);
  });

  it("inside the third test step, captures from steps 0 and 1 are visible", () => {
    const offset = offsetOf("method: DELETE");
    const captures = collectVisibleCaptures(SOURCE, offset);
    const names = captures.map((c) => c.name).sort();
    expect(names).toEqual([
      "auth_token",
      "created_at",
      "display_name",
      "session_id",
      "user_id",
    ]);
  });

  it("captures from another test are not visible", () => {
    const offset = offsetOf("method: DELETE");
    const captures = collectVisibleCaptures(SOURCE, offset);
    expect(captures.find((c) => c.name === "pong")).toBeUndefined();
  });

  it("inside a sibling test, the first test's captures are not visible", () => {
    const offset = offsetOf("http://localhost/ping");
    const captures = collectVisibleCaptures(SOURCE, offset);
    const names = captures.map((c) => c.name).sort();
    expect(names).toEqual(["auth_token", "session_id"]);
  });

  it("inside teardown, every declared capture is in scope", () => {
    const offset = offsetOf("http://localhost/cleanup");
    const captures = collectVisibleCaptures(SOURCE, offset);
    const names = captures.map((c) => c.name).sort();
    expect(names).toEqual([
      "auth_token",
      "created_at",
      "display_name",
      "pong",
      "session_id",
      "user_id",
    ]);
  });

  it("returns nothing for malformed YAML without throwing", () => {
    const broken = `name: "unclosed
tests:
  t:
    steps:
      - name: s
`;
    expect(() => collectVisibleCaptures(broken, 20)).not.toThrow();
    expect(collectVisibleCaptures(broken, 20)).toEqual([]);
  });
});
