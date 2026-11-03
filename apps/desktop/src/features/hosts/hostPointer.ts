import { useEffect, useLayoutEffect, useState, type PointerEvent as ReactPointerEvent } from "react";
import { swallowClick } from "@/app/tabDrag";

/** How far the pointer moves before a press becomes a drag or a selection box, in CSS pixels. */
const DRAG_THRESHOLD = 4;
/** How near the list's top or bottom edge the pointer scrolls it, and the most it scrolls in a frame. */
const EDGE = 40;
const MAX_SCROLL = 18;
/** The most moving rows, and cards, that follow the pointer; the count says how many move in all. */
const GHOST_ROWS = 5;
const GHOST_CARDS = 3;
/** How far each card in the stack sits behind the one before it. */
const STACK_STEP = 6;

/** A rectangle in the list's content, which scrolls with it. */
export interface Box {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** A listed host and where it is. */
export interface Place extends Box {
  id: string;
}

/** The rows of a list, or cards in a grid that wraps. */
export type Layout = "rows" | "grid";

/** A box with Shift adds the hosts it touches to the selection, with Ctrl (⌘) switches them, and otherwise selects them alone. */
export type BoxMode = "replace" | "add" | "toggle";

/** The hosts the box touches, in the list's order. */
export function hostsInBox(places: Place[], box: Box): string[] {
  return places
    .filter((p) => p.left < box.right && p.right > box.left && p.top < box.bottom && p.bottom > box.top)
    .map((p) => p.id);
}

/** The part of `box` inside `within`, or null when none of it is. */
export function clipBox(box: Box, within: Box): Box | null {
  const left = Math.max(box.left, within.left);
  const top = Math.max(box.top, within.top);
  const right = Math.min(box.right, within.right);
  const bottom = Math.min(box.bottom, within.bottom);
  return right > left && bottom > top ? { left, top, right, bottom } : null;
}

/** The selection while a box touches `inBox`, from the selection `start` the press began with. */
export function boxSelection(start: string[], inBox: string[], mode: BoxMode): string[] {
  if (mode === "replace") return inBox;
  const boxed = new Set(inBox);
  const kept = start.filter((id) => !boxed.has(id));
  if (mode === "add") return [...kept, ...inBox];
  const had = new Set(start);
  return [...kept, ...inBox.filter((id) => !had.has(id))];
}

/**
 * The host that hosts dropped at (`x`, `y`) go before, or null for after the last. `places` leaves out the hosts being
 * moved. Rows go by their middle; cards go row by row, then by the middle of each card.
 */
export function dropBefore(places: Place[], x: number, y: number, layout: Layout): string | null {
  const next = places.find((p) =>
    layout === "rows" ? (p.top + p.bottom) / 2 > y : p.top > y || (p.bottom >= y && (p.left + p.right) / 2 > x),
  );
  return next?.id ?? null;
}

/**
 * The line that shows where the hosts go: along the top of the row they go before, or down the left of the card, in
 * the middle of the gap; after the last row or card at the end. Between two rows of cards, the line goes at the end
 * of the row the pointer is in.
 */
export function dropLine(places: Place[], before: string | null, y: number, layout: Layout, gap: number): Box | null {
  if (places.length === 0) return null;
  const i = before === null ? places.length : places.findIndex((p) => p.id === before);
  if (i < 0) return null;
  const next = places[i] as Place | undefined;
  const prev = places[i - 1] as Place | undefined;
  if (layout === "rows") {
    const at = next ? next.top : prev!.bottom;
    const { left, right } = next ?? prev!;
    return { left, right, top: at - 1, bottom: at + 1 };
  }
  const endOfRow = !next || (prev !== undefined && prev.top !== next.top && y < next.top);
  const card = endOfRow ? prev! : next;
  const at = endOfRow ? card.right + gap / 2 : card.left - gap / 2;
  return { left: at - 1, right: at + 1, top: card.top, bottom: card.bottom };
}

/** What the hosts page lends the pointer. Each is read at the time, so it can change between presses. */
export interface PointerSite {
  list(): HTMLElement | null;
  layout(): Layout;
  /** The saved hosts listed, in order: quick connect rows are neither boxed nor moved. */
  hostIds(): string[];
  selected(): string[];
  /** Hosts can be moved: not while a search ranks them. */
  canMove(): boolean;
  /** Ctrl+click is ⌘-click. */
  mac(): boolean;
  /** The selection the box makes, as the box changes. */
  boxSelect(ids: string[]): void;
  /** Selects the pressed host alone as it starts to move, when it was not selected. */
  pick(id: string): void;
  drop(moving: string[], before: string | null): void;
  /** Class names of the box, the drop line, the host that follows the pointer and its count. */
  classes: { box: string; line: string; ghost: string; count: string };
}

interface Press {
  pointerId: number;
  kind: "box" | "move";
  /** Where the press was, in the list's content. */
  origin: { x: number; y: number };
  /** Where the pointer is, in the viewport. */
  client: { x: number; y: number };
  started: boolean;
  /** The pressed host, for a move. */
  el: HTMLElement | null;
  mode: BoxMode;
  /** The selection the press began with. */
  start: string[];
  shown: string[];
  moving: string[];
  before: string | null;
  /** Where the pressed host was under the pointer, so it follows from there. */
  grab: { x: number; y: number };
  overlays: HTMLElement[];
  ghost: HTMLElement | null;
  dimmed: HTMLElement[];
  frame: number;
}

/** A point of the viewport in the list's content. */
function contentPoint(list: HTMLElement, x: number, y: number): { x: number; y: number } {
  const r = list.getBoundingClientRect();
  return { x: x - r.left - list.clientLeft + list.scrollLeft, y: y - r.top - list.clientTop + list.scrollTop };
}

/** The part of the list's content that shows: inside its edges and below its sticky head, if it has one. */
function shownPart(list: HTMLElement): Box {
  const head = Array.from(list.children).find((el) => getComputedStyle(el).position === "sticky");
  const covered = head ? head.getBoundingClientRect().bottom - list.getBoundingClientRect().top - list.clientTop : 0;
  return {
    left: list.scrollLeft,
    top: list.scrollTop + Math.max(0, covered),
    right: list.scrollLeft + list.clientWidth,
    bottom: list.scrollTop + list.clientHeight,
  };
}

/**
 * The click that the release of a press ended with Escape makes must not act either: beside the hosts it would select
 * none. A press that begins first means the release was missed, outside the window.
 */
function swallowRelease(pointerId: number) {
  const done = () => {
    window.removeEventListener("pointerup", up, true);
    window.removeEventListener("pointerdown", done, true);
  };
  const up = (e: PointerEvent) => {
    if (e.pointerId !== pointerId) return;
    done();
    swallowClick();
  };
  window.addEventListener("pointerup", up, true);
  window.addEventListener("pointerdown", done, true);
}

function place(el: HTMLElement, box: Box) {
  el.style.left = `${box.left}px`;
  el.style.top = `${box.top}px`;
  el.style.width = `${box.right - box.left}px`;
  el.style.height = `${box.bottom - box.top}px`;
}

/**
 * Drawing a box from beside the hosts selects the hosts it touches, and dragging a host moves it, or every selected
 * host when it is one of them, to where it is let go (HOST-16). Both take no part in HTML drag and drop, which Tauri's
 * file drop takes over on Windows; the list scrolls while the pointer is near its edge, and Escape ends either without
 * a change.
 */
export class HostPointer {
  private press: Press | null = null;

