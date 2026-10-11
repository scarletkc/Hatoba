import type { MouseEvent } from "react";
import { isTauri } from "@/ipc/api";
import type { Platform } from "@/lib/platform";
import s from "./ResizeHandles.module.css";

type ResizeDirection = Parameters<import("@tauri-apps/api/window").Window["startResizeDragging"]>[0];

const DIRECTIONS: ResizeDirection[] = [
  "North",
  "South",
  "East",
  "West",
  "NorthWest",
  "NorthEast",
  "SouthWest",
  "SouthEast",
];

/**
 * Linux's painted window chrome puts the WebView's own 5 px resize hit-test inside the
 * outer margin, which the GTK input shape makes click-through. These handles fill the resize
 * band just outside the painted edge (shown only while `:root[data-floating]`) and start the
 * drag themselves.
 */
export function ResizeHandles({ platform }: { platform: Platform }) {
  if (platform !== "linux" || !isTauri()) return null;

  const onMouseDown = (dir: ResizeDirection) => (e: MouseEvent) => {
    if (e.button !== 0) return;
    e.preventDefault();
    void import("@tauri-apps/api/window")
      .then(({ getCurrentWindow }) => getCurrentWindow().startResizeDragging(dir))
      .catch(() => {});
  };

  return (
    <>
      {DIRECTIONS.map((dir) => (
        <div key={dir} aria-hidden className={s.handle} data-direction={dir} onMouseDown={onMouseDown(dir)} />
      ))}
    </>
  );
}
