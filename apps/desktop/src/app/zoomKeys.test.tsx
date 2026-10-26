import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { Platform } from "@/lib/platform";

// The app store follows the system theme from its first import; jsdom has no matchMedia.
vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
const { defaultTerminalSettings, useTermSettings } = await import("@/session/theme");
const { useApp } = await import("./store");
const { nextSize, nextZoom, useZoomKeys } = await import("./zoomKeys");

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let unmount: (() => void) | null = null;
beforeEach(() => useTermSettings.setState({ settings: defaultTerminalSettings("windows") }));
afterEach(() => {
  act(() => unmount?.());
  unmount = null;
  document.body.innerHTML = "";
});

function Keys({ platform }: { platform: Platform }) {
  useZoomKeys(platform);
  return null;
}

function render(platform: Platform = "windows") {
  document.body.innerHTML = `<div class="xterm"><canvas id="screen"></canvas></div><aside data-zoom="ai"><p id="ai"></p></aside><p id="list"></p>`;
  const host = document.createElement("div");
  document.body.appendChild(host);
  const root = createRoot(host);
  act(() => root.render(<Keys platform={platform} />));
  unmount = () => root.unmount();
}

/** Ctrl+wheel over `id`; says whether the page's own handling was kept from it. */
function ctrlWheel(id: string, deltaY: number): boolean {
  const e = new WheelEvent("wheel", { deltaY, ctrlKey: true, bubbles: true, cancelable: true });
  document.getElementById(id)!.dispatchEvent(e);
  return e.defaultPrevented;
}

const fontSize = () => useTermSettings.getState().settings.font_size;

describe("Ctrl+wheel (TERM-13)", () => {
  it("changes the font of the terminal under the pointer, before xterm sees it", () => {
    render();
    let xtermSaw = 0;
    document.getElementById("screen")!.addEventListener("wheel", () => xtermSaw++);
    expect(ctrlWheel("screen", -100)).toBe(true);
    expect(fontSize()).toBe(14);
    expect(ctrlWheel("screen", 100)).toBe(true);
    expect(fontSize()).toBe(13);
    expect(xtermSaw).toBe(0);
  });

  it("adds up a touchpad pinch's small steps", () => {
    render();
    for (let i = 0; i < 4; i++) ctrlWheel("screen", -10);
    expect(fontSize()).toBe(13);
    ctrlWheel("screen", -10);
    expect(fontSize()).toBe(14);
  });

  it("leaves the interface zoom and the AI text to the keys", () => {
    render();
    expect(ctrlWheel("list", -100)).toBe(false);
    expect(ctrlWheel("ai", -100)).toBe(false);
    expect(useApp.getState().zoom.zoom).toBe(100);
    expect(fontSize()).toBe(13);
  });

  it("does nothing on macOS, which has the keys only", () => {
    render("macos");
    expect(ctrlWheel("screen", -100)).toBe(false);
    expect(fontSize()).toBe(13);
  });
});

describe("zoom steps", () => {
  it("moves the interface zoom one browser step, no further than the screen allows", () => {
    expect(nextZoom(100, "in", 200)).toBe(110);
    expect(nextZoom(100, "out", 200)).toBe(90);
    expect(nextZoom(150, "in", 150)).toBe(150);
    expect(nextZoom(67, "out", 200)).toBe(67);
    expect(nextZoom(175, "reset", 200)).toBe(100);
  });

  it("moves a text size 1 px within its range", () => {
    const range = { min: 10, max: 24, reset: 13 };
    expect(nextSize(13, "in", range)).toBe(14);
    expect(nextSize(24, "in", range)).toBe(24);
    expect(nextSize(10, "out", range)).toBe(10);
    expect(nextSize(20, "reset", range)).toBe(13);
  });
});
