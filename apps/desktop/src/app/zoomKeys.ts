import { useEffect } from "react";
import { toast } from "@/components/overlay";
import { defaultTerminalSettings, FONT_SIZE_MAX, FONT_SIZE_MIN, useTermSettings } from "@/features/terminal/theme";
import { t } from "@/i18n";
import { api } from "@/ipc/api";
import type { Platform } from "@/lib/platform";
import { ZOOM_STEPS } from "@/lib/zoom";
import { errorMessage } from "./errors";
import { matchZoom, type ZoomAction } from "./shortcuts";
import { useApp } from "./store";
import { showZoomHud } from "./ZoomHud";

export type ZoomTarget = "terminal" | "interface";

/** What a zoom at `el` changes (TERM-13): the terminal font in a terminal, else the interface. */
export function zoomTarget(el: Element | null): ZoomTarget {
  if (el?.closest(".xterm")) return "terminal";
  return "interface";
}

/** The next interface zoom step from `current`, no larger than `max`. */
export function nextZoom(current: number, action: ZoomAction, max: number): number {
  if (action === "reset") return 100;
  const next = action === "in" ? ZOOM_STEPS.find((s) => s > current) : ZOOM_STEPS.findLast((s) => s < current);
  return Math.min(next ?? current, max);
}

/** The next terminal font size from `current`. */
export function nextFontSize(current: number, action: ZoomAction, platform: Platform): number {
  if (action === "reset") return defaultTerminalSettings(platform).font_size;
  return Math.min(FONT_SIZE_MAX, Math.max(FONT_SIZE_MIN, current + (action === "in" ? 1 : -1)));
}

/**
 * Saves the latest value passed, one save at a time: keys held down or a fast wheel set many sizes,
 * and only the last one needs to reach the store.
 */
function latestSaver<V>(save: (v: V) => Promise<unknown>): (v: V) => void {
  let pending: { v: V } | null = null;
  let saving = false;
  return (v) => {
    pending = { v };
    if (saving) return;
    saving = true;
    void (async () => {
      while (pending) {
        const { v: next } = pending;
        pending = null;
        await save(next).catch((e) => toast(errorMessage(t, e), "error"));
      }
      saving = false;
    })();
  };
}

const saveFontSize = latestSaver((font_size: number) => api.settings_save({ terminal: { font_size } }));

async function zoomInterface(action: ZoomAction) {
  const app = useApp.getState();
  const next = nextZoom(app.zoom.zoom, action, app.zoom.max);
  const zoom = next === app.zoom.zoom ? app.zoom : await app.setZoom(next);
  showZoomHud(`${zoom.zoom}%`, null);
}

/** Every terminal tab takes the new size, which is the synced Settings → Terminal → Font Size. */
function zoomTerminal(action: ZoomAction, el: Element | null, platform: Platform) {
  const { settings } = useTermSettings.getState();
  const font_size = nextFontSize(settings.font_size, action, platform);
  if (font_size !== settings.font_size) {
    useTermSettings.setState({ settings: { ...settings, font_size } });
    saveFontSize(font_size);
  }
  const term = el?.closest(".xterm")?.parentElement;
  showZoomHud(`${font_size} px`, term?.getBoundingClientRect() ?? null);
}

function zoom(action: ZoomAction, el: Element | null, platform: Platform) {
  if (zoomTarget(el) === "terminal") zoomTerminal(action, el, platform);
  else zoomInterface(action).catch((e) => toast(errorMessage(t, e), "error"));
}

/** Wheel pixels per zoom step: a mouse notch is about 100, a touchpad pinch many small deltas. */
const WHEEL_STEP = 50;
/** A pause this long starts the count again. */
const WHEEL_IDLE_MS = 300;

/**
 * TERM-13: the zoom keys change what has focus, and Ctrl+wheel what is under the pointer, also in a
 * terminal that a program reads the mouse in. macOS has the keys only.
 */
export function useZoomKeys(platform: Platform) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const action = matchZoom(e, platform);
      if (!action) return;
      e.preventDefault();
      e.stopPropagation();
      zoom(action, document.activeElement, platform);
    };
    let wheel = 0;
    let lastWheel = 0;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey || e.metaKey || e.altKey || platform === "macos") return;
      // Before xterm, which would scroll or report it to the program.
      e.preventDefault();
      e.stopPropagation();
      const delta = e.deltaMode === WheelEvent.DOM_DELTA_PIXEL ? e.deltaY : e.deltaY * WHEEL_STEP;
      if (e.timeStamp - lastWheel > WHEEL_IDLE_MS || Math.sign(delta) !== Math.sign(wheel)) wheel = 0;
      lastWheel = e.timeStamp;
      wheel += delta;
      if (Math.abs(wheel) < WHEEL_STEP) return;
      const action = wheel < 0 ? "in" : "out";
      wheel = 0;
      zoom(action, e.target instanceof Element ? e.target : null, platform);
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("wheel", onWheel, { capture: true, passive: false });
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("wheel", onWheel, { capture: true });
    };
  }, [platform]);
}
