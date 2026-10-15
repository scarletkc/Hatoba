import type { ITheme } from "@xterm/xterm";

export type TermMode = "dark" | "light";

/** A full terminal palette: xterm's theme with every colour a preview draws set. */
export type SchemeColors = ITheme &
  Required<
    Pick<
      ITheme,
      | "background"
      | "foreground"
      | "cursor"
      | "selectionBackground"
      | "black"
      | "red"
      | "green"
      | "yellow"
      | "blue"
      | "magenta"
      | "cyan"
      | "white"
      | "brightBlack"
      | "brightRed"
      | "brightGreen"
      | "brightYellow"
      | "brightBlue"
      | "brightMagenta"
      | "brightCyan"
      | "brightWhite"
    >
  >;

/** Search-match highlight colours (xterm needs #RRGGBB here). */
export interface SearchColors {
  matchBackground: string;
  matchBorder: string;
  matchOverviewRuler: string;
  activeMatchBackground: string;
  activeMatchBorder: string;
  activeMatchColorOverviewRuler: string;
}

export interface TermScheme {
  /** Stored in the synced settings (`dark_scheme` / `light_scheme`); never rename one. */
  id: string;
  /** The scheme's own name, the same in every language. */
  name: string;
  mode: TermMode;
  colors: SchemeColors;
  /**
   * xterm's `minimumContrastRatio`. Light schemes raise it so programs written for dark terminals
   * (white or yellow text) stay readable.
   */
  contrast: number;
  /** Fixed search highlights; derived from the palette when absent. */
  search?: SearchColors;
}

/** What a mode uses until a scheme is picked for it, and for an id this build does not know. */
export const DEFAULT_SCHEME: Record<TermMode, string> = { dark: "hatoba-dark", light: "hatoba-light" };

const LIGHT_CONTRAST = 3;

/*
 * Palettes from each scheme's own terminal ports, all MIT licensed: Dracula (dracula/dracula-theme),
 * Tokyo Night (enkia/tokyo-night-vscode-theme), Catppuccin (catppuccin/catppuccin), One Half
 * (sonph/onehalf), Nord (nordtheme/nord), Gruvbox (morhetz/gruvbox), Solarized (altercation/solarized,
 * as iTerm2 bundles it), Rosé Pine (rose-pine/rose-pine-theme), Kanagawa (rebelot/kanagawa.nvim) and
 * GitHub (primer/github-vscode-theme). Selection colours are chosen to keep text readable in xterm,
 * which draws the selection under the text.
 */
