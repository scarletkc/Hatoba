import { describe, expect, it, vi } from "vitest";
import type { HostView } from "@/ipc/types";

const backend = vi.hoisted(() => ({ pending: [] as ((commands: string[]) => void)[] }));

vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    command_history_list: () => new Promise<string[]>((resolve) => backend.pending.push(resolve)),
    command_history_add: async () => null,
    command_history_remove: async () => null,
    command_history_clear: async () => null,
  },
}));
vi.mock("@/app/store", () => ({ useApp: { subscribe: () => () => {} } }));

const { useVaultData } = await import("@/app/data");
const {
  addToHistory,
  clearAllHistory,
  dropServerLists,
  historyOf,
  listedHistory,
  removeFromHistory,
  serverListsOf,
  setServerLists,
} = await import("./history");

const host = (id: string) => ({ id }) as HostView;
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("a deleted host's history (TERM-15)", () => {
  it("is dropped, and so is a load of it still running", async () => {
    useVaultData.setState({ hosts: [host("a"), host("b")] });
    const a = { key: "a", saved: true };
    const b = { key: "b", saved: true };
    historyOf(a);
    backend.pending.shift()!(["ls"]);
    await settle();
    expect(historyOf(a)).toEqual(["ls"]);
    historyOf(b);
    const loadB = backend.pending.shift()!;

    useVaultData.setState({ hosts: [] });
    expect(historyOf(a)).toEqual([]);
    loadB(["pwd"]);
    await settle();
    expect(historyOf(b)).toEqual([]);
  });
});

describe("what a tab's server offers (TERM-16)", () => {
  const owner = { key: "tab:t1", saved: false };
  const commands = ["git", "ls"];

  it("lists the server's other commands after the tab's own, the same list until it changes", () => {
    const own = { key: "tab:t0", saved: false };
    addToHistory(own, "pwd");
    setServerLists("t0", { history: ["htop", "pwd"], commands });
    const first = listedHistory("t0", own);
    expect(first).toEqual(["pwd", "htop"]);
    expect(listedHistory("t0", own)).toBe(first);
    expect(serverListsOf("t2").history).toEqual([]);
  });

  it("removes a command from the tab's copy only, and clearing keeps the command names", async () => {
    setServerLists("t1", { history: ["htop", "pwd"], commands });
    removeFromHistory("t1", owner, "htop");
    expect(listedHistory("t1", owner)).toEqual(["pwd"]);
    await clearAllHistory();
    expect(serverListsOf("t1")).toEqual({ history: [], commands });
    dropServerLists("t1");
    expect(serverListsOf("t1").commands).toEqual([]);
  });
});
