import type { SchemeColors as PaletteColors } from "@/ipc/types";
import { mix, modeOf, PALETTE_KEYS, paletteOf, schemeFor, toHex, type PaletteKey, type TermMode } from "./schemes";

/** A scheme read from a file, before the user saves it (TERM-06). */
export interface ImportedScheme {
  name: string;
  mode: TermMode;
  colors: PaletteColors;
}

export type ImportFailure = "tooLarge" | "unreadable" | "noScheme";

export class SchemeImportError extends Error {
  constructor(readonly reason: ImportFailure) {
    super(reason);
  }
}

/** Larger files are not colour schemes; Windows Terminal's whole settings.json stays far below this. */
export const MAX_IMPORT_BYTES = 1024 * 1024;

const NAME_MAX = 64;

const ANSI: readonly PaletteKey[] = PALETTE_KEYS.slice(4);

/**
 * The colour schemes in a Windows Terminal scheme (JSON: one scheme, a list of them, or a whole
 * settings.json with its `schemes`) or an iTerm2 `.itermcolors` file (one scheme, or two when it keeps
 * separate colours for light and dark mode). Colours a file leaves out are filled in: the cursor from the
 * text, the selection from the text over the background, ANSI colours from Hatoba's scheme for the mode.
 */
export function parseSchemeFile(text: string, fileName: string): ImportedScheme[] {
  const base = fileName.replace(/^.*[\\/]/, "");
  const stem = base.replace(/\.[^.]*$/, "") || base;
  const found = text.trimStart().startsWith("<") ? fromItermColors(text, stem) : fromWindowsTerminal(text, stem);
  if (found.length === 0) throw new SchemeImportError("noScheme");
  return found;
}

/** Fills in what a file left out; `null` without a background, text, and some ANSI colour. */
function complete(name: string, partial: Partial<Record<PaletteKey, string>>): ImportedScheme | null {
  const { background, foreground } = partial;
  if (!background || !foreground || !ANSI.some((k) => partial[k])) return null;
  const mode = modeOf(background);
  const fallback = paletteOf(schemeFor(mode, null));
  const colors = {} as PaletteColors;
  for (const key of PALETTE_KEYS) colors[key] = partial[key] ?? fallback[key];
  colors.cursor = partial.cursor ?? foreground;
  colors.selection = partial.selection ?? mix(foreground, background, 0.3);
  return { name: cleanName(name), mode, colors };
}

function cleanName(name: string): string {
  const flat = name.replace(/[\p{Cc}\s]+/gu, " ").trim();
  return [...flat].slice(0, NAME_MAX).join("").trim();
}

// ───────────────────────── Windows Terminal ─────────────────────────

/** Windows Terminal's keys for each colour; "purple" is its name for magenta. */
const WT_KEYS: Record<PaletteKey, string[]> = {
  foreground: ["foreground"],
  background: ["background"],
  cursor: ["cursorColor"],
  selection: ["selectionBackground"],
  black: ["black"],
  red: ["red"],
  green: ["green"],
  yellow: ["yellow"],
  blue: ["blue"],
  magenta: ["purple", "magenta"],
  cyan: ["cyan"],
  white: ["white"],
  bright_black: ["brightBlack"],
  bright_red: ["brightRed"],
  bright_green: ["brightGreen"],
  bright_yellow: ["brightYellow"],
  bright_blue: ["brightBlue"],
  bright_magenta: ["brightPurple", "brightMagenta"],
  bright_cyan: ["brightCyan"],
  bright_white: ["brightWhite"],
};

function fromWindowsTerminal(text: string, stem: string): ImportedScheme[] {
  let value: unknown;
  try {
    value = JSON.parse(stripJsonComments(text));
  } catch {
    throw new SchemeImportError("unreadable");
  }
  const list: unknown[] = Array.isArray(value) ? value : isObject(value) && Array.isArray(value.schemes) ? value.schemes : [value];
  return list.flatMap((entry) => {
    if (!isObject(entry)) return [];
    const partial: Partial<Record<PaletteKey, string>> = {};
    for (const key of PALETTE_KEYS) {
      for (const name of WT_KEYS[key]) {
        const raw = entry[name];
        const hex = typeof raw === "string" ? toHex(raw) : null;
        if (hex) {
          partial[key] = hex;
          break;
        }
      }
    }
    const name = typeof entry.name === "string" && entry.name.trim() ? entry.name : stem;
    const scheme = complete(name, partial);
    return scheme ? [scheme] : [];
  });
}

function isObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** settings.json is JSON with comments and trailing commas; drops both and leaves strings alone. */
export function stripJsonComments(text: string): string {
  let out = "";
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (c === '"') {
      const start = i;
      for (i++; i < text.length && text[i] !== '"'; i++) if (text[i] === "\\") i++;
      out += text.slice(start, i + 1);
    } else if (c === "/" && text[i + 1] === "/") {
      while (i + 1 < text.length && text[i + 1] !== "\n") i++;
    } else if (c === "/" && text[i + 1] === "*") {
      const end = text.indexOf("*/", i + 2);
      i = end < 0 ? text.length : end + 1;
      out += " ";
    } else if (c !== "," || !closesNext(text, i + 1)) {
      out += c;
    }
  }
  return out;
}

/** Whether only whitespace and comments stand between `from` and a closing bracket. */
function closesNext(text: string, from: number): boolean {
  let j = from;
  for (;;) {
    while (j < text.length && /\s/.test(text[j])) j++;
    if (text.startsWith("//", j)) {
      const nl = text.indexOf("\n", j);
      j = nl < 0 ? text.length : nl;
    } else if (text.startsWith("/*", j)) {
      const end = text.indexOf("*/", j + 2);
      j = end < 0 ? text.length : end + 2;
    } else {
      return text[j] === "}" || text[j] === "]";
    }
  }
}

// ───────────────────────── iTerm2 ─────────────────────────

const ITERM_KEYS: ReadonlyMap<string, PaletteKey> = new Map<string, PaletteKey>([
  ["Foreground Color", "foreground"],
  ["Background Color", "background"],
  ["Cursor Color", "cursor"],
  ["Selection Color", "selection"],
  ...ANSI.map((key, n): [string, PaletteKey] => [`Ansi ${n} Color`, key]),
]);

/** iTerm2 3.5 and later keep separate colours for light and dark mode under these suffixes. */
const ITERM_VARIANTS = [" (Dark)", " (Light)"];

function fromItermColors(text: string, stem: string): ImportedScheme[] {
  const doc = new DOMParser().parseFromString(text, "application/xml");
  const root = doc.querySelector("plist > dict");
  if (doc.querySelector("parsererror") || !root) throw new SchemeImportError("unreadable");
  const entries = new Map<string, string>();
  for (const [key, value] of pairs(root)) {
    const hex = value.tagName === "dict" ? itermColor(value) : null;
    if (hex) entries.set(key, hex);
  }
  const variants = entries.has("Background Color") ? [""] : ITERM_VARIANTS;
  return variants.flatMap((suffix) => {
    const partial: Partial<Record<PaletteKey, string>> = {};
    for (const [name, key] of ITERM_KEYS) {
      const hex = entries.get(name + suffix);
      if (hex) partial[key] = hex;
    }
    const scheme = complete(stem + suffix, partial);
    return scheme ? [scheme] : [];
  });
}

/** A plist dict's `<key>` elements with the value element after each. */
function pairs(dict: Element): [string, Element][] {
  const out: [string, Element][] = [];
  const children = [...dict.children];
  for (let i = 0; i + 1 < children.length; i++) {
    if (children[i].tagName === "key") out.push([children[i].textContent?.trim() ?? "", children[i + 1]]);
  }
  return out;
}

/** A colour dict's components (0 to 1) as `#rrggbb`. The colour space is read as sRGB. */
function itermColor(dict: Element): string | null {
  const values = new Map<string, number>();
  for (const [key, value] of pairs(dict)) {
    const n = Number(value.textContent);
    if ((value.tagName === "real" || value.tagName === "integer") && Number.isFinite(n)) values.set(key, n);
  }
  const rgb = ["Red", "Green", "Blue"].map((c) => values.get(`${c} Component`));
  if (rgb.some((v) => v === undefined)) return null;
  const channels = rgb.map((v) => Math.round(Math.min(1, Math.max(0, v!)) * 255));
  return toHex(`rgba(${channels.join(",")},${values.get("Alpha Component") ?? 1})`);
}
