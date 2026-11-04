import { describe, expect, it, vi } from "vitest";
import type { HostView } from "@/ipc/types";

const backend = vi.hoisted(() => ({ pending: [] as ((commands: string[]) => void)[] }));

vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    command_history_list: () => new Promise<string[]>((resolve) => backend.pending.push(resolve)),
    command_history_add: async () => null,
  },
}));
vi.mock("@/app/store", () => ({ useApp: { subscribe: () => () => {} } }));

const { useVaultData } = await import("@/app/data");
const { historyOf } = await import("./history");

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
