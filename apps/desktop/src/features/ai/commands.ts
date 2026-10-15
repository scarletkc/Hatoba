/**
 * AI-50, AI-51: the `/` and `@` commands of the input box. `/` at the start of the text runs a
 * command: a built-in action, a skill, or an MCP prompt. `@` at the start of a word puts something
 * into the message as context: a file or folder on the tab's host, an MCP resource, or text files
 * from this computer. This module holds what can be decided from text alone; the menu is
 * `CommandMenu.tsx`, and running a command is in `Composer.tsx`.
 */

import type { MessageKey } from "@/i18n";
import type { AiHostEntry, AiSkillCommand, McpPromptView, McpResourceView } from "@/ipc/types";

/** The built-in commands, named as Claude Code names the same features. */
export type BuiltinName = "btw" | "clear" | "compact" | "mcp" | "memory" | "model";

export interface BuiltinCommand {
  name: BuiltinName;
  /** Other names that run it, which the menu matches too. */
  aliases: readonly string[];
  /** What follows the name, such as `<question>`; null when it takes nothing. */
  args: MessageKey | null;
  /** The command needs what follows its name: Enter in the menu completes it rather than running it. */
  argsRequired: boolean;
  description: MessageKey;
}

export const BUILTINS: readonly BuiltinCommand[] = [
  { name: "btw", aliases: [], args: "ai.cmd.btw.args", argsRequired: true, description: "ai.cmd.btw" },
  { name: "clear", aliases: ["new", "reset"], args: null, argsRequired: false, description: "ai.cmd.clear" },
  { name: "compact", aliases: [], args: "ai.cmd.compact.args", argsRequired: false, description: "ai.cmd.compact" },
  { name: "mcp", aliases: [], args: null, argsRequired: false, description: "ai.cmd.mcp" },
  { name: "memory", aliases: [], args: null, argsRequired: false, description: "ai.cmd.memory" },
  { name: "model", aliases: [], args: null, argsRequired: false, description: "ai.cmd.model" },
];

/** The built-in command named `name`, or by one of its aliases. */
export function findBuiltin(name: string): BuiltinCommand | undefined {
  return BUILTINS.find((c) => c.name === name || c.aliases.includes(name));
}

/** What opens the menu: the word at the caret that starts with `/` or `@`, and what follows the sign up to the caret. */
export interface Trigger {
  sign: "/" | "@";
  /** Where the sign is in the text. */
  start: number;
  /** The caret. */
  end: number;
  /** The text between the sign and the caret; for `@"…`, without the opening quote. */
  query: string;
  /** An `@` mention written in quotes, which may hold spaces. */
  quoted: boolean;
}

/**
 * The trigger at the caret, if any: `/` at the very start of the text followed by no space before the
 * caret, or `@` at the start of the text or after a space, followed by no space (or, after `@"`, no
 * quote) before the caret. A selection, as a caret that is not collapsed, opens nothing. A command's
 * query is its whole word, wherever the caret is in it, so that Enter runs the command the word names.
 */
