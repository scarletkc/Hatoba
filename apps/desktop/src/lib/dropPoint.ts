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
 * the view's own coordinates on macOS (`wkwebview/drag_drop.rs`) and Linux, where GTK's `drag-drop`
 * signal reports logical pixels (`webkitgtk/drag_drop.rs`, passed through unscaled by
 * tauri-runtime-wry).
 *
 * On Windows `devicePixelRatio` holds both the display scale and the interface zoom (TERM-13). On
 * macOS and Linux the view's pixels are CSS pixels times `zoom` alone: WebKit's `devicePixelRatio`
 * holds the zoom too, but also the display scale, which would shrink every drop's coordinates (a
 * 200%-scaled Linux desktop halves them).
 */
export function dropPointToCss(pos: { x: number; y: number }, platform: Platform, devicePixelRatio: number, zoom: number) {
  const scale = (platform === "windows" ? devicePixelRatio : zoom) || 1;
  return { x: pos.x / scale, y: pos.y / scale };
}

/** True when a native drop at `pos` lands inside `box`. */
export function isDropInside(
  box: Box | undefined,
  pos: { x: number; y: number },
  platform: Platform,
  devicePixelRatio: number,
  zoom: number,
): boolean {
  if (!box) return false;
  const { x, y } = dropPointToCss(pos, platform, devicePixelRatio, zoom);
  return x >= box.left && x <= box.right && y >= box.top && y <= box.bottom;
}
