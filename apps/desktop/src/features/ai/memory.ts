/**
 * AI-53: the memories the assistant keeps across conversations. Rust checks a memory the same way
 * when it is saved (`ai/memory.rs` in src-tauri); the limits are `MAX_MEMORY_BYTES` and
 * `MAX_MEMORY_NAME_CHARS` in crates/hatoba-ai/src/tools.rs.
 */

import type { AiMemoryView } from "@/ipc/types";
import { byteCount, charCount } from "./instructions";

/** The most bytes of UTF-8 the memories of one scope may hold together, names and texts. */
export const MEMORY_MAX_BYTES = 16 * 1024;
/** The longest name, in characters. */
export const MEMORY_NAME_MAX_CHARS = 100;

/**
 * Whose memories, as a key: `global` for every conversation's, `host:<id>` for a saved host's, `local:<computer>` for
 * a computer's local terminal's (§7.5). Names are unique, and {@link MEMORY_MAX_BYTES} counts, within each.
 */
export type MemoryScope = "global" | `host:${string}` | `local:${string}`;

type Scoped = Pick<AiMemoryView, "host_id" | "local">;

/** The scope of a memory, as Rust's `Scope::new` reads it: the host when there is one. */
export function memoryScope(m: Scoped): MemoryScope {
  if (m.host_id) return `host:${m.host_id}`;
  return m.local ? `local:${m.local}` : "global";
}

/** The fields a memory of `scope` is saved with. */
export function scopeFields(scope: MemoryScope): Scoped {
  if (scope.startsWith("host:")) return { host_id: scope.slice("host:".length), local: null };
  if (scope.startsWith("local:")) return { host_id: null, local: scope.slice("local:".length) };
  return { host_id: null, local: null };
}

/** A name as Rust stores it: one line, control characters as spaces, trimmed. */
export function cleanMemoryName(name: string): string {
  return name.replace(/\p{Cc}/gu, " ").trim();
}

/** A text as Rust stores it: line breaks as `\n`, trimmed. */
export function cleanMemoryContent(content: string): string {
  return content.replace(/\r\n?/g, "\n").trim();
}

/** The bytes a memory takes of {@link MEMORY_MAX_BYTES}. */
export function memorySize(memory: { name: string; content: string }): number {
  return byteCount(memory.name) + byteCount(memory.content);
}

/** The bytes the memories take together. */
export function memoriesSize(memories: readonly { name: string; content: string }[]): number {
  return memories.reduce((sum, m) => sum + memorySize(m), 0);
}

export type MemoryProblem = "empty_name" | "long_name" | "empty_content" | "taken" | "full";

/**
 * The memories an edit of `id` (null adds one) into `scope` saves in place of, as Rust's `memory::edited`: `id`, and
 * when the edit keeps its name and its scope, the other memories of that name there, which devices saved before they
 * synced and which count as one with it.
 */
export function editedIds(memories: readonly AiMemoryView[], id: string | null, scope: MemoryScope, name: string): string[] {
  if (id === null) return [];
  const clean = cleanMemoryName(name);
  const edited = memories.find((m) => m.id === id);
  const kept = edited?.name === clean && memoryScope(edited) === scope;
  return [id, ...(kept ? memories.filter((m) => m.id !== id && m.name === clean && memoryScope(m) === scope).map((m) => m.id) : [])];
}

/**
 * Why a memory of `scope` cannot be saved in place of the memories `replaced` (none adds one) beside the others in
 * `memories`, as Rust checks it, or null when it can. `name` and `content` are cleaned first.
 */
export function memoryProblem(
  memories: readonly AiMemoryView[],
  replaced: readonly string[],
  scope: MemoryScope,
  name: string,
  content: string,
): MemoryProblem | null {
  const memory = { name: cleanMemoryName(name), content: cleanMemoryContent(content) };
  if (!memory.name) return "empty_name";
  if (charCount(memory.name) > MEMORY_NAME_MAX_CHARS) return "long_name";
  if (!memory.content) return "empty_content";
  const others = memories.filter((m) => !replaced.includes(m.id) && memoryScope(m) === scope);
  if (others.some((m) => m.name === memory.name)) return "taken";
  if (memoriesSize(others) + memorySize(memory) > MEMORY_MAX_BYTES) return "full";
  return null;
}
