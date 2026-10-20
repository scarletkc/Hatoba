import { useEffect, useLayoutEffect, useState, type PointerEvent as ReactPointerEvent } from "react";
import { motion } from "@/lib/motion";
import { useTabs, type SessionTab } from "./tabs";

/** How far the pointer moves sideways before a press on a tab becomes a drag, in CSS pixels. */
const DRAG_THRESHOLD = 4;
/** How long a tab takes to slide into its place: no time with reduced motion. */
const slideMs = () => motion("--motion-fast").duration;

/** Where a tab was in the strip when the drag started, in viewport pixels. */
export interface Slot {
  left: number;
  width: number;
}

/**
 * The index the dragged tab lands at, its left edge at `left`: after each tab that followed it whose middle its right
 * edge has passed, and before each tab ahead of it whose middle its left edge has passed. Edges rather than its middle,
 * so a wide tab held against the end of the strip still passes a narrow last tab.
 */
export function dropIndex(slots: Slot[], from: number, left: number): number {
  const right = left + slots[from].width;
  let index = from;
  slots.forEach((slot, i) => {
    const middle = slot.left + slot.width / 2;
    if (i > from && right > middle) index++;
    if (i < from && left < middle) index--;
  });
  return index;
}

/** How far the tab at `i` moves aside while the tab at `from` is held over `to`; `step` is the dragged tab's width and the gap. */
export function shiftOf(i: number, from: number, to: number, step: number): number {
  if (from < i && i <= to) return -step;
  if (to <= i && i < from) return step;
  return 0;
}

interface Drag {
  id: string;
  el: HTMLElement;
  pointerId: number;
  startX: number;
  started: boolean;
  /** Set when the drag starts: the session tabs' elements, ids and places, in order. */
  items: HTMLElement[];
  ids: string[];
  slots: Slot[];
  from: number;
  to: number;
  dx: number;
  step: number;
}

function translate(el: HTMLElement, dx: number) {
  el.style.transform = dx ? `translateX(${dx}px)` : "";
}

/** The click that ends a drag lands on a tab; it must not act. */
function swallowClick() {
  const swallow = (e: MouseEvent) => {
    e.stopPropagation();
    e.preventDefault();
  };
  window.addEventListener("click", swallow, { capture: true, once: true });
  window.setTimeout(() => window.removeEventListener("click", swallow, { capture: true }), 0);
}

/**
 * Drags a session tab along the tab bar (TERM-01), as in browsers: the tab follows the pointer within the strip, the
 * others slide aside, and dropping it moves it there. It takes no part in HTML drag and drop, which Tauri's file drop
 * takes over on Windows. As in browsers, pressing a tab brings it to the front, so the drag measures the tabs as they
 * show with it in front; Escape puts it back.
 */
export class TabDrag {
  private drag: Drag | null = null;
  /** A drop the store has not shown yet: the tabs to put back in the flow, and where the dragged one was let go. */
  private landing: { el: HTMLElement; items: HTMLElement[]; left: number } | null = null;
  private slide: { els: HTMLElement[]; timer: number } | null = null;

  /** `pointerdown` on the tab `id`, whose element has `data-tab-id`. A press on its close button only closes it. */
  down = (e: ReactPointerEvent<HTMLElement>, id: string) => {
    if (e.button !== 0 || this.drag) return;
    const close = (e.target as Element).closest('[role="button"]');
    if (close && e.currentTarget.contains(close)) return;
    useTabs.getState().activate(id);
    this.drag = {
      id,
      el: e.currentTarget,
      pointerId: e.pointerId,
      startX: e.clientX,
      started: false,
      items: [],
      ids: [],
      slots: [],
      from: 0,
      to: 0,
      dx: 0,
      step: 0,
    };
    window.addEventListener("pointermove", this.move);
    window.addEventListener("pointerup", this.up);
    window.addEventListener("pointercancel", this.cancel);
    window.addEventListener("keydown", this.key, true);
    window.addEventListener("blur", this.cancel);
  };

  /** After each change to the tabs: shows a drop in place, and ends a press or drag whose tabs opened or closed meanwhile. */
  rendered(tabs: SessionTab[]) {
    const d = this.drag;
    if (d && !tabs.some((t) => t.id === d.id)) this.cancel();
    else if (d?.started && (tabs.length !== d.ids.length || tabs.some((t, i) => t.id !== d.ids[i]))) this.cancel();
    const landing = this.landing;
    if (!landing) return;
    this.landing = null;
    for (const item of landing.items) {
      item.style.transition = "";
      translate(item, 0);
    }
    this.slideHome(landing.el, landing.left - landing.el.getBoundingClientRect().left, []);
  }

