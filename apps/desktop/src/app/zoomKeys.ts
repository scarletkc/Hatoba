import { useEffect } from "react";
import { toast } from "@/components/overlay";
import { useAi } from "@/features/ai/store";
import { AI_TEXT_SIZE_DEFAULT, AI_TEXT_SIZE_MAX, AI_TEXT_SIZE_MIN } from "@/features/ai/textSize";
import { defaultTerminalSettings, FONT_SIZE_MAX, FONT_SIZE_MIN, useTermSettings } from "@/features/terminal/theme";
import { t } from "@/i18n";
import { api } from "@/ipc/api";
import type { Platform } from "@/lib/platform";
import { ZOOM_STEPS } from "@/lib/zoom";
import { errorMessage } from "./errors";
import { matchZoom, type ZoomAction } from "./shortcuts";
import { useApp } from "./store";
import { showZoomHud } from "./ZoomHud";

export type ZoomTarget = "terminal" | "ai" | "interface";

/** What a zoom at `el` changes (TERM-13): the terminal font in a terminal, the AI text in the AI panel, else the interface. */
export function zoomTarget(el: Element | null): ZoomTarget {
  if (el?.closest(".xterm")) return "terminal";
  if (el?.closest("[data-zoom='ai']")) return "ai";
  return "interface";
}

/** The next interface zoom step from `current`, no larger than `max`. */
export function nextZoom(current: number, action: ZoomAction, max: number): number {
  if (action === "reset") return 100;
  const next = action === "in" ? ZOOM_STEPS.find((s) => s > current) : ZOOM_STEPS.findLast((s) => s < current);
  return Math.min(next ?? current, max);
}

/** The next size from `current`, one px a step within `min` and `max`. */
export function nextSize(current: number, action: ZoomAction, { min, max, reset }: { min: number; max: number; reset: number }): number {
  if (action === "reset") return reset;
  return Math.min(max, Math.max(min, current + (action === "in" ? 1 : -1)));
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
const saveAiTextSize = latestSaver((text_size: number) => api.ai_settings_save({ text_size }));

async function zoomInterface(action: ZoomAction) {
  const app = useApp.getState();
  const next = nextZoom(app.zoom.zoom, action, app.zoom.max);
  const zoom = next === app.zoom.zoom ? app.zoom : await app.setZoom(next);
  showZoomHud(`${zoom.zoom}%`, null);
}

/** Every terminal tab takes the new size, which is the synced Settings → Terminal → Font Size. */
function zoomTerminal(action: ZoomAction, el: Element | null, platform: Platform) {
  const { settings } = useTermSettings.getState();
  const reset = defaultTerminalSettings(platform).font_size;
  const font_size = nextSize(settings.font_size, action, { min: FONT_SIZE_MIN, max: FONT_SIZE_MAX, reset });
  if (font_size !== settings.font_size) {
    useTermSettings.setState({ settings: { ...settings, font_size } });
    saveFontSize(font_size);
  }
  const term = el?.closest(".xterm")?.parentElement;
  showZoomHud(`${font_size} px`, term?.getBoundingClientRect() ?? null);
}

/** The synced Settings → Appearance → AI Panel → Text Size; the panel's conversation and input take it at once. */
export function setAiTextSize(text_size: number) {
  const { catalog } = useAi.getState();
  const settings = catalog.settings;
  if (!settings || text_size === settings.text_size) return;
  useAi.setState({ catalog: { ...catalog, settings: { ...settings, text_size } } });
  saveAiTextSize(text_size);
}

function zoomAiText(action: ZoomAction, el: Element | null) {
  const settings = useAi.getState().catalog.settings;
  if (!settings) return;
  const text_size = nextSize(settings.text_size, action, { min: AI_TEXT_SIZE_MIN, max: AI_TEXT_SIZE_MAX, reset: AI_TEXT_SIZE_DEFAULT });
  setAiTextSize(text_size);
  showZoomHud(`${text_size} px`, el?.closest("[data-zoom='ai']")?.getBoundingClientRect() ?? null);
}

function zoom(action: ZoomAction, el: Element | null, platform: Platform) {
  switch (zoomTarget(el)) {
    case "terminal":
      return zoomTerminal(action, el, platform);
    case "ai":
      return zoomAiText(action, el);
    case "interface":
      return void zoomInterface(action).catch((e) => toast(errorMessage(t, e), "error"));
  }
}

/** Wheel pixels per font step: a mouse notch is about 100, a touchpad pinch many small deltas. */
const WHEEL_STEP = 50;
/** A pause this long starts the count again. */
const WHEEL_IDLE_MS = 300;

/**
 * TERM-13: the zoom keys change what has focus. Ctrl+wheel changes only the font of the terminal
 * under the pointer, as in Windows Terminal, and also while a program reads the mouse there.
 * Elsewhere a stray Ctrl+wheel or touchpad pinch would zoom the whole window, which editors leave to
 * the keys.
 * macOS has the keys only.
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
      const target = e.target instanceof Element ? e.target : null;
      if (zoomTarget(target) !== "terminal") return;
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
      zoomTerminal(action, target, platform);
    };
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("wheel", onWheel, { capture: true, passive: false });
    return () => {
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("wheel", onWheel, { capture: true });
    };
  }, [platform]);
}
