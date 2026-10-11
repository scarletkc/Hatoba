import { useEffect, useRef, useState } from "react";
import type { AiFilePreview } from "@/ipc/types";
import { aiErrorMessage, previewFileCall } from "./actions";

/** How long the card waits after the last keystroke in Edit before it asks for the diff again. */
const EDIT_PAUSE_MS = 300;

/** Where the approval card's diff stands (AI-39, AI-40). */
export interface FilePreviewState {
  preview: AiFilePreview | null;
  /** The preview is being read; Run waits for it, so the user approves what they saw. */
  loading: boolean;
  /** The preview could not be asked for at all (as opposed to a call that cannot apply). */
  failure: string | null;
}

/**
 * AI-39, AI-40: the change an `edit_file` or `write_file` call waiting for approval would make. Rust
 * reads the file once per call; `edited` (the arguments changed in Edit, or null) asks again after a
 * pause in typing, against the same read. Nothing is asked while `enabled` is false (a card of another
 * tool).
 */
export function useFilePreview(slotId: string, callId: string, edited: string | null, enabled: boolean): FilePreviewState {
  const [state, setState] = useState<FilePreviewState>({ preview: null, loading: enabled, failure: null });
  const asked = useRef(false);
  useEffect(() => {
    if (!enabled) return;
    let current = true;
    setState((st) => ({ ...st, loading: true }));
    const timer = window.setTimeout(
      () => {
        asked.current = true;
        previewFileCall(slotId, callId, edited).then(
          (preview) => current && setState({ preview, loading: false, failure: null }),
          (e: unknown) => current && setState({ preview: null, loading: false, failure: aiErrorMessage(e) }),
        );
      },
      asked.current ? EDIT_PAUSE_MS : 0,
    );
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
  }, [slotId, callId, edited, enabled]);
  return state;
}
