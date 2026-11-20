import { byteCount, HOST_NOTES_MAX_BYTES } from "@/lib/ai/instructions";
import { ENV_MAX_VARS, envRows, envTooLarge, validateEnvRows } from "@/lib/envVars";
import type { Platform } from "@/lib/platform";
import type { HatobaApi } from "../api";
import type { GroupView, HostView } from "../types";
import * as D from "./data";
import { delay, fail, type MockDeps } from "./shared";

/** The room between neighbouring places in the custom order, as `PLACE_STEP` in Rust (HOST-16). */
const PLACE_STEP = 1 << 16;

type HostsApi = Pick<
  HatobaApi,
  | "hosts_list"
  | "host_get"
  | "host_save"
  | "host_delete"
  | "host_duplicate"
  | "hosts_arrange"
  | "host_set_favorite"
  | "host_set_show_stats"
  | "dashboard_set_shown"
  | "host_copy_password"
  | "groups_list"
  | "group_save"
  | "group_delete"
  | "tags_list"
  | "hosts_probe"
  | "hosts_import_preview"
  | "hosts_import"
>;

export interface HostsMockDeps extends Pick<MockDeps, "state" | "needUnlocked" | "id" | "touch"> {
  platform: Platform;
  onHostDeleted(hostId: string): void;
}

