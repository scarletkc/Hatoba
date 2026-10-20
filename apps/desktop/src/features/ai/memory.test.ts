import { describe, expect, it } from "vitest";
import type { AiMemoryView } from "@/ipc/types";
import {
  cleanMemoryContent,
  cleanMemoryName,
  editedIds,
  MEMORY_MAX_BYTES,
  MEMORY_NAME_MAX_CHARS,
  memoriesSize,
  memoryProblem,
  memoryScope,
  scopeFields,
  type MemoryScope,
} from "./memory";

const memory = (id: string, name: string, content: string, scope: MemoryScope = "global"): AiMemoryView => ({
  id,
  name,
  content,
  ...scopeFields(scope),
  updated_at: 0,
});

describe("memories (AI-53)", () => {
  it("cleans names and texts as Rust stores them", () => {
    expect(cleanMemoryName("  web-1\nsudo\t")).toBe("web-1 sudo");
    expect(cleanMemoryContent("  a\r\nb\rc  ")).toBe("a\nb\nc");
  });

  it("reads a memory's scope from its fields, the host first", () => {
    expect(memoryScope({ host_id: null, local: null })).toBe("global");
    expect(memoryScope({ host_id: "h1", local: null })).toBe("host:h1");
    expect(memoryScope({ host_id: null, local: "DEMO-PC" })).toBe("local:DEMO-PC");
    expect(memoryScope({ host_id: "h1", local: "DEMO-PC" })).toBe("host:h1");
    for (const scope of ["global", "host:h1", "local:DEMO-PC"] as const) expect(memoryScope(scopeFields(scope))).toBe(scope);
  });

  it("refuses what Rust refuses", () => {
    const saved = [memory("m1", "web-1 sudo", "Uses doas.")];
    expect(memoryProblem(saved, [], "global", " ", "x")).toBe("empty_name");
    expect(memoryProblem(saved, [], "global", "n".repeat(MEMORY_NAME_MAX_CHARS + 1), "x")).toBe("long_name");
    expect(memoryProblem(saved, [], "global", "n", " \n ")).toBe("empty_content");
    expect(memoryProblem(saved, [], "global", " web-1 sudo ", "x")).toBe("taken");
    // Saving a memory under its own name is no clash, and another scope may use the name.
    expect(memoryProblem(saved, ["m1"], "global", "web-1 sudo", "Uses doas, not sudo.")).toBeNull();
    expect(memoryProblem(saved, [], "host:h1", "web-1 sudo", "x")).toBeNull();
  });

  it("counts the names and texts of each scope's memories against its own limit", () => {
    const half = "x".repeat(MEMORY_MAX_BYTES / 2 - 1);
    const saved = [memory("a", "a", half), memory("b", "b", half)];
    expect(memoriesSize(saved)).toBe(MEMORY_MAX_BYTES);
    expect(memoryProblem(saved, [], "global", "c", "x")).toBe("full");
    expect(memoryProblem(saved, [], "local:DEMO-PC", "c", "x")).toBeNull();
    // Replacing one counts its new size, not both.
    expect(memoryProblem(saved, ["a"], "global", "a", "short")).toBeNull();
    // A move counts only the scope it goes to.
    expect(memoryProblem([...saved, memory("h", "h", "x", "host:h1")], ["a"], "host:h1", "a", half)).toBeNull();
    // Bytes of UTF-8, as Rust counts them.
    expect(memoriesSize([memory("z", "记忆", "é")])).toBe(8);
  });

  it("edits memories that devices saved under one name as one, as Rust does", () => {
    const saved = [
      memory("laptop", "web-1 sudo", "a"),
      memory("desktop", "web-1 sudo", "b"),
      memory("x", "other", "c"),
      memory("host", "web-1 sudo", "d", "host:h1"),
    ];
    expect(editedIds(saved, null, "global", "web-1 sudo")).toEqual([]);
    expect(editedIds(saved, "desktop", "global", " web-1 sudo ")).toEqual(["desktop", "laptop"]);
    expect(memoryProblem(saved, editedIds(saved, "desktop", "global", "web-1 sudo"), "global", "web-1 sudo", "merged")).toBeNull();
    // A rename or a move leaves the others alone, and one onto another memory's name is refused.
    expect(editedIds(saved, "desktop", "global", "web-1 doas")).toEqual(["desktop"]);
    expect(editedIds(saved, "desktop", "local:DEMO-PC", "web-1 sudo")).toEqual(["desktop"]);
    expect(memoryProblem(saved, editedIds(saved, "x", "global", "web-1 sudo"), "global", "web-1 sudo", "c")).toBe("taken");
    expect(memoryProblem(saved, editedIds(saved, "desktop", "host:h1", "web-1 sudo"), "host:h1", "web-1 sudo", "b")).toBe("taken");
  });
});