  constructor(private site: PointerSite) {}

  /** After each render of the page, which the next press and move read. */
  rendered(site: PointerSite) {
    this.site = site;
  }

  /** `pointerdown` in the list. */
  down = (e: ReactPointerEvent<HTMLElement>) => {
    if (e.button !== 0 || e.pointerType === "touch" || this.press) return;
    const target = e.target as Element;
    if (target.closest("button, a, input, textarea, select, [role='button']")) return;
    const list = this.site.list();
    if (!list) return;
    const option = target.closest<HTMLElement>('[role="option"][data-id]');
    if (option) {
      const modified = e.shiftKey || e.altKey || e.ctrlKey || e.metaKey;
      if (modified || !this.site.canMove() || !this.site.hostIds().includes(option.dataset.id!)) return;
    }
    const add = this.site.mac() ? e.metaKey : e.ctrlKey;
    const r = option?.getBoundingClientRect();
    this.press = {
      pointerId: e.pointerId,
      kind: option ? "move" : "box",
      origin: contentPoint(list, e.clientX, e.clientY),
      client: { x: e.clientX, y: e.clientY },
      started: false,
      el: option,
      mode: e.shiftKey ? "add" : add ? "toggle" : "replace",
      start: this.site.selected(),
      shown: [],
      moving: [],
      before: null,
      grab: r ? { x: e.clientX - r.left, y: e.clientY - r.top } : { x: 0, y: 0 },
      overlays: [],
      ghost: null,
      dimmed: [],
      frame: 0,
    };
    window.addEventListener("pointermove", this.move);
    window.addEventListener("pointerup", this.up);
    window.addEventListener("pointercancel", this.cancel);
    window.addEventListener("keydown", this.key, true);
    window.addEventListener("blur", this.cancel);
    window.addEventListener("scroll", this.scrolled, true);
  };

