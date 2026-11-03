import { contrast, type TermMode } from "@/session/schemes";

/** WCAG AA for text. */
const AA = 4.5;

export interface BarColors {
  background: string;
  /** The status, the target and the figures of the resource usage. */
  text: string;
  /** Labels, the latency and the buttons. */
  secondary: string;
  /** Disabled buttons, and the dot of a closed session. */
  muted: string;
}

function channels(hex: string): [number, number, number] | null {
  const m = /^#([0-9a-f]{6})$/i.exec(hex.trim());
  if (!m) return null;
  const n = Number.parseInt(m[1], 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** `#RRGGBB` that is `amount` of `a` over `b`. */
function mix(a: [number, number, number], b: [number, number, number], amount: number): string {
  return `#${a
    .map((c, i) =>
      Math.round(c * amount + b[i] * (1 - amount))
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}

/**
 * The colors of the session status bar, which sits on the terminal's background (TERM-06): the scheme's
 * foreground, taken toward white or black while it reads below AA; a secondary one, as faint as AA
 * allows; and a muted one for what is disabled.
 */
export function barColors(background: string, foreground: string, mode: TermMode): BarColors {
  const bg = channels(background);
  const fg = channels(foreground);
  if (!bg || !fg) {
    return {
      background,
      text: foreground,
      secondary: `color-mix(in srgb, ${foreground} 72%, ${background})`,
      muted: `color-mix(in srgb, ${foreground} 42%, ${background})`,
    };
  }
  const edge: [number, number, number] = mode === "dark" ? [255, 255, 255] : [0, 0, 0];
  let text = mix(fg, fg, 1);
  for (let t = 0.05; contrast(text, background) < AA && t <= 1; t += 0.05) text = mix(edge, fg, t);
  const strong = channels(text)!;
  let amount = 0.72;
  while (amount < 1 && contrast(mix(strong, bg, amount), background) < AA) amount += 0.02;
  return {
    background: mix(bg, bg, 1),
    text,
    secondary: mix(strong, bg, Math.min(amount, 1)),
    muted: mix(strong, bg, 0.42),
  };
}
