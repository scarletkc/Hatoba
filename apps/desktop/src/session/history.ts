import type { TabSource } from "@/app/tabs";
import { useVaultData } from "@/app/data";
import { useApp } from "@/app/store";
import { api } from "@/ipc/api";
import { withCommand } from "./suggest";

/**
 * The command history of TERM-15, per saved host (SSH or Telnet) and per local shell. The backend keeps
 * it sealed on this device; a copy is held here while the vault is unlocked, so suggestions never wait
 * for it. A quick connection (HOST-12) keeps its commands for the tab only.
 */
export interface HistoryOwner {
  key: string;
  /** Saved in the backend; false for a quick connection, whose history lives in the tab. */
  saved: boolean;
}

/** What a tab's server offers suggestions (TERM-16), kept in memory for the tab. */
export interface ServerLists {
  /** The shell's own history, newest first. */
  history: readonly string[];
  /** The names of the commands on its `PATH`, sorted. */
  commands: readonly string[];
}

/** Whose history a tab reads and adds to. */
export function historyOwner(tabId: string, source: TabSource): HistoryOwner {
  if (source.local) return { key: `local:${source.local.id}`, saved: true };
  const hostId = source.hostId ?? source.telnet?.hostId ?? null;
  return hostId !== null ? { key: hostId, saved: true } : { key: `tab:${tabId}`, saved: false };
}

const lists = new Map<string, string[]>();
const loading = new Map<string, Promise<void>>();
/** Saved histories read from the backend since the unlock. */
const loaded = new Set<string>();
const listeners = new Set<() => void>();
/** Returned for a history with nothing in it, the same each time. */
const EMPTY: readonly string[] = [];
/** By tab. */
const serverLists = new Map<string, ServerLists>();
const NO_SERVER_LISTS: ServerLists = { history: EMPTY, commands: EMPTY };
/** The list each tab last showed, the same until what it is made of changes. */
const merged = new Map<string, { own: readonly string[]; server: readonly string[]; all: readonly string[] }>();

function changed() {
  listeners.forEach((listener) => listener());
}

/** Calls `listener` when any history changes. Returns the unsubscribe function. */
export function onHistoryChange(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** The owner's commands, newest first, as far as they are loaded; starts loading them. */
export function historyOf(owner: HistoryOwner): readonly string[] {
  const list = lists.get(owner.key) ?? EMPTY;
  if (owner.saved && !loaded.has(owner.key) && !loading.has(owner.key)) {
    const load: Promise<void> = api
      .command_history_list(owner.key)
      .then((saved) => {
        // The lock, Clear Command History or the host's deletion dropped this load.
        if (loading.get(owner.key) !== load) return;
        loaded.add(owner.key);
        // A command added while loading stays on top.
        const added = lists.get(owner.key) ?? [];
        lists.set(
          owner.key,
          added.reduceRight((acc, c) => withCommand(acc, c), saved),
        );
        changed();
      })
      .catch(() => {
        /* suggestions are best effort */
      })
      .finally(() => {
        if (loading.get(owner.key) === load) loading.delete(owner.key);
      });
    loading.set(owner.key, load);
  }
  return list;
}

/** What the tab's server offers, as far as it answered. */
export function serverListsOf(tabId: string): ServerLists {
  return serverLists.get(tabId) ?? NO_SERVER_LISTS;
}

/** Keeps what the tab's server answered. */
export function setServerLists(tabId: string, lists: ServerLists) {
  serverLists.set(tabId, lists);
  changed();
}

/** The tab closed: what its server answered goes with it. */
export function dropServerLists(tabId: string) {
  serverLists.delete(tabId);
  merged.delete(tabId);
}

/** What the list of commands shows (TERM-14): the owner's commands, then the server's other ones. */
export function listedHistory(tabId: string, owner: HistoryOwner): readonly string[] {
  const own = historyOf(owner);
  const server = serverListsOf(tabId).history;
  if (server.length === 0) return own;
  const last = merged.get(tabId);
  if (last?.own === own && last.server === server) return last.all;
  const mine = new Set(own);
  const all = [...own, ...server.filter((c) => !mine.has(c))];
  merged.set(tabId, { own, server, all });
  return all;
}

/** Adds a command the user ran at a prompt. */
export function addToHistory(owner: HistoryOwner, command: string) {
  const list = lists.get(owner.key) ?? [];
  const next = withCommand(list, command);
  if (next.length === list.length && next.every((c, i) => c === list[i])) return;
  lists.set(owner.key, next);
  changed();
  if (owner.saved) void api.command_history_add(owner.key, command).catch(() => {});
}

/**
 * Removes a command from the history (Shift+Delete in the list), and from the copy the tab holds of its
 * server's history: the file on the server stays as it is.
 */
export function removeFromHistory(tabId: string, owner: HistoryOwner, command: string) {
  const server = serverLists.get(tabId);
  if (server) serverLists.set(tabId, { ...server, history: server.history.filter((c) => c !== command) });
  const list = lists.get(owner.key);
  if (list)
    lists.set(
      owner.key,
      list.filter((c) => c !== command),
    );
  changed();
  if (owner.saved) void api.command_history_remove(owner.key, command).catch(() => {});
}

/** Settings → Terminal → Clear Command History: every history on this device, the tabs' own included. */
export async function clearAllHistory() {
  await api.command_history_clear();
  // The copies of the servers' histories too; each server's file stays as it is.
  for (const [tabId, server] of serverLists) serverLists.set(tabId, { ...server, history: EMPTY });
  // A load that started before the clear ended would bring the commands back.
  forgetHistory();
}

/** A quick connection's tab closed: its history goes with it. */
export function dropTabHistory(tabId: string) {
  lists.delete(`tab:${tabId}`);
}

/** Drops the copy of one owner's history, and a load of it still running. */
function forgetOwner(key: string) {
  const had = lists.delete(key);
  loading.delete(key);
  loaded.delete(key);
  if (had) changed();
}

/** Drops every copy held here, which the lock does. */
export function forgetHistory() {
  lists.clear();
  loading.clear();
  loaded.clear();
  changed();
}

useApp.subscribe((st, prev) => {
  if (prev.phase === "unlocked" && st.phase !== "unlocked") forgetHistory();
});

// A deleted host's history goes with it (TERM-15), also while one of its tabs stays open.
useVaultData.subscribe((st, prev) => {
  if (st.hosts === prev.hosts && st.telnetHosts === prev.telnetHosts) return;
  const kept = new Set([...st.hosts, ...st.telnetHosts].map((h) => h.id));
  for (const host of [...prev.hosts, ...prev.telnetHosts]) if (!kept.has(host.id)) forgetOwner(host.id);
});
