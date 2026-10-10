import { useEffect, useRef, useState, type DragEvent, type RefObject } from "react";
import { useApp } from "@/app/store";
import { isTauri } from "@/ipc/api";
import { isDropInside } from "@/lib/dropPoint";

/**
 * Drag & drop upload target (SFTP-02). In the app the webview hands us native file paths
 * (`onDragDropEvent`; `lib/dropPoint.ts` converts the positions); in a plain browser we use HTML5 events and
 * only get file names, which is enough for the mock backend.
 */
export function useFileDrop(panel: RefObject<HTMLElement | null>, enabled: boolean, onDrop: (paths: string[]) => void) {
  const [dragging, setDragging] = useState(false);
  const latest = useRef(onDrop);
  useEffect(() => {
    latest.current = onDrop;
  });

  useEffect(() => {
    if (!isTauri() || !enabled) return;
    let cancelled = false;
    let unlisten: (() => void) | undefined;

    const inside = (pos: { x: number; y: number }) =>
      isDropInside(panel.current?.getBoundingClientRect(), pos, useApp.getState().info.platform, window.devicePixelRatio);

    void import("@tauri-apps/api/webview")
      .then(({ getCurrentWebview }) =>
        getCurrentWebview().onDragDropEvent(({ payload }) => {
          if (payload.type === "leave") setDragging(false);
          else if (payload.type === "drop") {
            setDragging(false);
            if (inside(payload.position) && payload.paths.length > 0) latest.current(payload.paths);
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

  const html5 = isTauri()
    ? {}
    : {
        onDragEnter: (e: DragEvent) => {
          if (!enabled || !e.dataTransfer.types.includes("Files")) return;
          e.preventDefault();
          setDragging(true);
        },
        onDragOver: (e: DragEvent) => {
          if (!enabled || !e.dataTransfer.types.includes("Files")) return;
          e.preventDefault();
        },
        onDragLeave: (e: DragEvent) => {
          if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDragging(false);
        },
        onDrop: (e: DragEvent) => {
          if (!enabled) return;
          e.preventDefault();
          setDragging(false);
          const names = Array.from(e.dataTransfer.files, (f) => f.name);
          if (names.length > 0) latest.current(names);
        },
      };

  return { dragging, html5 };
}
