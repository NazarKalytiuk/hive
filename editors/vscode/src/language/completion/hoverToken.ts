/**
 * Shape of a `{{ ... }}` token resolved for a hover at a given cursor
 * offset in a single line. `rangeStart` and `rangeEnd` are column
 * offsets *within the line* (not the whole document), so callers can
 * convert them to VS Code `Position`s themselves.
 */
export type HoverToken =
  | { kind: "none" }
  | {
      kind: "env" | "capture" | "builtin" | "empty";
      identifier: string;
      rangeStart: number;
      rangeEnd: number;
    };

/**
 * Find the `{{ ... }}` expression enclosing a cursor column in a line
 * of source. Returns `{ kind: "none" }` if the cursor is outside any
 * interpolation, inside an expression that does not match one of the
 * supported prefixes (env, capture, $), or inside a malformed pair.
 *
 * Uses a single-pass scan so it stays O(line length) even on lines
 * with multiple interpolations.
 */
export function findHoverToken(line: string, column: number): HoverToken {
  let searchFrom = 0;
  while (searchFrom < line.length) {
    const openIdx = line.indexOf("{{", searchFrom);
    if (openIdx < 0) {
      return { kind: "none" };
    }
    const closeIdx = line.indexOf("}}", openIdx + 2);
    if (closeIdx < 0) {
      return { kind: "none" };
    }
    const end = closeIdx + 2;
    // Cursor must sit strictly inside the {{ … }} range (between
    // the opening `{{` and the closing `}}`). This also matches a
    // cursor placed on the `{` or `}` characters themselves, which
    // users hover on occasionally.
    if (column >= openIdx && column <= end) {
      const raw = line.slice(openIdx + 2, closeIdx).trim();
      return classifyExpression(raw, openIdx, end);
    }
    searchFrom = end;
  }
  return { kind: "none" };
}

function classifyExpression(
  raw: string,
  rangeStart: number,
  rangeEnd: number,
): HoverToken {
  if (raw === "") {
    return { kind: "empty", identifier: "", rangeStart, rangeEnd };
  }
  if (raw.startsWith("env.")) {
    return {
      kind: "env",
      identifier: raw.slice(4).trim(),
      rangeStart,
      rangeEnd,
    };
  }
  if (raw === "env") {
    return { kind: "env", identifier: "", rangeStart, rangeEnd };
  }
  if (raw.startsWith("capture.")) {
    return {
      kind: "capture",
      identifier: raw.slice(8).trim(),
      rangeStart,
      rangeEnd,
    };
  }
  if (raw === "capture") {
    return { kind: "capture", identifier: "", rangeStart, rangeEnd };
  }
  if (raw.startsWith("$")) {
    // Everything after the `$` up to the first `(` is the function
    // name. `$random_hex(8)` → `random_hex`.
    const afterDollar = raw.slice(1);
    const parenIdx = afterDollar.indexOf("(");
    const name = parenIdx >= 0 ? afterDollar.slice(0, parenIdx) : afterDollar;
    return {
      kind: "builtin",
      identifier: name.trim(),
      rangeStart,
      rangeEnd,
    };
  }
  return { kind: "none" };
}
