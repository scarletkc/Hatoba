import { useId } from "react";
import { cx } from "@/lib/cx";
import s from "./AppLogo.module.css";

/** The app icon (`src-tauri/app-icon.svg`). `className` sets its size. */
export function AppLogo({ className }: { className?: string }) {
  return <div aria-hidden className={cx(s.appLogo, className)} />;
}

/**
 * A swell of `periods` sine waves across a 1×1 box, its surface between `top` and `top + depth`, filled
 * down to the bottom. `shift` moves it right by that part of a period. Each half wave is the usual cubic
 * for a cosine.
 */
function swell(periods: number, top: number, depth: number, shift = 0, closed = true): string {
  const half = 1 / periods / 2;
  const n = (v: number) => +v.toFixed(4);
  const start = (shift - 1) * 2 * half;
  let d = `M${n(start)} ${top}`;
  for (let x = start, crest = true; x < 1; x += half, crest = !crest) {
    const [from, to] = crest ? [top, top + depth] : [top + depth, top];
    d += `C${n(x + 0.3642 * half)} ${n(from)} ${n(x + 0.6358 * half)} ${n(to)} ${n(x + half)} ${n(to)}`;
  }
  return closed ? `${d}V1H${n(start)}Z` : d;
}

// Two periods across the window, as on the icon, with a second swell behind, half a period over.
const FRONT = swell(2, 0.34, 0.16);
const SURFACE = swell(2, 0.34, 0.16, 0, false);
const BACK = swell(2, 0.1, 0.2, 0.5);

/**
 * The first-launch and lock screens' backdrop, drawn from the app icon: its sky over a calm sea at the
 * bottom of the window, textured with the icon's seigaiha. It sits behind everything else in its
 * screen, which must be a stacking context.
 */
export function BrandBackdrop() {
  const id = useId().replace(/[^\w-]/g, "");
  return (
    <div aria-hidden className={s.backdrop}>
      <svg className={s.sea}>
        <defs>
          <linearGradient id={`${id}-water`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" className={s.waterTop} />
            <stop offset="1" className={s.waterBottom} />
          </linearGradient>
          {/* The scales show near the surface and fade out with depth, clear of text at the bottom. */}
          <linearGradient id={`${id}-depth`} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0.3" className={s.scalesTop} />
            <stop offset="0.9" className={s.scalesBottom} />
          </linearGradient>
          <clipPath id={`${id}-front`} clipPathUnits="objectBoundingBox">
            <path d={FRONT} />
          </clipPath>
          {/* The icon's tile at a little over a third of its size: each row of scales covers the one above. */}
          <pattern id={`${id}-scales`} width="40" height="20" patternUnits="userSpaceOnUse">
            {[
              [20, -10],
              [0, 0],
              [40, 0],
              [20, 10],
              [0, 20],
              [40, 20],
              [20, 30],
            ].map(([x, y]) => (
              <g key={`${x},${y}`} transform={`translate(${x} ${y})`} stroke="#ffffff" strokeWidth="1">
                <circle r="20" fill="#000000" />
                <circle r="14.4" fill="none" />
                <circle r="8.8" fill="none" />
                <circle r="3.2" fill="none" />
              </g>
            ))}
          </pattern>
          <mask id={`${id}-waves`}>
            <rect width="100%" height="100%" fill={`url(#${id}-scales)`} />
          </mask>
        </defs>
        <svg viewBox="0 0 1 1" preserveAspectRatio="none" width="100%" height="100%">
          <path d={BACK} className={s.back} />
          <path d={FRONT} fill={`url(#${id}-water)`} />
        </svg>
        <rect
          width="100%"
          height="100%"
          fill={`url(#${id}-depth)`}
          clipPath={`url(#${id}-front)`}
          mask={`url(#${id}-waves)`}
        />
        <svg viewBox="0 0 1 1" preserveAspectRatio="none" width="100%" height="100%">
          <path d={SURFACE} className={s.surface} vectorEffect="non-scaling-stroke" />
        </svg>
      </svg>
    </div>
  );
}
