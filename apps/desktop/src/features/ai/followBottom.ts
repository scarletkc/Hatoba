import { useCallback, useLayoutEffect, useMemo, useRef, type KeyboardEvent, type RefObject, type WheelEvent } from "react";

/** How near the bottom, in pixels, a scroll down has to end for the list to follow new content again. */
const NEAR_BOTTOM = 48;
/** Keys that scroll the list up while focus is in it, outside a text field. */
const UP_KEYS = new Set(["ArrowUp", "PageUp", "Home"]);

const gapBelow = (el: HTMLElement) => el.scrollHeight - el.scrollTop - el.clientHeight;

const isTextField = (target: EventTarget | null) =>
  target instanceof HTMLElement && (target.isContentEditable || ["INPUT", "TEXTAREA", "SELECT"].includes(target.tagName));

/** Whether turning the wheel up over `target` scrolls `list`, not a scroller inside it that can still go up. */
function wheelScrollsList(list: HTMLElement, target: EventTarget | null): boolean {
  for (let el = target instanceof Element ? target : null; el && el !== list; el = el.parentElement) {
    if (el.scrollTop > 0 && /auto|scroll/.test(getComputedStyle(el).overflowY)) return false;
  }
  return list.scrollTop > 0;
}

export interface FollowBottom {
  onScroll(): void;
  onWheel(e: WheelEvent): void;
  onKeyDown(e: KeyboardEvent): void;
  /** Follows again from the bottom, as when the user sends a message. */
  stick(): void;
  /** Stops following, as when something else scrolls the list to an entry. */
  release(): void;
}

/**
 * Keeps a chat list at its bottom while new content comes in, and lets go the moment the user scrolls
 * up: a wheel turn or a key in its own event, before the next piece of a reply can pull the list back,
 * and any other move up (the scrollbar, a touch) when the list next scrolls or grows, whichever comes
 * first. Only a scroll back down near the bottom follows again. While `live`, content that grows on its
 * own (a diff that loads into an approval card) is followed too; otherwise only what a render adds and
 * changes to the list's own size are, so a block the user opens at the bottom opens downward.
 */
export function useFollowBottom(ref: RefObject<HTMLElement | null>, live: boolean): FollowBottom {
  const follow = useRef(true);
  /** Where the list was last scrolled to, by the user or by following. */
  const top = useRef(0);
  const liveNow = useRef(live);

  const toBottom = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    top.current = el.scrollTop;
  }, [ref]);

  // A move up whose scroll event has not arrived yet lets go here rather than being undone.
  const pin = useCallback(() => {
    const el = ref.current;
    if (!el || !follow.current) return;
    if (el.scrollTop < top.current && gapBelow(el) > 1) follow.current = false;
    else toBottom();
  }, [ref, toBottom]);

  const watched = useRef<{ observer: ResizeObserver; nodes: Set<Element> } | null>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      if (liveNow.current || entries.some((e) => e.target === el)) pin();
    });
    observer.observe(el);
    watched.current = { observer, nodes: new Set() };
    return () => {
      observer.disconnect();
      watched.current = null;
    };
  }, [ref, pin]);

  // Every render: follow what it added, and watch the list's children for what grows after it.
  useLayoutEffect(() => {
    liveNow.current = live;
    pin();
    const el = ref.current;
    const w = watched.current;
    if (!el || !w) return;
    for (const node of w.nodes) {
      if (node.parentElement === el) continue;
      w.observer.unobserve(node);
      w.nodes.delete(node);
    }
    for (const node of el.children) {
      if (w.nodes.has(node)) continue;
      w.observer.observe(node);
      w.nodes.add(node);
    }
  });

  return useMemo(
    () => ({
      onScroll() {
        const el = ref.current;
        if (!el) return;
        const gap = gapBelow(el);
        if (el.scrollTop < top.current && gap > 1) follow.current = false;
        else if (gap < NEAR_BOTTOM) follow.current = true;
        top.current = el.scrollTop;
      },
      onWheel(e) {
        const el = ref.current;
        if (el && e.deltaY < 0 && !e.ctrlKey && wheelScrollsList(el, e.target)) follow.current = false;
      },
      onKeyDown(e) {
        const el = ref.current;
        if (el && UP_KEYS.has(e.key) && !e.defaultPrevented && !isTextField(e.target) && el.scrollTop > 0) follow.current = false;
      },
      stick() {
        follow.current = true;
        toBottom();
      },
      release() {
        follow.current = false;
      },
    }),
    [ref, toBottom],
  );
}
