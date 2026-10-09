import { useEffect, useRef, useState, type DragEvent, type RefObject } from "react";
import { isTauri } from "@/ipc/api";

/** Where text files dropped on the AI panel go: `File`s in the browser, paths in the desktop app. */
export interface PanelDrop {
  onFiles: (files: File[]) => void;
  onPaths: (paths: string[]) => void;
}

/**
 * AI-35: text files dropped on the AI panel. In the browser the WebView's own drag and drop hands
 * over the files, which are read with the File API. The desktop app's webview takes file drops
 * itself (as for the SFTP panel's uploads, `features/sftp/useFileDrop.ts`) and gives only their
 * paths, positions in physical pixels; Rust reads those (`ai_read_dropped_files`), and only the
 * paths of the window's last drop.
 */
export function usePanelFileDrop(panel: RefObject<HTMLElement | null>, enabled: boolean, drop: PanelDrop) {
  const [dragging, setDragging] = useState(false);
  const latest = useRef(drop);
  useEffect(() => {
    latest.current = drop;
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
          if (payload.type === "leave") setDragging(false);
          else if (payload.type === "drop") {
            setDragging(false);
            if (inside(payload.position) && payload.paths.length > 0) latest.current.onPaths(payload.paths);
          } else setDragging(inside(payload.position));
        }),
      )
      .then((un) => {
        if (cancelled) un();
        else unlisten = un;
      });
    return () => {
      cancelled = true;
      unlisten?.();
      setDragging(false);
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
      if (files.length > 0) latest.current.onFiles(files);
    },
  };
  return { dragging, handlers };
}
