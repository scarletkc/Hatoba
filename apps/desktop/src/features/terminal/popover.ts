import { useEffect, useLayoutEffect, useState, type RefObject } from "react";
import { useOverlaysLocked, type MenuAnchor } from "@/components/overlay";

/**
 * Placement and dismissal of a status bar popover: below its button and kept inside the window
 * (`Menu`'s rule), re-measured when anything in `measure` changes, and closed by a click outside it,
 * Escape, the window losing focus, or the vault locking. A click on an element matching `trigger`
 * is left to the button, which toggles the popover itself. `onClose` gets `true` when closed from
 * the keyboard, so typing can continue in the terminal.
 */
export function useStatusPopover(
  ref: RefObject<HTMLElement | null>,
  anchor: MenuAnchor,
  onClose: (restoreFocus: boolean) => void,
  trigger: string,
  measure: readonly unknown[],
): { pos: { left: number; top: number }; locked: boolean } {
  const [pos, setPos] = useState({ left: anchor.x, top: anchor.y });
  const locked = useOverlaysLocked();

  useEffect(() => {
    if (locked) onClose(false);
  }, [locked, onClose]);

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    let left = anchor.alignRight ? anchor.x - r.width : anchor.x;
    let top = anchor.y;
    left = Math.max(8, Math.min(left, window.innerWidth - r.width - 8));
    if (top + r.height > window.innerHeight - 8) top = Math.max(8, anchor.y - r.height - 8);
    setPos({ left, top });
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `measure` is the caller's list of size inputs
  }, [ref, anchor.x, anchor.y, anchor.alignRight, ...measure]);

  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      const target = e.target as Element;
      if (ref.current && !ref.current.contains(target) && !target.closest?.(trigger)) onClose(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        onClose(true);
      }
    };
    const close = () => onClose(false);
    window.addEventListener("mousedown", onDown, true);
    window.addEventListener("keydown", onKey, true);
    window.addEventListener("blur", close);
    return () => {
      window.removeEventListener("mousedown", onDown, true);
      window.removeEventListener("keydown", onKey, true);
      window.removeEventListener("blur", close);
    };
  }, [ref, onClose, trigger]);

  return { pos, locked };
}
