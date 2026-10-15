/**
 * Mock of the AI memory commands (AI-53), checked like Rust. `?memory=demo` starts with sample
 * memories; `?memory=full` with memories that leave no room.
 */

import { cleanMemoryContent, cleanMemoryName, MEMORY_MAX_BYTES, memoryProblem } from "@/features/ai/memory";
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

  function save(id: string | null, name: string, content: string): AiMemoryView {
    const problem = memoryProblem(memories, id, name, content);
    if (problem) throw { code: "invalid_input", detail: DETAIL[problem], field: "memory" } satisfies AppError;
    const saved: AiMemoryView = { id: id ?? `mem-new-${++seq}`, name: cleanMemoryName(name), content: cleanMemoryContent(content), updated_at: Date.now() };
    memories = id ? memories.map((m) => (m.id === id ? saved : m)) : [...memories, saved];
    return structuredClone(saved);
  }

  return {
    ai_memories_list: async () => structuredClone(memories),
    ai_memory_save: async (input) => {
      if (input.id && !memories.some((m) => m.id === input.id)) throw { code: "not_found", detail: "memory not found" } satisfies AppError;
      return save(input.id, input.name, input.content);
    },
    ai_memory_delete: async (id) => {
      if (!memories.some((m) => m.id === id)) throw { code: "not_found", detail: "memory not found" } satisfies AppError;
      memories = memories.filter((m) => m.id !== id);
    },
    ai_memories_delete_all: async () => {
      memories = [];
    },
    memory_tool: (action, name, content) => {
      const found = memories.find((m) => m.name === cleanMemoryName(name));
      if (action === "delete") {
        if (!found) return { status: "error", content: `There is no memory named "${cleanMemoryName(name)}".` };
        memories = memories.filter((m) => m !== found);
        return { status: "ok", content: `Deleted the memory "${found.name}".` };
      }
      try {
        const saved = save(found?.id ?? null, name, content);
        return { status: "ok", content: `Saved the memory "${saved.name}".` };
      } catch (e) {
        return { status: "error", content: (e as AppError).detail };
      }
    },
  };
}
