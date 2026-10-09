import { describe, expect, it } from "vitest";
import {
  ENV_MAX_BYTES,
  ENV_NAME_MAX_CHARS,
  ENV_VALUE_MAX_CHARS,
  envInput,
  envRows,
  envTooLarge,
  isBlankEnvRow,
  newEnvRow,
  validateEnvRows,
  type EnvRow,
} from "./envVars";

const row = (name: string, value = ""): EnvRow => ({ ...newEnvRow(), name, value });

describe("env rows (SSH-14)", () => {
  it("loads saved variables with their own ids", () => {
    const rows = envRows([
      { name: "TZ", value: "Asia/Tokyo" },
      { name: "LANG", value: "" },
    ]);
    expect(rows.map((r) => [r.name, r.value])).toEqual([
      ["TZ", "Asia/Tokyo"],
      ["LANG", ""],
    ]);
    expect(new Set(rows.map((r) => r.uid)).size).toBe(2);
  });

  it("accepts shell variable names and any one-line value", () => {
    const rows = [row("LANG", "ja_JP.UTF-8"), row("_x1"), row("lang", "names are case-sensitive"), row(" TZ ", "a = b\tc")];
    expect(validateEnvRows(rows)).toEqual([null, null, null, null]);
  });

  it("names each problem", () => {
    expect(validateEnvRows([row("", "x")])).toEqual([{ name: "empty" }]);
    for (const bad of ["1ST", "A-B", "A B", "A=B", "日本", "a.b"]) expect(validateEnvRows([row(bad)])).toEqual([{ name: "invalid" }]);
    expect(validateEnvRows([row("N".repeat(ENV_NAME_MAX_CHARS))])).toEqual([null]);
    expect(validateEnvRows([row("N".repeat(ENV_NAME_MAX_CHARS + 1))])).toEqual([{ name: "tooLong" }]);
    expect(validateEnvRows([row("A", "1"), row("B"), row(" A ", "2")])).toEqual([null, null, { name: "duplicate" }]);
    for (const bad of ["a\nb", "a\rb", "a\0b", "a\x1bb", "a\x7fb", "a\u0085b"])
      expect(validateEnvRows([row("V", bad)])).toEqual([{ value: "invalid" }]);
    expect(validateEnvRows([row("V", "a\tb")])).toEqual([null]);
    expect(validateEnvRows([row("V", "値".repeat(ENV_VALUE_MAX_CHARS))])).toEqual([null]);
    expect(validateEnvRows([row("V", "値".repeat(ENV_VALUE_MAX_CHARS + 1))])).toEqual([{ value: "tooLong" }]);
    expect(validateEnvRows([row("1", "\n")])).toEqual([{ name: "invalid", value: "invalid" }]);
  });

  it("counts names and values together in UTF-8 bytes, as Rust does", () => {
    const value = (n: number) => "x".repeat(n);
    const rows = [row("A", value(4_096)), row("B", value(4_096)), row("C", value(4_096)), row("D", value(ENV_MAX_BYTES - 3 * 4_096 - 4))];
    expect(envTooLarge(rows)).toBe(false);
    rows[3].value += "x";
    expect(envTooLarge(rows)).toBe(true);
    // 3 bytes per character, and blank rows don't count.
    expect(envTooLarge([row("V", "値".repeat(4_096)), row("W", "値".repeat(1_364)), row("", "")])).toBe(false);
    expect(envTooLarge([row("V", "値".repeat(4_096)), row("W", "値".repeat(1_365))])).toBe(true);
  });

  it("ignores and drops blank rows", () => {
    const rows = [row(""), row("  ", "  "), row(" TZ ", " UTC ")];
    expect(rows.map(isBlankEnvRow)).toEqual([true, true, false]);
    expect(validateEnvRows(rows)).toEqual([null, null, null]);
    expect(envInput(rows)).toEqual([{ name: "TZ", value: " UTC " }]);
  });
});
