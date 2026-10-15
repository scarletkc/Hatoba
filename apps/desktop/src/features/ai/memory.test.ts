import { describe, expect, it } from "vitest";
import type { AiMemoryView } from "@/ipc/types";
import { cleanMemoryContent, cleanMemoryName, editedIds, MEMORY_MAX_BYTES, MEMORY_NAME_MAX_CHARS, memoriesSize, memoryProblem } from "./memory";

const memory = (id: string, name: string, content: string): AiMemoryView => ({ id, name, content, updated_at: 0 });

describe("memories (AI-53)", () => {
  it("cleans names and texts as Rust stores them", () => {
    expect(cleanMemoryName("  web-1\nsudo\t")).toBe("web-1 sudo");
    expect(cleanMemoryContent("  a\r\nb\rc  ")).toBe("a\nb\nc");
  });

  it("refuses what Rust refuses", () => {
    const saved = [memory("m1", "web-1 sudo", "Uses doas.")];
    expect(memoryProblem(saved, [], " ", "x")).toBe("empty_name");
    expect(memoryProblem(saved, [], "n".repeat(MEMORY_NAME_MAX_CHARS + 1), "x")).toBe("long_name");
    expect(memoryProblem(saved, [], "n", " \n ")).toBe("empty_content");
    expect(memoryProblem(saved, [], " web-1 sudo ", "x")).toBe("taken");
    // Saving a memory under its own name is no clash.
    expect(memoryProblem(saved, ["m1"], "web-1 sudo", "Uses doas, not sudo.")).toBeNull();
  });

  it("counts every memory's name and text against one limit", () => {
    const half = "x".repeat(MEMORY_MAX_BYTES / 2 - 1);
    const saved = [memory("a", "a", half), memory("b", "b", half)];
    expect(memoriesSize(saved)).toBe(MEMORY_MAX_BYTES);
    expect(memoryProblem(saved, [], "c", "x")).toBe("full");
    // Replacing one counts its new size, not both.
    expect(memoryProblem(saved, ["a"], "a", "short")).toBeNull();
    // Bytes of UTF-8, as Rust counts them.
    expect(memoriesSize([memory("z", "记忆", "é")])).toBe(8);
  });

  it("edits memories that devices saved under one name as one, as Rust does", () => {
    const saved = [memory("laptop", "web-1 sudo", "a"), memory("desktop", "web-1 sudo", "b"), memory("x", "other", "c")];
    expect(editedIds(saved, null, "web-1 sudo")).toEqual([]);
    expect(editedIds(saved, "desktop", " web-1 sudo ")).toEqual(["desktop", "laptop"]);
    expect(memoryProblem(saved, editedIds(saved, "desktop", "web-1 sudo"), "web-1 sudo", "merged")).toBeNull();
    // A rename leaves the others alone, and one into another memory's name is refused.
    expect(editedIds(saved, "desktop", "web-1 doas")).toEqual(["desktop"]);
    expect(memoryProblem(saved, editedIds(saved, "x", "web-1 sudo"), "web-1 sudo", "c")).toBe("taken");
  });
});
