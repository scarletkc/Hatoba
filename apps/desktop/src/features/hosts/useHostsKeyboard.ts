import type { KeyboardEvent, MutableRefObject, RefObject } from "react";
import { isImeEvent } from "@hatoba/ui";
import type { AnyHostView, HostView, LocalPrefs, QuickTarget, TelnetHostView } from "@/ipc/types";
import { isMac, type Platform } from "@/lib/platform";
import { connectTarget, connectTelnetTarget } from "@/session/connect";
import {
  parseQuickConnect,
  parseTelnet,
  savedHostFor,
  savedTelnetHostFor,
  type TypedTarget,
} from "@/session/quickConnect";
import { TELNET_ROW, TYPED_ROW, type QuickRowItem } from "./HostsRows";
import type { PickKeys } from "./selection";

export interface HostsKeyboardOptions {
  searchRef: RefObject<HTMLInputElement | null>;
  listRef: RefObject<HTMLDivElement | null>;
  cardsRef: RefObject<HTMLDivElement | null>;
  anchorRef: MutableRefObject<string | null>;
  platform: Platform;
  view: LocalPrefs["hosts_view"];
  stats: boolean;
  searching: boolean;
  query: string;
  hosts: HostView[];
  telnetHosts: TelnetHostView[];
  visible: readonly AnyHostView[];
  hostIds: readonly string[];
  rowIds: readonly string[];
  quickRows: readonly QuickRowItem[];
  selected: readonly string[];
  selectedSet: ReadonlySet<string>;
  focusId: string | null;
  several: boolean;
  suggestions: readonly QuickTarget[];
  suggestOpen: boolean;
  suggestActive: number;
  setSuggestAt: (index: number) => void;
  setSuggestDismissed: (dismissed: boolean) => void;
  connectQuick: (target: TypedTarget) => void;
  pickRows: (ids: string[]) => void;
  pickWith: (id: string, keys: PickKeys) => void;
  selectNone: () => void;
  moveTo: (moving: string[], before: string | null) => void;
  setDetail: (id: string | null) => void;
  remove: (list: AnyHostView[]) => void;
  connect: (id: string) => void;
  connectAll: (list: AnyHostView[]) => void;
}

/**
 * Keyboard handler for the hosts page: arrows move the selection, Enter connects (HOST-07),
 * Alt+Up/Down reorders (HOST-16), and typing focuses the search field.
 */