export function createHostsMock(ctx: HostsMockDeps): HostsApi & { placeNewHost(after?: string): number } {
  const { state, needUnlocked, id, touch, platform, onHostDeleted } = ctx;

  /** Numbers the hosts and Telnet hosts in `order` again, in that order (HOST-16). */
  function arrangeHosts(order: string[]) {
    const place = new Map(order.map((hid, i) => [hid, (i + 1) * PLACE_STEP]));
    state.hosts = state.hosts.map((h) => (place.has(h.id) ? { ...h, sort: place.get(h.id)! } : h));
    state.telnetHosts = state.telnetHosts.map((h) => (place.has(h.id) ? { ...h, sort: place.get(h.id)! } : h));
  }

  /** The place of a host being added, as `Vault::place_new_host` gives it: after `after`, or else last. */
  function placeNewHost(after?: string): number {
    const all = [...state.hosts, ...state.telnetHosts];
    if (after && all.find((h) => h.id === after)?.sort === 0) return 0;
    const placed = all.filter((h) => h.sort > 0).sort((a, b) => a.sort - b.sort);
    if (placed.length === 0) return 0;
    const i = after ? placed.findIndex((h) => h.id === after) : -1;
    if (i < 0 || i === placed.length - 1) return placed[placed.length - 1].sort + PLACE_STEP;
    if (placed[i + 1].sort - placed[i].sort >= 2) return Math.floor((placed[i].sort + placed[i + 1].sort) / 2);
    arrangeHosts(placed.map((h) => h.id));
    return placeNewHost(after);
  }

  return {
    placeNewHost,
    hosts_list: async () => {
      needUnlocked();
      return state.hosts.map((h) => ({ ...h }));
    },
    host_get: async (hid) => {
      const h = state.hosts.find((x) => x.id === hid);
      if (!h) fail("not_found");
      return { ...h };
    },
    host_save: async (input) => {
      needUnlocked();
      if (!input.address.trim()) fail("invalid_input", "address is required", { field: "address" });
      if (byteCount(input.ai_notes) > HOST_NOTES_MAX_BYTES)
        fail("invalid_input", "AI notes are limited to 16 KB", { field: "ai_notes" });
      if (input.proxy_mode === "proxy" && !state.proxies.some((p) => p.id === input.proxy_id))
        fail("invalid_input", "proxy not found", { field: "proxy_id" });
      const env = input.env.map((v) => ({ name: v.name.trim(), value: v.value }));
      if (
        env.length > ENV_MAX_VARS ||
        validateEnvRows(envRows(env)).some(Boolean) ||
        envTooLarge(envRows(env)) ||
        env.some((v) => !v.name)
      )
        fail("invalid_input", "invalid environment variables", { field: "env" });
      const existing = input.id ? state.hosts.find((h) => h.id === input.id) : undefined;
      const view: HostView = {
        id: existing?.id ?? id("h"),
        name: input.name.trim() || Array.from(input.address.trim()).slice(0, 128).join(""),
        address: input.address.trim(),
        port: input.port || 22,
        username: input.username.trim(),
        auth_kind: input.auth_kind,
        has_password:
          input.auth_kind === "password" &&
          (input.password !== null ? input.password.length > 0 : !!existing?.has_password),
        key_id: input.auth_kind === "key" ? input.key_id : null,
        group_id: input.group_id,
        tags: input.tags,
        favorite: input.favorite,
        sort: existing?.sort ?? placeNewHost(),
        jump_host_id: input.jump_host_id,
        proxy_mode: input.proxy_mode,
        proxy_id: input.proxy_mode === "proxy" ? input.proxy_id : null,
        note: input.note,
        ai_notes: input.ai_notes,
        env,
        updated_at: Date.now(),
        last_connected_at: existing?.last_connected_at ?? null,
        os: existing?.os ?? null,
        show_stats: existing?.show_stats ?? false,
        dashboard: existing?.dashboard ?? true,
      };
      state.hosts = existing ? state.hosts.map((h) => (h.id === view.id ? view : h)) : [...state.hosts, view];
      touch();
      return view;
    },
    host_delete: async (hid) => {
      state.hosts = state.hosts
        .filter((h) => h.id !== hid)
        .map((h) => (h.jump_host_id === hid ? { ...h, jump_host_id: null } : h));
      onHostDeleted(hid);
      touch();
    },
    host_duplicate: async (hid) => {
      const h = state.hosts.find((x) => x.id === hid);
      if (!h) fail("not_found");
      const copy = {
        ...h,
        id: id("h"),
        name: `${h.name}-copy`,
        favorite: false,
        sort: placeNewHost(hid),
        last_connected_at: null,
        os: null,
        show_stats: false,
      };
      state.hosts = [...state.hosts, copy];
      touch();
      return copy;
    },
    hosts_arrange: async (order) => {
      needUnlocked();
      arrangeHosts(
        order.filter((hid) => state.hosts.some((h) => h.id === hid) || state.telnetHosts.some((h) => h.id === hid)),
      );
      touch();
    },
    host_set_favorite: async (hid, favorite) => {
      state.hosts = state.hosts.map((h) => (h.id === hid ? { ...h, favorite } : h));
      touch();
    },
    host_set_show_stats: async (hid, on) => {
      needUnlocked();
      state.hosts = state.hosts.map((h) => (h.id === hid ? { ...h, show_stats: on } : h));
    },
    dashboard_set_shown: async (itemId, shown) => {
      needUnlocked();
      if (!state.hosts.some((h) => h.id === itemId) && !state.groups.some((g) => g.id === itemId))
        fail("not_found", "host or group");
      state.hosts = state.hosts.map((h) => (h.id === itemId ? { ...h, dashboard: shown } : h));
      state.groups = state.groups.map((g) => (g.id === itemId ? { ...g, dashboard: shown } : g));
    },
    host_copy_password: async () => {},
    groups_list: async () => state.groups.map((g) => ({ ...g })),
    group_save: async (input) => {
      const g: GroupView = {
        id: input.id ?? id("g"),
        name: input.name,
        parent_id: input.parent_id,
        sort: input.sort,
        dashboard: state.groups.find((x) => x.id === input.id)?.dashboard ?? true,
      };
      state.groups = input.id ? state.groups.map((x) => (x.id === g.id ? g : x)) : [...state.groups, g];
      touch();
      return g;
    },
    group_delete: async (gid) => {
      state.groups = state.groups.filter((g) => g.id !== gid);
      state.hosts = state.hosts.map((h) => (h.group_id === gid ? { ...h, group_id: null } : h));
      state.telnetHosts = state.telnetHosts.map((h) => (h.group_id === gid ? { ...h, group_id: null } : h));
      touch();
    },
    tags_list: async () => {
      const counts = new Map<string, number>();
      [...state.hosts, ...state.telnetHosts].forEach((h) =>
        h.tags.forEach((t) => counts.set(t, (counts.get(t) ?? 0) + 1)),
      );
      return [...counts.entries()]
        .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
        .map(([name, count]) => ({ name, count }));
    },
    hosts_probe: async (ids) => {
      await delay(250);
      return ids.map((hid) => {
        const online = D.PROBE[hid] ?? true;
        return { id: hid, online, latency_ms: online ? 20 + ((hid.length * 7) % 60) : null };
      });
    },
    hosts_import_preview: async (source, path) => {
      await delay(200);
      const candidates = D.importCandidates(source).map((c) => ({
        ...c,
        exists: state.hosts.some((h) => h.name.toLowerCase() === c.name.toLowerCase()),
      }));
      return { location: path ?? D.importLocation(source, platform), encrypted: false, candidates };
    },
    hosts_import: async (source, _path, ids, keyFiles) => {
      await delay(300);
      const chosen = D.importCandidates(source).filter((c) => ids.includes(c.id));
      for (const c of chosen) {
        // A folder becomes a group named after its path.
        let groupId: string | null = null;
        if (c.folder) {
          const found = state.groups.find((g) => !g.parent_id && g.name.toLowerCase() === c.folder?.toLowerCase());
          const g: GroupView = found ?? {
            id: id("g"),
            name: c.folder,
            parent_id: null,
            sort: state.groups.length,
            dashboard: true,
          };
          if (!found) state.groups.push(g);
          groupId = g.id;
        }
        state.hosts.push({
          ...D.HOSTS[0],
          id: id("h"),
          name: c.name,
          address: c.address,
          port: c.port,
          username: c.username,
          favorite: false,
          sort: placeNewHost(),
          tags: [],
          group_id: groupId,
          last_connected_at: null,
          os: null,
          key_id: null,
          auth_kind: "ask",
          proxy_mode: "device_default",
          proxy_id: null,
        });
      }
      touch();
      const warnings = chosen.flatMap((c) =>
        c.skipped_proxy ? [`${c.name}: ${c.skipped_proxy} was not imported; set a proxy or jump host for it`] : [],
      );
      return { hosts_created: chosen.length, keys_imported: keyFiles.length, warnings };
    },
  };
}
