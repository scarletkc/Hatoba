import { useEffect } from "react";
import { create } from "zustand";
import s from "./ZoomHud.module.css";

interface Hud {
  text: string;
  /** The center of the part that changed, in CSS pixels; null centers it in the window. */
  at: { x: number; y: number } | null;
  /** Bumped on every change, so the same value shown twice stays up for the full time again. */
  tick: number;
}

const useHud = create<{ hud: Hud | null }>(() => ({ hud: null }));

const SHOWN_MS = 900;

/** Shows the size a zoom key or Ctrl+wheel set (TERM-13), at the center of `box`, as browsers do. */
export function showZoomHud(text: string, box: DOMRect | null) {
  const at = box ? { x: box.left + box.width / 2, y: box.top + box.height / 2 } : null;
  useHud.setState((st) => ({ hud: { text, at, tick: (st.hud?.tick ?? 0) + 1 } }));
}

export function ZoomHud() {
  const hud = useHud((st) => st.hud);
  useEffect(() => {
    if (!hud) return;
    const timer = window.setTimeout(() => useHud.setState({ hud: null }), SHOWN_MS);
    return () => window.clearTimeout(timer);
  }, [hud]);
  if (!hud) return null;
  return (
    <div className={s.hud} role="status" style={hud.at ? { left: hud.at.x, top: hud.at.y } : undefined}>
      {hud.text}
    </div>
  );
}
