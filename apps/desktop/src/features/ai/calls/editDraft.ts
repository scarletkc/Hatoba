import { parseSendKey, readTerminalInput, showTerminalInput } from "../terminal/sendKeys";
import { parseArgs, prettyArgs, type ToolKind } from "../tools";

/** AI-13: the keys a `send_input` call presses after its text, in order: `key`, when given, then `keys`. */
export function callKeys(args: Record<string, unknown>): string[] {
  const keys = [...(args.key ? [args.key] : []), ...(Array.isArray(args.keys) ? args.keys : [])];
  return keys.map((key) => (typeof key === "string" ? key : key === undefined || key === null ? "" : String(key)));
}

/** The fields the Edit form shows: the main input of each tool, or the JSON for others. */
export interface Draft {
  main: string;
  /** A `run_command` call's directory; empty for the home directory. */
  dir: string;
  timeout: string;
  keys: string[];
  wait: string;
}

export function draftFrom(kind: ToolKind, args: Record<string, unknown>, json: string): Draft {
  const str = (v: unknown) => (typeof v === "string" ? v : v === undefined || v === null ? "" : String(v));
  const none = { dir: "", timeout: "", keys: [] as string[], wait: "" };
  switch (kind) {
    case "run_command":
      return { ...none, main: str(args.command), dir: str(args.workdir), timeout: str(args.timeout_seconds) };
    case "send_input":
      return { ...none, main: showTerminalInput(str(args.text)), keys: callKeys(args), wait: str(args.wait_seconds) };
    case "fetch_url":
      return { ...none, main: str(args.url) };
    // AI-39, AI-40: Edit changes the text the call writes; the diff follows.
    case "edit_file":
      return { ...none, main: str(args.new_string) };
    case "write_file":
      return { ...none, main: str(args.content) };
    default:
      return { ...none, main: prettyArgs(json) };
  }
}

/**
 * The arguments after Edit, or an error for invalid JSON. Numbers left empty are dropped, and so is a directory
 * left empty that the call did not give (an empty one is the home directory too).
 */
export function argsFrom(kind: ToolKind, base: Record<string, unknown>, d: Draft): { json: string } | { error: true } {
  const num = (v: string) => (v.trim() === "" ? undefined : Number(v));
  const without = (o: Record<string, unknown>) =>
    Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined));
  switch (kind) {
    case "run_command": {
      const workdir = d.dir === "" && typeof base.workdir !== "string" ? undefined : d.dir;
      return { json: JSON.stringify(without({ ...base, command: d.main, workdir, timeout_seconds: num(d.timeout) })) };
    }
    case "send_input": {
      const keys = d.keys;
      if (keys.some((key) => parseSendKey(key) === null)) return { error: true };
      return {
        json: JSON.stringify(
          without({
            ...base,
            text: readTerminalInput(d.main, typeof base.text === "string" ? base.text : ""),
            key: undefined,
            keys: keys.length ? keys : undefined,
            wait_seconds: num(d.wait),
          }),
        ),
      };
    }
    case "fetch_url":
      return { json: JSON.stringify({ ...base, url: d.main.trim() }) };
    case "edit_file":
      return { json: JSON.stringify({ ...base, new_string: d.main }) };
    case "write_file":
      return { json: JSON.stringify({ ...base, content: d.main }) };
    default: {
      const parsed = parseArgs(d.main);
      return parsed ? { json: JSON.stringify(parsed) } : { error: true };
    }
  }
}
