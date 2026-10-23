/** TERM-13: the interface zoom levels in percent. Keep in step with `ZOOM_STEPS` in src-tauri/src/platform/window.rs. */
export const ZOOM_STEPS = [67, 75, 80, 90, 100, 110, 125, 150, 175, 200] as const;

/** The step nearest to `percent`. */
export function zoomStep(percent: number): number {
  return ZOOM_STEPS.reduce((best, s) => (Math.abs(s - percent) < Math.abs(best - percent) ? s : best), 100);
}

/**
 * Tells the CSS the zoom in effect: `--ui-zoom` is the factor, and `--window-px` (tokens.css) is one
 * window pixel, for chrome that must line up with the native window at every zoom.
 */
export function applyZoom(percent: number) {
  document.documentElement.style.setProperty("--ui-zoom", String(percent / 100));
}
