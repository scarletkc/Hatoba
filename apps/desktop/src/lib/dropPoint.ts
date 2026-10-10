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
 * but wry fills it in per platform: real physical pixels on Windows, and the view's own coordinates
 * in points on macOS (wry `wkwebview/drag_drop.rs`), which are CSS pixels already. Dividing the
 * macOS value by the Retina scale would put every drop at half the coordinates.
 */
export function dropPointToCss(pos: { x: number; y: number }, platform: Platform, devicePixelRatio: number) {
  const scale = platform === "macos" ? 1 : devicePixelRatio || 1;
  return { x: pos.x / scale, y: pos.y / scale };
}

/** True when a native drop at `pos` lands inside `box`. */
export function isDropInside(box: Box | undefined, pos: { x: number; y: number }, platform: Platform, devicePixelRatio: number): boolean {
  if (!box) return false;
  const { x, y } = dropPointToCss(pos, platform, devicePixelRatio);
  return x >= box.left && x <= box.right && y >= box.top && y <= box.bottom;
}
