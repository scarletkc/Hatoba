import type { KeyboardEvent, PointerEvent } from "react";
import { cx } from "@/lib/cx";
import s from "./Sparkline.module.css";

/** Room at each edge so the end dot and its ring are not clipped. */
const PAD = 5;

export interface SparkSeries {
  /** Oldest first, one per reading; `null` leaves a gap. */
  values: (number | null)[];
  /** Sets `--spark-line` and `--spark-dot` for the series. */
  className: string;
}

/**
 * A small line chart of the latest readings on a window of `slots` positions, the newest at the
 * right edge (TERM-12). Pointing at it or moving along it with the arrow keys picks a reading,
 * which the caller shows in place of the current value.
 */
export function Sparkline({
  series,
  slots,
  max,
  width,
  height,
  label,
  picked,
  onPick,
}: {
  series: SparkSeries[];
  slots: number;
  /** The value at the top edge; the bottom edge is 0. */
  max: number;
  width: number;
  height: number;
  label: string;
  picked: number | null;
  onPick(index: number | null): void;
}) {
  const count = Math.max(0, ...series.map((x) => x.values.length));
  const step = (width - 2 * PAD) / Math.max(1, slots - 1);
  const x = (i: number) => width - PAD - (count - 1 - i) * step;
  const y = (v: number) => height - PAD - (Math.min(v, max) / (max || 1)) * (height - 2 * PAD);
  const last = count - 1;
  const at = picked ?? last;

  const path = (values: (number | null)[]) => {
    let d = "";
    let pen = false;
    values.forEach((v, i) => {
      if (v == null) {
        pen = false;
        return;
      }
      d += `${pen ? "L" : "M"}${x(i).toFixed(1)},${y(v).toFixed(1)}`;
      pen = true;
    });
    return d;
  };

  const pickAt = (e: PointerEvent<SVGSVGElement>) => {
    if (count === 0) return;
    const px = e.clientX - e.currentTarget.getBoundingClientRect().left;
    const i = Math.round(last - (width - PAD - px) / step);
    onPick(Math.max(0, Math.min(last, i)));
  };

  const onKey = (e: KeyboardEvent<SVGSVGElement>) => {
    if (count === 0) return;
    const next =
      e.key === "ArrowLeft" ? at - 1 : e.key === "ArrowRight" ? at + 1 : e.key === "Home" ? 0 : e.key === "End" ? last : null;
    if (next == null) return;
    e.preventDefault();
    onPick(Math.max(0, Math.min(last, next)));
  };

  return (
    <svg
      className={s.chart}
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={label}
      tabIndex={0}
      onPointerMove={pickAt}
      onPointerDown={pickAt}
      onPointerLeave={() => onPick(null)}
      onFocus={() => onPick(last >= 0 ? last : null)}
      onBlur={() => onPick(null)}
      onKeyDown={onKey}
    >
      <line className={s.baseline} x1={PAD} x2={width - PAD} y1={height - PAD + 0.5} y2={height - PAD + 0.5} />
      {picked != null && <line className={s.hairline} x1={x(picked)} x2={x(picked)} y1={PAD - 2} y2={height - PAD} />}
      {series.map((sr, k) => (
        <path key={k} className={cx(s.line, sr.className)} d={path(sr.values)} />
      ))}
      {at >= 0 &&
        series.map((sr, k) => {
          const v = sr.values[at];
          return v == null ? null : <circle key={k} className={cx(s.dot, sr.className)} cx={x(at)} cy={y(v)} r={3.5} />;
        })}
    </svg>
  );
}
