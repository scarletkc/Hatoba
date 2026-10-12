import { useEffect, useMemo, useRef } from "react";
import { useApp } from "@/app/store";

export interface FreshReads {
  /** Reads the data again, as after a sync round; to retry a failed read. */
  refresh(): void;
  /**
   * Runs the save of an edit the pane already shows. Reads wait until it has landed, and when it fails
   * the pane reads what is stored.
   */
  save(run: () => Promise<void>): Promise<void>;
  /** The pane shows an edit that was saved elsewhere (a dialog's), which a read in flight may predate. */
  edited(): void;
}

/**
 * Reads the synced data a settings pane shows: when the pane opens, and again after each sync status
 * that may follow a pull, so the pane shows what other devices changed. A read never replaces newer data:
 * it waits while a save of the pane's is in flight, one that an edit raced is dropped and made again, and
 * one that a later read overtook is dropped.
 */
export function useFreshReads<T>(read: () => Promise<T>, show: (data: T) => void, failed: (e: unknown) => void): FreshReads {
  const latest = useRef({ read, show, failed });
  latest.current = { read, show, failed };
  const state = useRef({ live: true, edits: 0, saving: 0, waiting: false, reads: 0 });
  const syncTick = useApp((st) => st.syncTick);

  const reads = useMemo((): FreshReads => {
    const s = state.current;
    const refresh = () => {
      if (s.saving > 0) {
        s.waiting = true;
        return;
      }
      s.waiting = false;
      const edits = s.edits;
      // Only the latest read answers: an earlier one that finishes after it has older data.
      const read = ++s.reads;
      latest.current.read().then(
        (data) => {
          if (!s.live || read !== s.reads) return;
          if (s.edits === edits) latest.current.show(data);
          else if (s.saving > 0) s.waiting = true;
          else refresh();
        },
        (e: unknown) => {
          if (s.live && read === s.reads) latest.current.failed(e);
        },
      );
    };
    const save = async (run: () => Promise<void>) => {
      s.edits++;
      s.saving++;
      try {
        await run();
      } catch (e) {
        s.waiting = true;
        throw e;
      } finally {
        s.saving--;
        if (s.saving === 0 && s.waiting && s.live) refresh();
      }
    };
    const edited = () => {
      s.edits++;
    };
    return { refresh, save, edited };
  }, []);

  useEffect(() => {
    const s = state.current;
    s.live = true;
    return () => {
      s.live = false;
    };
  }, []);
  useEffect(() => reads.refresh(), [reads, syncTick]);

  return reads;
}
