/**
 * Mock of the AI memory commands (AI-53), checked like Rust. `?memory=demo` starts with sample
 * memories; `?memory=full` with memories that leave no room.
 */

import { cleanMemoryContent, cleanMemoryName, editedIds, MEMORY_MAX_BYTES, memoryProblem } from "@/features/ai/memory";
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
  return [
    { id: "mem-1", name: "web-1 sudo", content: "web-1 uses doas, not sudo.", updated_at: now - 5 * 86_400_000 },
    {
      id: "mem-2",
      name: "Deploys",
      content: "The API is deployed with `make deploy` from /srv/api on prod-api-tokyo, and restarted with pm2, not systemd.",
      updated_at: now - 2 * 86_400_000,
    },
    { id: "mem-3", name: "Replies", content: "Answer in Chinese; keep commands and file paths in English.", updated_at: now - 3_600_000 },
  ];
}

export interface AiMemoryMock extends AiMemoryApi {
  /** What the mock's `memory` tool does, as Rust's `memory::run` (AI-53). */
  memory_tool(action: string, name: string, content: string): { status: "ok" | "error"; content: string };
}

export function createAiMemoryMock(): AiMemoryMock {
  const flags = new Set((new URLSearchParams(location.search).get("memory") ?? "").split(",").map((x) => x.trim()));
  let memories: AiMemoryView[] = flags.has("demo") ? demoMemories() : [];
  if (flags.has("full")) memories = [{ id: "mem-big", name: "Runbook", content: "x".repeat(MEMORY_MAX_BYTES - 7), updated_at: Date.now() }];
  let seq = 0;

  /** As Rust's `memory::save`: in place of the memories `replaced`, the first keeping its place and the others deleted. */
  function save(replaced: readonly string[], name: string, content: string): AiMemoryView {
    const problem = memoryProblem(memories, replaced, name, content);
    if (problem) throw { code: "invalid_input", detail: DETAIL[problem], field: "memory" } satisfies AppError;
    const id = replaced[0] ?? `mem-new-${++seq}`;
    const saved: AiMemoryView = { id, name: cleanMemoryName(name), content: cleanMemoryContent(content), updated_at: Date.now() };
    memories = replaced.length
      ? memories.flatMap((m) => (m.id === id ? [saved] : replaced.includes(m.id) ? [] : [m]))
      : [...memories, saved];
    return structuredClone(saved);
  }

  return {
    ai_memories_list: async () => structuredClone(memories),
    ai_memory_save: async (input) => {
      if (input.id && !memories.some((m) => m.id === input.id)) throw { code: "not_found", detail: "memory not found" } satisfies AppError;
      return save(editedIds(memories, input.id, input.name), input.name, input.content);
    },
    ai_memory_delete: async (id) => {
      if (!memories.some((m) => m.id === id)) throw { code: "not_found", detail: "memory not found" } satisfies AppError;
      memories = memories.filter((m) => m.id !== id);
    },
    ai_memories_delete_all: async () => {
      memories = [];
    },
    memory_tool: (action, name, content) => {
      // As Rust's `memory::run`: memories saved under one name before a sync count as one.
      const clean = cleanMemoryName(name);
      const found = memories.filter((m) => m.name === clean).map((m) => m.id);
      if (action === "delete") {
        if (!found.length) return { status: "error", content: `There is no memory named "${clean}".` };
        memories = memories.filter((m) => !found.includes(m.id));
        return { status: "ok", content: `Deleted the memory "${clean}".` };
      }
      try {
        const saved = save(found, name, content);
        return { status: "ok", content: `Saved the memory "${saved.name}".` };
      } catch (e) {
        return { status: "error", content: (e as AppError).detail };
      }
    },
  };
}