export function triggerAt(text: string, caret: number, selectionEnd: number = caret): Trigger | null {
  if (caret !== selectionEnd) return null;
  const before = text.slice(0, caret);
  if (before.startsWith("/") && !/\s/.test(before)) {
    const word = /^\/\S*/.exec(text)![0];
    return { sign: "/", start: 0, end: word.length, query: word.slice(1), quoted: false };
  }
  const quoted = /(?:^|\s)@"([^"]*)$/.exec(before);
  if (quoted) return { sign: "@", start: caret - quoted[1].length - 2, end: caret, query: quoted[1], quoted: true };
  const plain = /(?:^|\s)@([^\s"]*)$/.exec(before);
  if (plain) return { sign: "@", start: caret - plain[1].length - 1, end: caret, query: plain[1], quoted: false };
  return null;
}

/** The command the text starts with: `/name` followed by a space, a line break or the end, and what follows it. */
export function leadingCommand(text: string): { name: string; args: string } | null {
  const m = /^\/([^\s/]+)(?:\s+([\s\S]*))?$/.exec(text.trim());
  return m ? { name: m[1], args: (m[2] ?? "").trim() } : null;
}

/**
 * The + menu's **Slash Commands** (`name` null) and a skill of its **Skills** (AI-50): the box's text with `/` or
 * `/name ` at its start, in place of a command it starts with, what was typed after it, and the caret. After `/` alone
 * the caret stays on the command's word, so the menu opens on it.
 */
export function commandDraft(draft: string, name: string | null): { text: string; caret: number } {
  const rest = (leadingCommand(draft)?.args ?? draft).trimStart();
  if (name === null) return { text: rest ? `/ ${rest}` : "/", caret: 1 };
  const head = `/${name} `;
  return { text: head + rest, caret: head.length };
}

/**
 * The words of a prompt command's arguments (AI-50), filling the prompt's arguments in order: words are
 * separated by spaces, a word in double quotes may hold spaces, and the last argument takes the rest of
 * the text as it was written.
 */
export function promptArguments(args: string, names: readonly string[]): { name: string; value: string }[] {
  const out: { name: string; value: string }[] = [];
  let rest = args.trim();
  for (let i = 0; i < names.length && rest; i++) {
    if (i === names.length - 1) {
      const whole = /^"([^"]*)"$/.exec(rest);
      out.push({ name: names[i], value: whole ? whole[1] : rest });
      break;
    }
    const m = /^"([^"]*)"\s*|^(\S+)\s*/.exec(rest)!;
    out.push({ name: names[i], value: m[1] ?? m[2] });
    rest = rest.slice(m[0].length);
  }
  return out;
}

/**
 * The text that replaces a trigger when a mention is picked: `@` and the path or `server:uri`, in
 * quotes when it has a space, then a space.
 */
