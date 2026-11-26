import { describe, expect, it, vi } from "vitest";
import type { MenuEntry } from "@hatoba/ui";
import { NO_READOUT } from "@/features/dashboard/readings";
import type { GroupView, HostView, TelnetHostView } from "@/ipc/types";

vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));

const { t } = await import("@/i18n");
const {
  buildColumnsMenuEntries,
  buildGroupMenuEntries,
  buildHostMenuEntries,
  buildImportMenuEntries,
  buildMultiHostMenuEntries,
  buildNewMenuEntries,
  buildSortMenuEntries,
  buildTagsMenuEntries,
} = await import("./menus");

function labels(entries: MenuEntry[]): string[] {
  return entries.filter((e): e is Exclude<MenuEntry, { kind: "separator" }> => !("kind" in e)).map((e) => e.label);
}

function makeSshHost(overrides: Partial<HostView> = {}): HostView {
  return {
    id: "h1",
    name: "Prod",
    address: "10.0.0.1",
    port: 22,
    username: "ops",
    auth_kind: "password",
    has_password: true,
    key_id: null,
    group_id: null,
    tags: [],
    favorite: false,
    sort: 0,
    jump_host_id: null,
    proxy_mode: "device_default",
    proxy_id: null,
    note: "",
    ai_notes: "",
    updated_at: 0,
    last_connected_at: null,
    os: null,
    show_stats: false,
    dashboard: true,
    env: [],
    ...overrides,
  };
}

function makeTelnetHost(overrides: Partial<TelnetHostView> = {}): TelnetHostView {
  return {
    id: "t1",
    name: "Switch",
    address: "10.0.0.2",
    port: 23,
    username: "admin",
    has_password: false,
    group_id: null,
    tags: [],
    favorite: true,
    sort: 0,
    proxy_mode: "device_default",
    proxy_id: null,
    note: "",
    ai_notes: "",
    updated_at: 0,
    last_connected_at: null,
    ...overrides,
  };
}

describe("hosts pure menu builders", () => {
  it("builds single host menu with password and stats items only when applicable", () => {
    const baseCtx = {
      t,
      stats: true,
      readouts: { h1: { ...NO_READOUT, phase: "offline" as const } },
      groupMap: new Map<string, GroupView>(),
      onConnect: vi.fn(),
      onEdit: vi.fn(),
      onDuplicate: vi.fn(),
      onFavorite: vi.fn(),
      onCopyAddress: vi.fn(),
      onCopyPassword: vi.fn(),
      onDetails: vi.fn(),
      onRemove: vi.fn(),
    };

    const sshEntries = labels(buildHostMenuEntries(makeSshHost(), baseCtx));
    expect(sshEntries).toContain(t("hosts.menu.copyPassword"));
    expect(sshEntries).toContain(t("hosts.menu.details"));
    expect(sshEntries).toContain(t("btn.retry"));
    expect(sshEntries).toContain(t("dashboard.leaveOut"));

    // Telnet host has no stats section and no password entry when has_password is false.
    const telnetEntries = labels(buildHostMenuEntries(makeTelnetHost(), baseCtx));
    expect(telnetEntries).not.toContain(t("hosts.menu.copyPassword"));
    expect(telnetEntries).not.toContain(t("hosts.menu.details"));
    expect(telnetEntries).toContain(t("hosts.menu.unfavorite"));
  });

  it("builds multi-host and group menus", () => {
    const onFavorite = vi.fn();
    const multi = buildMultiHostMenuEntries([makeSshHost({ favorite: true }), makeTelnetHost({ favorite: true })], {
      t,
      onConnectAll: vi.fn(),
      onFavorite,
      onRemove: vi.fn(),
    });
    expect(labels(multi)).toEqual([t("hosts.menu.connect"), t("hosts.menu.unfavorite"), t("hosts.menu.delete")]);

    const group: GroupView = { id: "g1", name: "Prod", parent_id: null, sort: 0, dashboard: true };
    expect(labels(buildGroupMenuEntries(group, { t, stats: false, onRename: vi.fn(), onDelete: vi.fn() }))).toEqual([
      t("sidebar.renameGroup"),
      t("sidebar.deleteGroup"),
    ]);
    expect(labels(buildGroupMenuEntries(group, { t, stats: true, onRename: vi.fn(), onDelete: vi.fn() }))).toContain(
      t("dashboard.leaveOutGroup"),
    );
  });

  it("builds new, import, sort, tags, and columns menus", () => {
    expect(labels(buildNewMenuEntries(t, vi.fn(), vi.fn()))).toEqual([t("hosts.newHost"), t("sidebar.newGroup")]);
    expect(labels(buildImportMenuEntries(t, vi.fn()))[0]).toBe(t("hosts.import.sshConfig"));

    const sortNoStats = buildSortMenuEntries(t, {
      stats: false,
      sort: "recent",
      shownSort: "recent",
      onSelect: vi.fn(),
    });
    const sortWithStats = buildSortMenuEntries(t, { stats: true, sort: "cpu", shownSort: "cpu", onSelect: vi.fn() });
    expect(sortWithStats.length).toBeGreaterThan(sortNoStats.length);

    const emptyTags = buildTagsMenuEntries(t, { tagList: [], activeTags: [], onChange: vi.fn() });
    expect(labels(emptyTags)).toEqual([t("hosts.tags.none")]);

    const onTags = vi.fn();
    const activeTagsMenu = buildTagsMenuEntries(t, {
      tagList: [{ name: "prod", count: 2 }],
      activeTags: ["prod"],
      onChange: onTags,
    });
    expect(labels(activeTagsMenu)).toContain(t("hosts.tags.clear"));

    const onCols = vi.fn();
    const cols = buildColumnsMenuEntries(t, { hiddenColumns: ["uptime"], onChange: onCols });
    const uptimeItem = cols.find((e) => !("kind" in e) && e.label === t("dashboard.col.uptime"));
    expect(uptimeItem && !("kind" in uptimeItem) && uptimeItem.checked).toBe(false);
    if (uptimeItem && !("kind" in uptimeItem)) uptimeItem.onSelect();
    expect(onCols).toHaveBeenCalledWith([]);
  });
});