  private move = (e: PointerEvent) => {
    const d = this.drag;
    if (!d || e.pointerId !== d.pointerId) return;
    const dx = e.clientX - d.startX;
    if (!d.started) {
      if (Math.abs(dx) < DRAG_THRESHOLD) return;
      if (!this.start(d)) {
        this.cancel();
        return;
      }
    }
    const { slots, from } = d;
    const last = slots[slots.length - 1];
    d.dx = Math.min(Math.max(dx, slots[0].left - slots[from].left), last.left + last.width - slots[from].left - slots[from].width);
    translate(d.el, d.dx);
    const to = dropIndex(slots, from, slots[from].left + d.dx);
    if (to === d.to) return;
    d.to = to;
    d.items.forEach((item, i) => {
      if (i !== from) translate(item, shiftOf(i, from, to, d.step));
    });
  };

  /** Measures the tabs and lifts the pressed one; false when it is no longer in the strip. */
  private start(d: Drag): boolean {
    this.endSlide();
    const strip = d.el.parentElement;
    if (!strip) return false;
    d.items = Array.from(strip.querySelectorAll<HTMLElement>(":scope > [data-tab-id]"));
    if (!d.items.includes(d.el)) return false;
    d.ids = d.items.map((item) => item.dataset.tabId!);
    d.slots = d.items.map((item) => {
      const r = item.getBoundingClientRect();
      return { left: r.left, width: r.width };
    });
    d.from = d.to = d.items.indexOf(d.el);
    d.step = d.slots[d.from].width + (parseFloat(getComputedStyle(strip).columnGap) || 0);
    d.started = true;
    d.el.dataset.dragging = "";
    const ms = slideMs();
    for (const item of d.items) if (item !== d.el) item.style.transition = `transform ${ms}ms var(--ease-out)`;
    try {
      d.el.setPointerCapture(d.pointerId);
    } catch {
      // The pointer is gone; the window's listeners still end the drag.
    }
    return true;
  }

  private up = (e: PointerEvent) => {
    const d = this.drag;
    if (!d || e.pointerId !== d.pointerId) return;
    this.stop();
    if (!d.started) return;
    swallowClick();
    const tabs = useTabs.getState().tabs;
    if (d.to !== d.from) useTabs.getState().moveTab(d.id, d.to);
    if (useTabs.getState().tabs === tabs) {
      this.putBack(d);
      return;
    }
    // The tabs stay where they show until the store's order is on screen (`rendered`).
    this.landing = { el: d.el, items: d.items, left: d.slots[d.from].left + d.dx };
  };

  private key = (e: KeyboardEvent) => {
    if (e.key !== "Escape" || !this.drag?.started) return;
    e.preventDefault();
    e.stopPropagation();
    this.cancel();
  };

  /** Ends a drag without moving the tab. */
  cancel = () => {
    const d = this.drag;
    if (!d) return;
    this.stop();
    if (!d.started) return;
    try {
      d.el.releasePointerCapture(d.pointerId);
    } catch {
      // Already released.
    }
    this.putBack(d);
  };

  /** Stops listening to the pointer. */
  private stop() {
    this.drag = null;
    window.removeEventListener("pointermove", this.move);
    window.removeEventListener("pointerup", this.up);
    window.removeEventListener("pointercancel", this.cancel);
    window.removeEventListener("keydown", this.key, true);
    window.removeEventListener("blur", this.cancel);
  }

  /** Slides every tab back to where it was. */
  private putBack(d: Drag) {
    const others = d.items.filter((item) => item !== d.el);
    for (const item of others) translate(item, 0);
    this.slideHome(d.el, d.dx, others);
  }

  /** Slides the dragged tab from `dx` off its place into it, then clears what the drag set on the tabs. */
  private slideHome(el: HTMLElement, dx: number, others: HTMLElement[]) {
    el.style.transition = "none";
    translate(el, dx);
    void el.offsetWidth; // Starts the slide from `dx`.
    const ms = slideMs();
    el.style.transition = `transform ${ms}ms var(--ease-out)`;
    translate(el, 0);
    const els = [el, ...others];
    this.slide = { els, timer: window.setTimeout(() => this.endSlide(), ms) };
  }

  private endSlide() {
    if (!this.slide) return;
    window.clearTimeout(this.slide.timer);
    for (const el of this.slide.els) {
      el.style.transition = "";
      translate(el, 0);
      delete el.dataset.dragging;
    }
    this.slide = null;
  }

  dispose() {
    this.cancel();
    this.endSlide();
  }
}

/** The tab bar's drag of session tabs; pass its `down` to each session tab's `onPointerDown`. */
export function useTabDrag(tabs: SessionTab[]): TabDrag {
  const [drag] = useState(() => new TabDrag());
  useLayoutEffect(() => drag.rendered(tabs), [drag, tabs]);
  useEffect(() => () => drag.dispose(), [drag]);
  return drag;
}