export const SCHEMES: readonly TermScheme[] = [
  {
    id: "hatoba-dark",
    name: "Hatoba Dark",
    mode: "dark",
    contrast: 1,
    colors: {
      // The terminal itself uses the app's `--term` token (terminalTheme).
      background: "#0d0e10",
      foreground: "#d5d8de",
      cursor: "#d5d8de",
      selectionBackground: "rgba(122, 183, 255, 0.3)",
      selectionInactiveBackground: "rgba(122, 183, 255, 0.18)",
      black: "#2b2e36",
      red: "#f07178",
      green: "#8bd49c",
      yellow: "#e5c07b",
      blue: "#7ab7ff",
      magenta: "#c792ea",
      cyan: "#7fd1d6",
      white: "#d5d8de",
      brightBlack: "#6b717c",
      brightRed: "#ff8b92",
      brightGreen: "#a4e5b3",
      brightYellow: "#f0d08f",
      brightBlue: "#9cc9ff",
      brightMagenta: "#d7a8f0",
      brightCyan: "#9fe3e7",
      brightWhite: "#ffffff",
    },
    search: {
      matchBackground: "#4a3f1d",
      matchBorder: "#7a6a2e",
      matchOverviewRuler: "#e5c07b",
      activeMatchBackground: "#8a6d1f",
      activeMatchBorder: "#e5c07b",
      activeMatchColorOverviewRuler: "#e5c07b",
    },
  },
  {
    id: "dracula",
    name: "Dracula",
    mode: "dark",
    contrast: 1,
    colors: {
      background: "#282a36",
      foreground: "#f8f8f2",
      cursor: "#f8f8f2",
      selectionBackground: "#44475a",
      black: "#21222c",
      red: "#ff5555",
      green: "#50fa7b",
      yellow: "#f1fa8c",
      blue: "#bd93f9",
      magenta: "#ff79c6",
      cyan: "#8be9fd",
      white: "#f8f8f2",
      brightBlack: "#6272a4",
      brightRed: "#ff6e6e",
      brightGreen: "#69ff94",
      brightYellow: "#ffffa5",
      brightBlue: "#d6acff",
      brightMagenta: "#ff92df",
      brightCyan: "#a4ffff",
      brightWhite: "#ffffff",
    },
  },
  {
    id: "tokyo-night",
    name: "Tokyo Night",
    mode: "dark",
    contrast: 1,
    colors: {
      background: "#1a1b26",
      foreground: "#c0caf5",
      cursor: "#c0caf5",
      selectionBackground: "#283457",
      black: "#15161e",
      red: "#f7768e",
      green: "#9ece6a",
      yellow: "#e0af68",
      blue: "#7aa2f7",
      magenta: "#bb9af7",
      cyan: "#7dcfff",
      white: "#a9b1d6",
      brightBlack: "#414868",
      brightRed: "#f7768e",
      brightGreen: "#9ece6a",
      brightYellow: "#e0af68",
      brightBlue: "#7aa2f7",
      brightMagenta: "#bb9af7",
      brightCyan: "#7dcfff",
      brightWhite: "#c0caf5",
    },
  },
  {
    id: "catppuccin-mocha",
    name: "Catppuccin Mocha",
    mode: "dark",
    contrast: 1,
    colors: {
      background: "#1e1e2e",
      foreground: "#cdd6f4",
      cursor: "#f5e0dc",
      selectionBackground: "rgba(147, 153, 178, 0.3)",
      black: "#45475a",
      red: "#f38ba8",
      green: "#a6e3a1",
      yellow: "#f9e2af",
      blue: "#89b4fa",
      magenta: "#f5c2e7",
      cyan: "#94e2d5",
      white: "#bac2de",
      brightBlack: "#585b70",
      brightRed: "#f38ba8",
      brightGreen: "#a6e3a1",
      brightYellow: "#f9e2af",
      brightBlue: "#89b4fa",
      brightMagenta: "#f5c2e7",
      brightCyan: "#94e2d5",
      brightWhite: "#a6adc8",
    },
  },
  {
    id: "one-half-dark",
    name: "One Half Dark",
    mode: "dark",
    contrast: 1,
    colors: {
      background: "#282c34",
      foreground: "#dcdfe4",
      cursor: "#a3b3cc",
      selectionBackground: "#474e5d",
      black: "#282c34",
      red: "#e06c75",
      green: "#98c379",
      yellow: "#e5c07b",
      blue: "#61afef",
      magenta: "#c678dd",
      cyan: "#56b6c2",
      white: "#dcdfe4",
      brightBlack: "#5d677a",
      brightRed: "#e06c75",
      brightGreen: "#98c379",
      brightYellow: "#e5c07b",
      brightBlue: "#61afef",
      brightMagenta: "#c678dd",
      brightCyan: "#56b6c2",
      brightWhite: "#dcdfe4",
    },
  },
  {
    id: "nord",
    name: "Nord",
    mode: "dark",
    contrast: 1,
    colors: {
      background: "#2e3440",
      foreground: "#d8dee9",
      cursor: "#d8dee9",
      selectionBackground: "#434c5e",
      black: "#3b4252",
      red: "#bf616a",
      green: "#a3be8c",
      yellow: "#ebcb8b",
      blue: "#81a1c1",
      magenta: "#b48ead",
      cyan: "#88c0d0",
      white: "#e5e9f0",
      brightBlack: "#616e88",
      brightRed: "#bf616a",
      brightGreen: "#a3be8c",
      brightYellow: "#ebcb8b",
      brightBlue: "#81a1c1",
      brightMagenta: "#b48ead",
      brightCyan: "#8fbcbb",
      brightWhite: "#eceff4",
    },
  },
  {
    id: "gruvbox-dark",
    name: "Gruvbox Dark",
    mode: "dark",
    contrast: 1,
    colors: {
      background: "#282828",
      foreground: "#ebdbb2",
      cursor: "#ebdbb2",
      selectionBackground: "#665c54",
      black: "#282828",
      red: "#cc241d",
      green: "#98971a",
      yellow: "#d79921",
      blue: "#458588",
      magenta: "#b16286",
      cyan: "#689d6a",
      white: "#a89984",
      brightBlack: "#928374",
      brightRed: "#fb4934",
      brightGreen: "#b8bb26",
      brightYellow: "#fabd2f",
      brightBlue: "#83a598",
      brightMagenta: "#d3869b",
      brightCyan: "#8ec07c",
      brightWhite: "#ebdbb2",
    },
  },
  {
    id: "solarized-dark",
    name: "Solarized Dark",
    mode: "dark",
    contrast: 1,
    colors: {
      background: "#002b36",
      foreground: "#839496",
      cursor: "#839496",
      selectionBackground: "#073642",
      black: "#073642",
      red: "#dc322f",
      green: "#859900",
      yellow: "#b58900",
      blue: "#268bd2",
      magenta: "#d33682",
      cyan: "#2aa198",
      white: "#eee8d5",
      brightBlack: "#335e69",
      brightRed: "#cb4b16",
      brightGreen: "#586e75",
      brightYellow: "#657b83",
      brightBlue: "#839496",
      brightMagenta: "#6c71c4",
      brightCyan: "#93a1a1",
      brightWhite: "#fdf6e3",
    },
  },
  {
    id: "rose-pine",
    name: "Rosé Pine",
    mode: "dark",
    contrast: 1,
    colors: {
      background: "#191724",
      foreground: "#e0def4",
      cursor: "#524f67",
      cursorAccent: "#e0def4",
      selectionBackground: "#403d52",
      black: "#26233a",
      red: "#eb6f92",
      green: "#31748f",
      yellow: "#f6c177",
      blue: "#9ccfd8",
      magenta: "#c4a7e7",
      cyan: "#ebbcba",
      white: "#e0def4",
      brightBlack: "#6e6a86",
      brightRed: "#eb6f92",
      brightGreen: "#31748f",
      brightYellow: "#f6c177",
      brightBlue: "#9ccfd8",
      brightMagenta: "#c4a7e7",
      brightCyan: "#ebbcba",
      brightWhite: "#e0def4",
    },
  },
  {
    id: "kanagawa",
    name: "Kanagawa",
    mode: "dark",
    contrast: 1,
    colors: {
      background: "#1f1f28",
      foreground: "#dcd7ba",
      cursor: "#c8c093",
      selectionBackground: "#2d4f67",
      black: "#090618",
      red: "#c34043",
      green: "#76946a",
      yellow: "#c0a36e",
      blue: "#7e9cd8",
      magenta: "#957fb8",
      cyan: "#6a9589",
      white: "#c8c093",
      brightBlack: "#727169",
      brightRed: "#e82424",
      brightGreen: "#98bb6c",
      brightYellow: "#e6c384",
      brightBlue: "#7fb4ca",
      brightMagenta: "#938aa9",
      brightCyan: "#7aa89f",
      brightWhite: "#dcd7ba",
    },
  },
  {
    id: "github-dark",
    name: "GitHub Dark",
    mode: "dark",
    contrast: 1,
    colors: {
      background: "#0d1117",
      foreground: "#e6edf3",
      cursor: "#2f81f7",
      selectionBackground: "rgba(56, 139, 253, 0.3)",
      black: "#484f58",
      red: "#ff7b72",
      green: "#3fb950",
      yellow: "#d29922",
      blue: "#58a6ff",
      magenta: "#bc8cff",
      cyan: "#39c5cf",
      white: "#b1bac4",
      brightBlack: "#6e7681",
      brightRed: "#ffa198",
      brightGreen: "#56d364",
      brightYellow: "#e3b341",
      brightBlue: "#79c0ff",
      brightMagenta: "#d2a8ff",
      brightCyan: "#56d4dd",
      brightWhite: "#ffffff",
    },
  },
  {
    id: "hatoba-light",
    name: "Hatoba Light",
    mode: "light",
    contrast: 4.5,
    colors: {
      background: "#ffffff",
      foreground: "#1d1d1f",
      cursor: "#1d1d1f",
      selectionBackground: "rgba(0, 122, 255, 0.25)",
      selectionInactiveBackground: "rgba(0, 122, 255, 0.14)",
      black: "#1d1d1f",
      red: "#c9302c",
      green: "#1f8a3b",
      yellow: "#946200",
      blue: "#0b62d6",
      magenta: "#8a3fb8",
      cyan: "#0f7f89",
      white: "#c9ccd2",
      brightBlack: "#6b717c",
      brightRed: "#e5342b",
      brightGreen: "#28a745",
      brightYellow: "#a87a00",
      brightBlue: "#007aff",
      brightMagenta: "#a24dd6",
      brightCyan: "#12919d",
      brightWhite: "#ffffff",
    },
    search: {
      matchBackground: "#fff1a8",
      matchBorder: "#e0c34a",
      matchOverviewRuler: "#e0b400",
      activeMatchBackground: "#ffd54a",
      activeMatchBorder: "#b88700",
      activeMatchColorOverviewRuler: "#b88700",
    },
  },
  {
    id: "github-light",
    name: "GitHub Light",
    mode: "light",
    contrast: LIGHT_CONTRAST,
    colors: {
      background: "#ffffff",
      foreground: "#1f2328",
      cursor: "#0969da",
      selectionBackground: "rgba(9, 105, 218, 0.2)",
      black: "#24292f",
      red: "#cf222e",
      green: "#116329",
      yellow: "#4d2d00",
      blue: "#0969da",
      magenta: "#8250df",
      cyan: "#1b7c83",
      white: "#6e7781",
      brightBlack: "#57606a",
      brightRed: "#a40e26",
      brightGreen: "#1a7f37",
      brightYellow: "#633c01",
      brightBlue: "#218bff",
      brightMagenta: "#a475f9",
      brightCyan: "#3192aa",
      brightWhite: "#8c959f",
    },
  },
  {
    id: "one-half-light",
    name: "One Half Light",
    mode: "light",
    contrast: LIGHT_CONTRAST,
    colors: {
      background: "#fafafa",
      foreground: "#383a42",
      cursor: "#4f525e",
      selectionBackground: "#bfceff",
      black: "#383a42",
      red: "#e45649",
      green: "#50a14f",
      yellow: "#c18401",
      blue: "#0184bc",
      magenta: "#a626a4",
      cyan: "#0997b3",
      white: "#bababa",
      brightBlack: "#4f525e",
      brightRed: "#e06c75",
      brightGreen: "#98c379",
      brightYellow: "#e5c07b",
      brightBlue: "#61afef",
      brightMagenta: "#c678dd",
      brightCyan: "#56b6c2",
      brightWhite: "#ffffff",
    },
  },
  {
    id: "catppuccin-latte",
    name: "Catppuccin Latte",
    mode: "light",
    contrast: LIGHT_CONTRAST,
    colors: {
      background: "#eff1f5",
      foreground: "#4c4f69",
      cursor: "#dc8a78",
      selectionBackground: "rgba(124, 127, 147, 0.3)",
      black: "#5c5f77",
      red: "#d20f39",
      green: "#40a02b",
      yellow: "#df8e1d",
      blue: "#1e66f5",
      magenta: "#ea76cb",
      cyan: "#179299",
      white: "#acb0be",
      brightBlack: "#6c6f85",
      brightRed: "#d20f39",
      brightGreen: "#40a02b",
      brightYellow: "#df8e1d",
      brightBlue: "#1e66f5",
      brightMagenta: "#ea76cb",
      brightCyan: "#179299",
      brightWhite: "#bcc0cc",
    },
  },
  {
    id: "solarized-light",
    name: "Solarized Light",
    mode: "light",
    contrast: LIGHT_CONTRAST,
    colors: {
      background: "#fdf6e3",
      foreground: "#657b83",
      cursor: "#657b83",
      selectionBackground: "rgba(147, 161, 161, 0.3)",
      black: "#073642",
      red: "#dc322f",
      green: "#859900",
      yellow: "#b58900",
      blue: "#268bd2",
      magenta: "#d33682",
      cyan: "#2aa198",
      white: "#bbb5a2",
      brightBlack: "#002b36",
      brightRed: "#cb4b16",
      brightGreen: "#586e75",
      brightYellow: "#657b83",
      brightBlue: "#839496",
      brightMagenta: "#6c71c4",
      brightCyan: "#93a1a1",
      brightWhite: "#fdf6e3",
    },
  },
  {
    id: "gruvbox-light",
    name: "Gruvbox Light",
    mode: "light",
    contrast: LIGHT_CONTRAST,
    colors: {
      background: "#fbf1c7",
      foreground: "#3c3836",
      cursor: "#3c3836",
      selectionBackground: "#d5c4a1",
      black: "#fbf1c7",
      red: "#cc241d",
      green: "#98971a",
      yellow: "#d79921",
      blue: "#458588",
      magenta: "#b16286",
      cyan: "#689d6a",
      white: "#7c6f64",
      brightBlack: "#928374",
      brightRed: "#9d0006",
      brightGreen: "#79740e",
      brightYellow: "#b57614",
      brightBlue: "#076678",
      brightMagenta: "#8f3f71",
      brightCyan: "#427b58",
      brightWhite: "#3c3836",
    },
  },
  {
    id: "rose-pine-dawn",
    name: "Rosé Pine Dawn",
    mode: "light",
    contrast: LIGHT_CONTRAST,
    colors: {
      background: "#faf4ed",
      foreground: "#575279",
      cursor: "#575279",
      selectionBackground: "#dfdad9",
      black: "#f2e9e1",
      red: "#b4637a",
      green: "#286983",
      yellow: "#ea9d34",
      blue: "#56949f",
      magenta: "#907aa9",
      cyan: "#d7827e",
      white: "#575279",
      brightBlack: "#9893a5",
      brightRed: "#b4637a",
      brightGreen: "#286983",
      brightYellow: "#ea9d34",
      brightBlue: "#56949f",
      brightMagenta: "#907aa9",
      brightCyan: "#d7827e",
      brightWhite: "#575279",
    },
  },
];