  private move = (e: PointerEvent) => {
    const p = this.press;
    if (!p || e.pointerId !== p.pointerId) return;
    if (!p.started) {
      if (Math.hypot(e.clientX - p.client.x, e.clientY - p.client.y) < DRAG_THRESHOLD) return;
      if (!this.start(p)) {
        this.cancel();
        return;
      }
    }
    p.client = { x: e.clientX, y: e.clientY };
    this.update(p);
  };

  /** Lifts the pressed hosts, or begins the box; false when the list is gone. */
  private start(p: Press): boolean {
    const list = this.site.list();
    if (!list) return false;
    p.started = true;
    p.shown = this.site.hostIds();
    const { classes } = this.site;
    const overlay = (className: string) => {
      const el = document.createElement("div");
      el.className = className;
      el.setAttribute("aria-hidden", "true");
      list.appendChild(el);
      p.overlays.push(el);
      return el;
    };
    if (p.kind === "box") {
      overlay(classes.box);
    } else if (p.el) {
      const id = p.el.dataset.id!;
      const selected = new Set(p.start);
      if (selected.has(id)) {
        p.moving = p.shown.filter((x) => selected.has(x));
      } else {
        p.moving = [id];
        this.site.pick(id);
      }
      const moving = new Set(p.moving);
      for (const el of this.elements(list)) {
        if (!moving.has(el.dataset.id!)) continue;
        el.dataset.dragging = "";
        p.dimmed.push(el);
      }
      p.ghost = this.ghost(p.el, p.dimmed, p.moving.length);
      overlay(classes.line).hidden = true;
    }
    try {
      list.setPointerCapture(p.pointerId);
    } catch {
      // The pointer is gone; the window's listeners still end the press.
    }
    p.frame = requestAnimationFrame(this.scroll);
    return true;
  }

  /** The listed hosts' elements, in order. */
  private elements(list: HTMLElement): HTMLElement[] {
    const ids = new Set(this.site.hostIds());
    return Array.from(list.querySelectorAll<HTMLElement>('[role="option"][data-id]')).filter((el) =>
      ids.has(el.dataset.id!),
    );
  }

  /** Where the listed hosts are now: readings may have sorted them again since the press. */
  private places(list: HTMLElement, leaveOut: Set<string>): Place[] {
    const r = list.getBoundingClientRect();
    const dx = list.scrollLeft - r.left - list.clientLeft;
    const dy = list.scrollTop - r.top - list.clientTop;
    return this.elements(list)
      .filter((el) => !leaveOut.has(el.dataset.id!))
      .map((el) => {
        const b = el.getBoundingClientRect();
        return { id: el.dataset.id!, left: b.left + dx, top: b.top + dy, right: b.right + dx, bottom: b.bottom + dy };
      });
  }

  /**
   * Copies of the moving hosts that follow the pointer, the pressed one first: rows one under another, up to
   * {@link GHOST_ROWS}, and cards stacked behind it, up to {@link GHOST_CARDS}. With several, their number too.
   */
  private ghost(pressed: HTMLElement, moving: HTMLElement[], count: number): HTMLElement {
    const { classes } = this.site;
    const layout = this.site.layout();
    const ghost = document.createElement("div");
    ghost.className = classes.ghost;
    ghost.dataset.stack = layout;
    ghost.setAttribute("aria-hidden", "true");
    const r = pressed.getBoundingClientRect();
    ghost.style.width = `${r.width}px`;
    if (layout === "grid") ghost.style.height = `${r.height}px`;
    const els = [pressed, ...moving.filter((el) => el !== pressed)].slice(
      0,
      layout === "rows" ? GHOST_ROWS : GHOST_CARDS,
    );
    const copies = els.map((el, i) => {
      const r = el.getBoundingClientRect();
      const copy = el.cloneNode(true) as HTMLElement;
      copy.removeAttribute("data-id");
      copy.removeAttribute("role");
      delete copy.dataset.dragging;
      copy.style.width = `${r.width}px`;
      copy.style.height = `${r.height}px`;
      if (layout === "grid") copy.style.transform = `translate(${i * STACK_STEP}px, ${i * STACK_STEP}px)`;
      return copy;
    });
    // Cards behind come first, so the pressed one paints over them.
    ghost.append(...(layout === "grid" ? copies.reverse() : copies));
    if (count > 1) {
      const badge = document.createElement("span");
      badge.className = classes.count;
      badge.textContent = String(count);
      ghost.appendChild(badge);
    }
    document.body.appendChild(ghost);
    return ghost;
  }

