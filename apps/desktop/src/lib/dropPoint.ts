import type { Platform } from "./platform";

/** The edges of an element in CSS pixels, as `getBoundingClientRect` returns them. */
interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/**
 * Converts the position of a native file drop to CSS pixels. Tauri types it as a `PhysicalPosition`,
 * but wry fills it in per platform: real physical pixels on Windows (`webview2/drag_drop.rs`), and
 * the view's own coordinates — CSS pixels already — on macOS (`wkwebview/drag_drop.rs`) and Linux,
 * where GTK's `drag-drop` signal reports logical pixels (`webkitgtk/drag_drop.rs`, passed through
 * unscaled by tauri-runtime-wry). Dividing those by the display scale would shrink every drop's
 * coordinates (a 200%-scaled Linux desktop halves them).
 */
export function dropPointToCss(pos: { x: number; y: number }, platform: Platform, devicePixelRatio: number) {
  const scale = platform === "windows" ? devicePixelRatio || 1 : 1;
  return { x: pos.x / scale, y: pos.y / scale };
}

/** True when a native drop at `pos` lands inside `box`. */
export function isDropInside(box: Box | undefined, pos: { x: number; y: number }, platform: Platform, devicePixelRatio: number): boolean {
  if (!box) return false;
  const { x, y } = dropPointToCss(pos, platform, devicePixelRatio);
  return x >= box.left && x <= box.right && y >= box.top && y <= box.bottom;
}