export function useHostsKeyboard(opts: HostsKeyboardOptions): (e: KeyboardEvent) => void {
  const cardsPerRow = () => {
    const el = opts.cardsRef.current;
    if (!el) return 1;
    return Math.max(1, getComputedStyle(el).gridTemplateColumns.split(" ").filter(Boolean).length);
  };

  const move = (delta: number, range: boolean) => {
    const { rowIds, focusId, hostIds, pickWith, listRef } = opts;
    if (rowIds.length === 0) return;
    const i = focusId ? rowIds.indexOf(focusId) : -1;
    const next = rowIds[Math.max(0, Math.min(rowIds.length - 1, i + delta))];
    pickWith(next, { range: range && hostIds.includes(next), add: false });
    requestAnimationFrame(() =>
      listRef.current?.querySelector(`[data-id="${CSS.escape(next)}"]`)?.scrollIntoView({ block: "nearest" }),
    );
  };

  return (e: KeyboardEvent) => {
    const target = e.target as HTMLElement;
    const inSearch = target === opts.searchRef.current;
    const onList = target === opts.listRef.current;
    if (!inSearch && !onList) return;
    // The Enter that commits an IME candidate must not connect.
    if (e.key === "Enter" && isImeEvent(e)) return;
    // While the recent targets show under the field, the arrows go through them first.
    if (inSearch && opts.suggestOpen) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = opts.suggestions.length;
        opts.setSuggestAt(
          e.key === "ArrowDown" ? Math.min(n - 1, opts.suggestActive + 1) : Math.max(-1, opts.suggestActive - 1),
        );
        return;
      }
      if (e.key === "Enter" && opts.suggestActive >= 0) {
        e.preventDefault();
        opts.connectQuick(opts.suggestions[opts.suggestActive]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        opts.setSuggestDismissed(true);
        return;
      }
    }
    const { focusId, hostIds, selected, selectedSet, visible, view, quickRows, platform } = opts;
    const amongCards = view === "cards" && !quickRows.some((r) => r.id === focusId);
    const mod = isMac(platform) ? e.metaKey : e.ctrlKey;
    const selectedHosts = () => visible.filter((h) => selectedSet.has(h.id));
    if (onList && e.altKey && !mod && !e.shiftKey && (e.key === "ArrowDown" || e.key === "ArrowUp")) {
      // HOST-16: Alt+↑/↓ (⌥↑/↓) moves the selected hosts a place, as dragging them there would.
      e.preventDefault();
      if (opts.searching) return;
      const moving = hostIds.filter((id) => selectedSet.has(id));
      const staying = hostIds.filter((id) => !selectedSet.has(id));
      if (moving.length === 0 || staying.length === 0) return;
      const first = hostIds.indexOf(moving[0]);
      const last = hostIds.indexOf(moving[moving.length - 1]);
      if (e.key === "ArrowUp") {
        const above = staying.filter((id) => hostIds.indexOf(id) < first);
        opts.moveTo(moving, above.length > 0 ? above[above.length - 1] : staying[0]);
      } else {
        const below = staying.filter((id) => hostIds.indexOf(id) > last);
        opts.moveTo(moving, below.length > 1 ? below[1] : null);
      }
      requestAnimationFrame(() =>
        opts.listRef.current
          ?.querySelector(`[data-id="${CSS.escape(focusId ?? moving[0])}"]`)
          ?.scrollIntoView({ block: "nearest" }),
      );
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const step = amongCards ? cardsPerRow() : 1;
      move(e.key === "ArrowDown" ? step : -step, e.shiftKey);
    } else if (onList && amongCards && (e.key === "ArrowRight" || e.key === "ArrowLeft")) {
      e.preventDefault();
      move(e.key === "ArrowRight" ? 1 : -1, e.shiftKey);
    } else if (onList && e.key === " " && opts.stats && focusId && opts.hosts.some((h) => h.id === focusId)) {
      // Space shows the selected host's detail, as Quick Look does.
      e.preventDefault();
      opts.setDetail(focusId);
    } else if (onList && mod && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "a") {
      // Every listed host, the one the arrows move on from still last.
      e.preventDefault();
      opts.anchorRef.current = focusId;
      opts.pickRows(
        focusId && hostIds.includes(focusId) ? [...hostIds.filter((id) => id !== focusId), focusId] : [...hostIds],
      );
    } else if (onList && e.key === "Escape" && selected.length > 0) {
      e.preventDefault();
      opts.selectNone();
    } else if (onList && (e.key === "Delete" || (isMac(platform) && e.metaKey && e.key === "Backspace"))) {
      e.preventDefault();
      opts.remove(selectedHosts());
    } else if (e.key === "Enter" && opts.several) {
      e.preventDefault();
      opts.connectAll(selectedHosts());
    } else if (e.key === "Enter" && focusId === TELNET_ROW) {
      e.preventDefault();
      // The list may still show an earlier keystroke: connect what the field says now.
      const typed = parseTelnet(opts.query);
      if (!typed?.ok) return;
      const saved = savedTelnetHostFor(opts.telnetHosts, typed.target);
      if (saved) opts.connect(saved.id);
      else connectTelnetTarget(typed.target.address, typed.target.port);
    } else if (e.key === "Enter" && focusId === TYPED_ROW) {
      e.preventDefault();
      // The list may still show an earlier keystroke: connect what the field says now.
      const typed = parseQuickConnect(opts.query);
      if (!typed?.ok) return;
      const saved = savedHostFor(opts.hosts, typed.target);
      if (saved) opts.connect(saved.id);
      else void connectTarget(typed.target);
    } else if (e.key === "Enter" && focusId) {
      e.preventDefault();
      opts.connect(focusId);
    } else if (onList && e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey && e.key !== " ") {
      // Type to search.
      opts.searchRef.current?.focus();
    }
  };
}
