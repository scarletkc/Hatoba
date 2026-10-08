import { useEffect, useRef, useState, type DragEvent, type RefObject } from "react";
import { toast } from "@/components/overlay";
import { t } from "@/i18n";
import { isTauri } from "@/ipc/api";

/**
 * AI-35: text files dropped on the AI panel. The WebView's own drag and drop hands over the files,
 * which are read with the File API. The desktop app's webview takes file drops itself (for the
 * SFTP panel's uploads, `features/sftp/useFileDrop.ts`) and gives only paths, which the WebView
 * cannot read, so there a drop on the panel says to use the paperclip.
 */
export function usePanelFileDrop(panel: RefObject<HTMLElement | null>, enabled: boolean, onFiles: (files: File[]) => void) {
  const [dragging, setDragging] = useState(false);
  const latest = useRef(onFiles);
  useEffect(() => {
    latest.current = onFiles;
  });

  useEffect(() => {
    if (!isTauri() || !enabled) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    const inside = (pos: { x: number; y: number }) => {
      const r = panel.current?.getBoundingClientRect();
      if (!r) return false;
      const scale = window.devicePixelRatio || 1;
      return pos.x / scale >= r.left && pos.x / scale <= r.right && pos.y / scale >= r.top && pos.y / scale <= r.bottom;
    };
    void import("@tauri-apps/api/webview")
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent(({ payload }) => {
          if (payload.type === "drop" && inside(payload.position) && payload.paths.length > 0) toast(t("ai.attach.dropUnsupported"), "info");
        }),
      )
      .then((un) => {
        if (cancelled) un();
        else unlisten = un;
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [enabled, panel]);

  const hasFiles = (e: DragEvent) => enabled && !isTauri() && e.dataTransfer.types.includes("Files");
  const handlers = {
    onDragEnter: (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      setDragging(true);
    },
    onDragOver: (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "copy";
    },
    onDragLeave: (e: DragEvent) => {
      if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
    },
    onDrop: (e: DragEvent) => {
      if (!hasFiles(e)) return;
      e.preventDefault();
      setDragging(false);
      const files = Array.from(e.dataTransfer.files);
      if (files.length > 0) latest.current(files);
    },
  };
  return { dragging, handlers };
}
