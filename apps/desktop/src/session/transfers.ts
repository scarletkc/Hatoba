import { create } from "zustand";
import type { TransferProgressEvent } from "@/ipc/types";

export interface TransferItem extends TransferProgressEvent {
  /** Finished transfers start fading before they are removed. */
  fading: boolean;
}

interface TransfersState {
  items: TransferItem[];
  /** Per session: how many uploads have completed, so the file list knows when to refresh. */
  uploadsDone: Record<string, number>;
  handle(event: TransferProgressEvent): void;
  dismiss(id: string): void;
  purge(sessionId: string): void;
}

const FADE_AFTER_MS = 3500;
const REMOVE_AFTER_MS = 4200;

export const useTransfers = create<TransfersState>((set, get) => ({
  items: [],
  uploadsDone: {},
  handle: (ev) => {
    const prev = get().items.find((i) => i.transfer_id === ev.transfer_id);
    const item: TransferItem = { ...ev, fading: false };
    set((s) => ({
      items: prev ? s.items.map((i) => (i.transfer_id === ev.transfer_id ? item : i)) : [...s.items, item],
      uploadsDone:
        ev.state === "done" && ev.direction === "upload" && prev?.state !== "done"
          ? { ...s.uploadsDone, [ev.session_id]: (s.uploadsDone[ev.session_id] ?? 0) + 1 }
          : s.uploadsDone,
    }));
    // Done / cancelled transfers fade away; failed ones stay (with the error) until dismissed.
    if ((ev.state === "done" || ev.state === "cancelled") && prev?.state !== ev.state) {
      setTimeout(
        () =>
          set((s) => ({ items: s.items.map((i) => (i.transfer_id === ev.transfer_id ? { ...i, fading: true } : i)) })),
        FADE_AFTER_MS,
      );
      setTimeout(() => get().dismiss(ev.transfer_id), REMOVE_AFTER_MS);
    }
  },
  dismiss: (id) => set((s) => ({ items: s.items.filter((i) => i.transfer_id !== id) })),
  purge: (sessionId) => set((s) => ({ items: s.items.filter((i) => i.session_id !== sessionId) })),
}));

/** "38.2 / 61.6 MB": both numbers in the unit of the total. */
export function formatProgress(bytes: number, total: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let i = 0;
  let div = 1;
  while (total / div >= 1024 && i < units.length - 1) {
    div *= 1024;
    i++;
  }
  const f = (n: number) => (i === 0 ? String(Math.round(n)) : (n / div).toFixed(1));
  return `${f(Math.min(bytes, total))} / ${f(total)} ${units[i]}`;
}