  private update(p: Press) {
    const list = this.site.list();
    if (!list) return;
    const at = contentPoint(list, p.client.x, p.client.y);
    if (p.kind === "box") {
      const box = {
        left: Math.min(p.origin.x, at.x),
        top: Math.min(p.origin.y, at.y),
        right: Math.max(p.origin.x, at.x),
        bottom: Math.max(p.origin.y, at.y),
      };
      // Only the part that shows is drawn, so the box never widens the list; the hosts it touches still count.
      const drawn = clipBox(box, shownPart(list));
      const el = p.overlays[0];
      el.hidden = !drawn;
      if (drawn) place(el, drawn);
      const ids = boxSelection(p.start, hostsInBox(this.places(list, new Set()), box), p.mode);
      const now = this.site.selected();
      if (ids.length !== now.length || ids.some((id, i) => id !== now[i])) this.site.boxSelect(ids);
      return;
    }
    if (p.ghost) p.ghost.style.transform = `translate(${p.client.x - p.grab.x}px, ${p.client.y - p.grab.y}px)`;
    const layout = this.site.layout();
    const places = this.places(list, new Set(p.moving));
    p.before = dropBefore(places, at.x, at.y, layout);
    const first = places.length > 0 ? list.querySelector(`[data-id="${CSS.escape(places[0].id)}"]`) : null;
    const gap = first?.parentElement ? parseFloat(getComputedStyle(first.parentElement).columnGap) || 0 : 0;
    const line = dropLine(places, p.before, at.y, layout, gap);
    const el = p.overlays[0];
    el.hidden = !line;
    if (line) place(el, line);
  }

  /** Scrolls the list while the pointer is near its top or bottom edge, a frame at a time. */
  private scroll = () => {
    const p = this.press;
    const list = this.site.list();
    if (!p?.started || !list) return;
    const r = list.getBoundingClientRect();
    const y = p.client.y;
    const speed = (into: number) => Math.min(MAX_SCROLL, Math.ceil((into / EDGE) * MAX_SCROLL));
    const dy = y < r.top + EDGE ? -speed(r.top + EDGE - y) : y > r.bottom - EDGE ? speed(y - (r.bottom - EDGE)) : 0;
    if (dy !== 0) {
      const top = list.scrollTop;
      list.scrollTop = top + dy;
      // At once, as a release before the scroll event must drop where the line now shows.
      if (list.scrollTop !== top) this.update(p);
    }
    p.frame = requestAnimationFrame(this.scroll);
  };

  /** The list scrolled, also by the wheel: what the press shows moves with the pointer. */
  private scrolled = () => {
    if (this.press?.started) this.update(this.press);
  };

  private up = (e: PointerEvent) => {
    const p = this.press;
    if (!p || e.pointerId !== p.pointerId) return;
    this.end(p);
    if (!p.started) return;
    swallowClick();
    if (p.kind === "move") this.site.drop(p.moving, p.before);
  };

  private key = (e: KeyboardEvent) => {
    const p = this.press;
    if (e.key !== "Escape" || !p?.started) return;
    e.preventDefault();
    e.stopPropagation();
    this.cancel();
    swallowRelease(p.pointerId);
  };

  /** Ends the press without a change: a box gives back the selection it began with. */
  cancel = () => {
    const p = this.press;
    if (!p) return;
    this.end(p);
    if (p.started && p.kind === "box") this.site.boxSelect(p.start);
  };

  /** Stops listening to the pointer and takes away what the press showed. */
  private end(p: Press) {
    this.press = null;
    window.removeEventListener("pointermove", this.move);
    window.removeEventListener("pointerup", this.up);
    window.removeEventListener("pointercancel", this.cancel);
    window.removeEventListener("keydown", this.key, true);
    window.removeEventListener("blur", this.cancel);
    window.removeEventListener("scroll", this.scrolled, true);
    cancelAnimationFrame(p.frame);
    for (const el of p.overlays) el.remove();
    p.ghost?.remove();
    for (const el of p.dimmed) delete el.dataset.dragging;
    try {
      this.site.list()?.releasePointerCapture(p.pointerId);
    } catch {
      // Already released.
    }
  }
}

/** The hosts page's box selection and moves; pass its `down` to the list's `onPointerDown`. */
export function useHostPointer(site: PointerSite): HostPointer {
  const [pointer] = useState(() => new HostPointer(site));
  useLayoutEffect(() => pointer.rendered(site));
  useEffect(() => () => pointer.cancel(), [pointer]);
  return pointer;
}
