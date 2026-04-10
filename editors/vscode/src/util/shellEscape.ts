export function quoteArgForLog(arg: string): string {
  if (arg === "") {
    return "''";
  }
  if (/^[A-Za-z0-9_\-./=:@+,]+$/.test(arg)) {
    return arg;
  }
  return `'${arg.replace(/'/g, `'\\''`)}'`;
}

export function formatCommandForLog(bin: string, args: readonly string[]): string {
  return [bin, ...args].map(quoteArgForLog).join(" ");
}
