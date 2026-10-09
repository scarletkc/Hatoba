import { create } from "zustand";
import { api } from "@/ipc/api";
import type { GroupView, HostView, KeyView, ProxyView, TagCount } from "@/ipc/types";

/**
 * Decrypted-vault metadata shown across the app (sidebar counts, host list, key pickers).
 * Contains no secrets — only the DTOs the backend chooses to expose.
 */
interface VaultData {
  loaded: boolean;
  hosts: HostView[];
  groups: GroupView[];
  tags: TagCount[];
  keys: KeyView[];
  /** Saved proxies (SSH-13). */
  proxies: ProxyView[];
  reload(): Promise<void>;
  /** Re-reads only the hosts, for the device-local fields a connection updates (HOST-06, HOST-11). */
  reloadHosts(): Promise<void>;
  clear(): void;
}

/** Bumped by every full reload and clear, so a hosts-only reload already in flight is dropped. */
let generation = 0;

export const useVaultData = create<VaultData>((set) => ({
  loaded: false,
  hosts: [],
  groups: [],
  tags: [],
  keys: [],
  proxies: [],
  reload: async () => {
    generation++;
    const [hosts, groups, tags, keys, proxies] = await Promise.all([
      api.hosts_list(),
      api.groups_list(),
      api.tags_list(),
      api.keys_list(),
      api.proxies_list(),
    ]);
    set({ loaded: true, hosts, groups: [...groups].sort((a, b) => a.sort - b.sort || a.name.localeCompare(b.name)), tags, keys, proxies });
  },
  reloadHosts: async () => {
    const started = generation;
    const hosts = await api.hosts_list();
    if (started === generation) set({ hosts });
  },
  clear: () => {
    generation++;
    set({ loaded: false, hosts: [], groups: [], tags: [], keys: [], proxies: [] });
  },
}));

export function hostById(id: string | null | undefined): HostView | undefined {
  return id ? useVaultData.getState().hosts.find((h) => h.id === id) : undefined;
}
