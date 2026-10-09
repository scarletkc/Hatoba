import { charCount } from "@/features/ai/instructions";
import type { HostEnvVar } from "@/ipc/types";

/*
 * SSH-14: a host's environment variables while the host editor edits them. No React, no i18n. Rust checks the same
 * rules when the host is saved (`check_host_env` in crates/hatoba-core/src/model.rs).
 */

/** The most variables a host can have. */
export const ENV_MAX_VARS = 64;
/** The longest name, in characters. */
export const ENV_NAME_MAX_CHARS = 128;
/** The longest value, in characters. */
export const ENV_VALUE_MAX_CHARS = 4_096;
/** The most UTF-8 bytes the names and values take together. */
export const ENV_MAX_BYTES = 16 * 1024;

export interface EnvRow {
  uid: number;
  name: string;
  value: string;
}

let nextUid = 1;

export const newEnvRow = (): EnvRow => ({ uid: nextUid++, name: "", value: "" });

export const envRows = (vars: readonly HostEnvVar[]): EnvRow[] =>
  vars.map((v) => ({ uid: nextUid++, name: v.name, value: v.value }));

export interface EnvRowProblem {
  name?: "empty" | "invalid" | "tooLong" | "duplicate";
  value?: "invalid" | "tooLong";
}

/** A row the user added and left empty: it is dropped when saving. */
export const isBlankEnvRow = (row: EnvRow): boolean => row.name.trim() === "" && row.value.trim() === "";

// What a POSIX shell accepts as a variable name.
const NAME = /^[A-Za-z_][A-Za-z0-9_]*$/;
// Control characters other than tab, as Rust's `char::is_control` sees them.
const BAD_VALUE = /[\u0000-\u0008\u000a-\u001f\u007f-\u009f]/;

/**
 * The problems of each row (null when it is fine), index for index with `rows`. Blank rows are fine. Names are
 * compared as typed, since the server's are case-sensitive.
 */
export function validateEnvRows(rows: readonly EnvRow[]): (EnvRowProblem | null)[] {
  const seen = new Set<string>();
  return rows.map((row) => {
    if (isBlankEnvRow(row)) return null;
    const problem: EnvRowProblem = {};
    const name = row.name.trim();
    if (name === "") problem.name = "empty";
    else if (charCount(name) > ENV_NAME_MAX_CHARS) problem.name = "tooLong";
    else if (!NAME.test(name)) problem.name = "invalid";
    else if (seen.has(name)) problem.name = "duplicate";
    seen.add(name);
    if (BAD_VALUE.test(row.value)) problem.value = "invalid";
    else if (charCount(row.value) > ENV_VALUE_MAX_CHARS) problem.value = "tooLong";
    return Object.keys(problem).length ? problem : null;
  });
}

const utf8 = new TextEncoder();

/** Whether the rows that are kept take more than `ENV_MAX_BYTES` together, counted as Rust counts them. */
export function envTooLarge(rows: readonly EnvRow[]): boolean {
  const bytes = envInput(rows).reduce((sum, v) => sum + utf8.encode(v.name).length + utf8.encode(v.value).length, 0);
  return bytes > ENV_MAX_BYTES;
}

/** The `env` of `HostInput`: names trimmed, values as typed, blank rows dropped. */
export function envInput(rows: readonly EnvRow[]): HostEnvVar[] {
  return rows.filter((row) => !isBlankEnvRow(row)).map((row) => ({ name: row.name.trim(), value: row.value }));
}