/** The scheme picked for `mode`; the mode's default when none is, or `id` names no scheme of that mode. */
export function schemeFor(mode: TermMode, id: string | null | undefined): TermScheme {
  const picked = id ? SCHEMES.find((s) => s.id === id && s.mode === mode) : undefined;
  return picked ?? SCHEMES.find((s) => s.id === DEFAULT_SCHEME[mode])!;
}

export function schemesFor(mode: TermMode): TermScheme[] {
  return SCHEMES.filter((s) => s.mode === mode);
}

function rgb(hex: string): [number, number, number] {
  const n = Number.parseInt(hex.slice(1, 7), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

/** `#RRGGBB` that is `amount` of `a` over `b`. */
function mix(a: string, b: string, amount: number): string {
  const [ar, ag, ab] = rgb(a);
  const [br, bg, bb] = rgb(b);
  const ch = (x: number, y: number) =>
    Math.round(x * amount + y * (1 - amount))
      .toString(16)
      .padStart(2, "0");
  return `#${ch(ar, br)}${ch(ag, bg)}${ch(ab, bb)}`;
}

/** WCAG contrast ratio of two `#RRGGBB` colours. */
export function contrast(a: string, b: string): number {
  const lum = (hex: string) => {
    const [r, g, b] = rgb(hex).map((c) => {
      const v = c / 255;
      return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
    });
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** What a match's text needs against its highlight; Hatoba Dark's fixed colours give 3.4. */
export const MATCH_CONTRAST = 3;

/** The most of `yellow` over `background`, up to `amount`, that keeps the text readable on it. */
function highlightAmount(yellow: string, background: string, foreground: string, amount: number): number {
  let a = amount;
  while (a > 0.1 && contrast(foreground, mix(yellow, background, a)) < MATCH_CONTRAST) a -= 0.05;
  return a;
}

/** Search highlights: the scheme's own, or its yellow blended into the background. */
export function searchColors(scheme: TermScheme): SearchColors {
  if (scheme.search) return scheme.search;
  const { yellow, background, foreground } = scheme.colors;
  const dark = scheme.mode === "dark";
  const accent = dark ? yellow : mix(yellow, "#000000", 0.8);
  const active = highlightAmount(yellow, background, foreground, dark ? 0.6 : 0.55);
  // The other matches stay fainter than the current one.
  const match = Math.min(highlightAmount(yellow, background, foreground, dark ? 0.3 : 0.28), active / 2);
  return {
    matchBackground: mix(yellow, background, match),
    matchBorder: mix(yellow, background, dark ? 0.55 : 0.6),
    matchOverviewRuler: accent,
    activeMatchBackground: mix(yellow, background, active),
    activeMatchBorder: accent,
    activeMatchColorOverviewRuler: accent,
  };
}
