import type { TabSource } from "@/app/tabs";
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
/** Moves on with each lock, so a load that ends after it keeps nothing. */
let generation = 0;

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
    const gen = generation;
    const load = api
      .command_history_list(owner.key)
      .then((saved) => {
        if (gen !== generation) return;
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

/** Adds a command the user ran at a prompt. */
export function addToHistory(owner: HistoryOwner, command: string) {
  const list = lists.get(owner.key) ?? [];
  const next = withCommand(list, command);
  if (next.length === list.length && next.every((c, i) => c === list[i])) return;
  lists.set(owner.key, next);
  changed();
  if (owner.saved) void api.command_history_add(owner.key, command).catch(() => {});
}

/** Removes a command from the history (Shift+Delete in the list). */
export function removeFromHistory(owner: HistoryOwner, command: string) {
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
  // A load that started before the clear ended would bring the commands back.
  forgetHistory();
}

/** A quick connection's tab closed: its history goes with it. */
export function dropTabHistory(tabId: string) {
  lists.delete(`tab:${tabId}`);
}

/** Drops every copy held here, which the lock does. */
export function forgetHistory() {
  generation++;
  lists.clear();
  loading.clear();
  loaded.clear();
  changed();
}

useApp.subscribe((st, prev) => {
  if (prev.phase === "unlocked" && st.phase !== "unlocked") forgetHistory();
});