export function mentionText(target: string): string {
  return /[\s"]/.test(target) ? `@"${target.replace(/"/g, "")}" ` : `@${target} `;
}

/** `text` with the trigger's span (the sign up to the caret, and the rest of its word) replaced by `insert`, and the caret after it. */
export function replaceTrigger(text: string, trigger: Trigger, insert: string): { text: string; caret: number } {
  const after = text.slice(trigger.end);
  // A quoted mention runs to its closing quote when no space comes before it, an unclosed one to the next space.
  const closed = trigger.quoted ? /^[^"\s]*"/.exec(after) : null;
  const word = closed ? closed[0] : /^\S*/.exec(after)![0];
  let rest = after.slice(word.length);
  if (insert.endsWith(" ") && rest.startsWith(" ")) rest = rest.slice(1);
  return { text: text.slice(0, trigger.start) + insert + rest, caret: trigger.start + insert.length };
}

// ───────────── the `/` menu ─────────────

/** A prompt of a running MCP server that the conversation uses. */
export interface PromptCommand {
  serverId: string;
  server: string;
  prompt: McpPromptView;
  /** Its name after `/`: `mcp__<server>__<prompt>`, named as `commandNames` names it. */
  command: string;
}

/** The longest MCP tool name both protocols accept, which prompt commands keep to as well (AI-30). */
const MAX_NAME = 64;

/** A part of an MCP name with every character but ASCII letters, digits, `_` and `-` as `_`. */
const cleanName = (part: string) => part.trim().replace(/[^A-Za-z0-9_-]/gu, "_");

/**
 * The `/` commands of the prompts of `servers`, `[server name, prompt names]` in order (AI-50), named as Rust's
 * `tool_names` names MCP tools (AI-30): `mcp__<server>__<prompt>` cleaned, cut to 64 characters, and given a
 * numeric suffix when two collide, the first holder of a name keeping it.
 */
export function commandNames(servers: readonly (readonly [string, readonly string[]])[]): string[][] {
  const bases = servers.map(([server, names]) => names.map((name) => `mcp__${cleanName(server)}__${cleanName(name)}`.slice(0, MAX_NAME)));
  const unsuffixed = new Set(bases.flat());
  const taken = new Set<string>();
  return bases.map((list) =>
    list.map((base) => {
      let name = base;
      for (let n = 2; taken.has(name); n++) {
        const suffix = `_${n}`;
        const candidate = base.slice(0, MAX_NAME - suffix.length) + suffix;
        if (!unsuffixed.has(candidate) && !taken.has(candidate)) name = candidate;
      }
      taken.add(name);
      return name;
    }),
  );
}

export type CommandItem =
  | { kind: "builtin"; name: string; command: BuiltinCommand }
  | { kind: "skill"; name: string; skill: AiSkillCommand }
  | { kind: "prompt"; name: string; prompt: PromptCommand };

/** How well `name` matches `q` (lowercase): 0 for its start, 1 for anywhere else in it, null for not at all. */
function rank(name: string, q: string): number | null {
  const n = name.toLowerCase();
  if (n.startsWith(q)) return 0;
  return n.includes(q) ? 1 : null;
}

/**
 * The `/` menu for `query`: the built-in commands, then the skills, then the MCP prompts, each matching
 * its name (or a built-in's alias) at the start first, then anywhere.
 */
export function commandItems(query: string, skills: readonly AiSkillCommand[], prompts: readonly PromptCommand[]): CommandItem[] {
  const q = query.toLowerCase();
  const items: { item: CommandItem; rank: number; group: number }[] = [];
  for (const command of BUILTINS) {
    const r = Math.min(...[command.name, ...command.aliases].map((n) => rank(n, q) ?? Infinity));
    if (r !== Infinity) items.push({ item: { kind: "builtin", name: command.name, command }, rank: r, group: 0 });
  }
  for (const skill of skills) {
    // A skill named like a built-in command is run by the built-in, so the menu leaves it out.
    if (findBuiltin(skill.name)) continue;
    const r = rank(skill.name, q);
    if (r !== null) items.push({ item: { kind: "skill", name: skill.name, skill }, rank: r, group: 1 });
  }
  for (const prompt of prompts) {
    const r = Math.min(rank(prompt.command, q) ?? Infinity, rank(prompt.prompt.name, q) ?? Infinity);
    if (r !== Infinity) items.push({ item: { kind: "prompt", name: prompt.command, prompt }, rank: r, group: 2 });
  }
  return items.sort((a, b) => a.rank - b.rank || a.group - b.group).map((x) => x.item);
}

/** Whether Enter on a `/` menu item runs it at once, rather than completing its name for what follows. */
export function runsAtOnce(item: CommandItem): boolean {
  if (item.kind === "builtin") return !item.command.argsRequired;
  if (item.kind === "prompt") return !item.prompt.prompt.arguments.some((a) => a.required);
  return true;
}

// ───────────── the `@` menu ─────────────

/**
 * A path typed after `@` split into the directory to list and the start of the name to complete: `etc/ng`
 * lists `etc/`, `/` the root, `~/x` the home directory, and a path without `/` the home directory too,
 * which the host's scripts take relative paths from.
 */
export function splitPath(query: string): { dir: string; leaf: string } {
  const at = query.lastIndexOf("/");
  if (at < 0) return { dir: "", leaf: query };
  return { dir: query.slice(0, at + 1), leaf: query.slice(at + 1) };
}

/** The directory to list for a split path: the home directory for none. */
export function listPath(dir: string): string {
  return dir === "" ? "~" : dir;
}

/** The entries of a listed directory that complete `leaf`: names that start with it first, then those that contain it; hidden ones only for a leaf that starts with `.`. */
export function pathItems(entries: readonly AiHostEntry[], leaf: string): AiHostEntry[] {
  const q = leaf.toLowerCase();
  const shown = entries.filter((e) => leaf.startsWith(".") || !e.name.startsWith("."));
  const starts = shown.filter((e) => e.name.toLowerCase().startsWith(q));
  const contains = q ? shown.filter((e) => !e.name.toLowerCase().startsWith(q) && e.name.toLowerCase().includes(q)) : [];
  return [...starts, ...contains];
}

/** A resource of a running MCP server that the conversation uses. */
export interface ResourceItem {
  serverId: string;
  server: string;
  resource: McpResourceView;
}

/** How an `@` mention names a resource: `server:uri`, as Claude Code writes it. */
export function resourceTarget(r: ResourceItem): string {
  return `${r.server}:${r.resource.uri}`;
}

/** The resources whose server, name, title or URI contains `query`. */
export function resourceItems(resources: readonly ResourceItem[], query: string): ResourceItem[] {
  const q = query.toLowerCase();
  return resources.filter((r) =>
    [resourceTarget(r), r.resource.name, r.resource.title ?? ""].some((s) => s.toLowerCase().includes(q)),
  );
}

/** A directory listing as a `host_directory` block's text: one entry per line, a `/` after each directory. */
export function listingText(entries: readonly AiHostEntry[]): string {
  return entries.map((e) => (e.dir ? `${e.name}/` : e.name)).join("\n");
}
