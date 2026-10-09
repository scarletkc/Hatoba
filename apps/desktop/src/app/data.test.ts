import { describe, expect, it, vi } from "vitest";
import type { HostView } from "@/ipc/types";

const backend = vi.hoisted(() => ({ pending: [] as ((hosts: HostView[]) => void)[] }));

vi.mock("@/ipc/api", async (importActual) => ({
  ...(await importActual<typeof import("@/ipc/api")>()),
  api: {
    hosts_list: () => new Promise<HostView[]>((resolve) => backend.pending.push(resolve)),
    groups_list: async () => [],
    tags_list: async () => [],
    keys_list: async () => [],
  },
}));

const { useVaultData } = await import("./data");

const host = (id: string) => ({ id }) as HostView;

describe("reloadHosts", () => {
  it("applies the hosts it reads", async () => {
    const done = useVaultData.getState().reloadHosts();
    backend.pending.shift()!([host("a")]);
    await done;
    expect(useVaultData.getState().hosts.map((h) => h.id)).toEqual(["a"]);
  });

  it("drops a result that arrives after the vault data was cleared", async () => {
    const done = useVaultData.getState().reloadHosts();
    useVaultData.getState().clear();
    backend.pending.shift()!([host("a")]);
    await done;
    expect(useVaultData.getState().hosts).toEqual([]);
  });

  it("drops a result that a full reload started after it supersedes", async () => {
    const stale = useVaultData.getState().reloadHosts();
    const full = useVaultData.getState().reload();
    const [first, second] = backend.pending.splice(0);
    second([host("fresh")]);
    await full;
    first([host("stale")]);
    await stale;
    expect(useVaultData.getState().hosts.map((h) => h.id)).toEqual(["fresh"]);
  });
});
