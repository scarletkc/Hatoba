/**
 * Mock of the AI memory commands (AI-53), checked like Rust. `?memory=demo` starts with sample
 * memories; `?memory=full` with global memories that leave no room.
 */

import {
  cleanMemoryContent,
  cleanMemoryName,
  editedIds,
  MEMORY_MAX_BYTES,
  memoryProblem,
  memoryScope,
  scopeFields,
  type MemoryScope,
} from "@/features/ai/memory";
import type { HatobaApi } from "../api";
import type { AiMemoryView, AppError } from "../types";

export type AiMemoryApi = Pick<HatobaApi, "ai_memories_list" | "ai_memory_save" | "ai_memory_delete" | "ai_memories_delete_all">;

const DETAIL = {
  empty_name: "The memory has no name.",
  long_name: "The name is longer than 100 characters.",
  empty_content: "The memory is empty. To remove a memory, delete it.",
  taken: "Another memory has this name.",
  full: "Memory is full.",
} as const;

function demoMemories(): AiMemoryView[] {
  const now = Date.now();
  const global = scopeFields("global");
  return [
    { id: "mem-1", name: "sudo", content: "Uses doas, not sudo.", ...scopeFields("host:h-staging-web"), updated_at: now - 5 * 86_400_000 },
    {
      id: "mem-2",
      name: "Deploys",
      content: "The API is deployed with `make deploy` from /srv/api, and restarted with pm2, not systemd.",
      ...scopeFields("host:h-api-tokyo"),
      updated_at: now - 2 * 86_400_000,
    },
    { id: "mem-3", name: "Replies", content: "Answer in Chinese; keep commands and file paths in English.", ...global, updated_at: now - 3_600_000 },
    { id: "mem-4", name: "Shell", content: "PowerShell 7 is the default; Windows PowerShell 5.1 is also installed.", ...scopeFields("local:DEMO-PC"), updated_at: now - 86_400_000 },
  ];
}

/** The host or computer of a conversation, as Rust's `memory::Place`: the scope of its host memories and its name. */
export interface MemoryPlace {
  scope: MemoryScope;
  name: string;
}

export interface AiMemoryMock extends AiMemoryApi {
  /** What the mock's `memory` tool does in a conversation on `place`, as Rust's `memory::run` (AI-53). */
  memory_tool(args: { action: string; scope: string; name: string; content: string }, place: MemoryPlace | null): { status: "ok" | "error"; content: string };
}

export function createAiMemoryMock(): AiMemoryMock {
  const flags = new Set((new URLSearchParams(location.search).get("memory") ?? "").split(",").map((x) => x.trim()));
  let memories: AiMemoryView[] = flags.has("demo") ? demoMemories() : [];
  if (flags.has("full"))
    memories = [{ id: "mem-big", name: "Runbook", content: "x".repeat(MEMORY_MAX_BYTES - 7), ...scopeFields("global"), updated_at: Date.now() }];
  let seq = 0;

  /** As Rust's `memory::save`: in place of the memories `replaced`, the first keeping its place and the others deleted. */
  function save(replaced: readonly string[], scope: MemoryScope, name: string, content: string): AiMemoryView {
    const problem = memoryProblem(memories, replaced, scope, name, content);
    if (problem) throw { code: "invalid_input", detail: DETAIL[problem], field: "memory" } satisfies AppError;
    const id = replaced[0] ?? `mem-new-${++seq}`;
    const saved: AiMemoryView = { id, name: cleanMemoryName(name), content: cleanMemoryContent(content), ...scopeFields(scope), updated_at: Date.now() };
    memories = replaced.length
      ? memories.flatMap((m) => (m.id === id ? [saved] : replaced.includes(m.id) ? [] : [m]))
      : [...memories, saved];
    return structuredClone(saved);
  }

  const named = (scope: MemoryScope, name: string) => memories.filter((m) => memoryScope(m) === scope && m.name === name).map((m) => m.id);

  return {
    ai_memories_list: async () => structuredClone(memories),
    ai_memory_save: async (input) => {
      if (input.id && !memories.some((m) => m.id === input.id)) throw { code: "not_found", detail: "memory not found" } satisfies AppError;
      const scope = memoryScope(input);
      return save(editedIds(memories, input.id, scope, input.name), scope, input.name, input.content);
    },
    ai_memory_delete: async (id) => {
      if (!memories.some((m) => m.id === id)) throw { code: "not_found", detail: "memory not found" } satisfies AppError;
      memories = memories.filter((m) => m.id !== id);
    },
    ai_memories_delete_all: async () => {
      memories = [];
    },
    memory_tool: ({ action, scope, name, content }, place) => {
      // As Rust's `memory::run`: memories saved under one name before a sync count as one.
      if (scope !== "global" && scope !== "host") return { status: "error", content: "The arguments are not valid: scope must be global or host." };
      if (scope === "host" && !place)
        return { status: "error", content: "This conversation is not on a saved host or a local terminal, so it has no host memories." };
      const at = scope === "host" ? place : null;
      const [kind, of] = at ? ["host memory", ` of ${at.name}`] : ["global memory", ""];
      const clean = cleanMemoryName(name);
      const found = named(at?.scope ?? "global", clean);
      if (action === "delete") {
        if (!found.length) return { status: "error", content: `There is no ${kind} named "${clean}"${of}.` };
        memories = memories.filter((m) => !found.includes(m.id));
        return { status: "ok", content: `Deleted the ${kind} "${clean}"${of}.` };
      }
      try {
        const saved = save(found, at?.scope ?? "global", name, content);
        return { status: "ok", content: `Saved the ${kind} "${saved.name}"${of}.` };
      } catch (e) {
        return { status: "error", content: (e as AppError).detail };
      }
    },
  };
}
