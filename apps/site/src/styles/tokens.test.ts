import { readdirSync, readFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * The site's stylesheets and components take their colours, text sizes, spaces, radii, shadows and
 * durations from tokens.css, so the same role looks the same on every page. A raw value anywhere
 * else fails here: use a token, or add a step to tokens.css when none fits. Widths, heights, offsets
 * and border widths are geometry and stay free.
 */

const SRC = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const TOKENS = resolve(SRC, "styles/tokens.css");

/** Properties whose px values must be tokens. */
const SIZED = /^(font-size|border-radius|gap|row-gap|column-gap|padding(-[a-z-]+)?|margin(-[a-z-]+)?)$/;
const TIMED = /^(transition|animation)(-duration|-delay)?$/;

function files(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) return files(path);
    return /\.(css|astro)$/.test(entry.name) && path !== TOKENS ? [path] : [];
  });
}

/** Only the line breaks of `text`, so that line numbers stay right. */
const lines = (text: string) => "\n".repeat(text.split("\n").length - 1);

/** The CSS of a file: all of a stylesheet, or the <style> blocks of a component, with comments blanked and lines kept. */
function css(path: string): string {
  const text = readFileSync(path, "utf8");
  let source = text;
  if (!path.endsWith(".css")) {
    source = "";
    let at = 0;
    for (const block of text.matchAll(/<style[^>]*>([\s\S]*?)<\/style>/g)) {
      const start = block.index + block[0].indexOf(">") + 1;
      source += lines(text.slice(at, start)) + block[1];
      at = start + block[1].length;
    }
    source += lines(text.slice(at));
  }
  return source.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, " "));
}

function declarations(text: string): { prop: string; value: string; line: number }[] {
  return [...text.matchAll(/(?<![-\w])([a-z-]+)\s*:\s*([^;{}]*)[;}]/g)]
    .filter((match) => !/^(--|@)/.test(match[1]))
    .map((match) => ({ prop: match[1], value: match[2].trim(), line: text.slice(0, match.index).split("\n").length }));
}

const tokens = new Set([...readFileSync(TOKENS, "utf8").matchAll(/(--[\w-]+)\s*:/g)].map((match) => match[1]));

const found = files(SRC).flatMap((path) => {
  const name = relative(SRC, path).replace(/\\/g, "/");
  // Custom properties a component sets for itself, in its CSS or from its markup.
  const local = new Set([...readFileSync(path, "utf8").matchAll(/["']?(--[\w-]+)["']?\s*:/g)].map((match) => match[1]));
  return declarations(css(path)).map((d) => ({ ...d, at: `${name}:${d.line} ${d.prop}: ${d.value}`, local }));
});

describe("site styles", () => {
  it("finds the stylesheets and components", () => {
    expect(new Set(found.map((d) => d.at.split(":")[0])).size).toBeGreaterThan(8);
  });

  it("take colours from tokens", () => {
    const raw = found.filter((d) => /#[0-9a-f]{3,8}\b|\b(rgba?|hsla?|oklch|lab)\(/i.test(d.value.replace(/url\([^)]*\)/g, "")));
    expect(raw.map((d) => d.at)).toEqual([]);
  });

  it("take text sizes, spaces and radii from tokens", () => {
    const raw = found.filter((d) => SIZED.test(d.prop) && /(?<![\w.])-?\d*\.?\d+px/.test(d.value));
    expect(raw.map((d) => d.at)).toEqual([]);
  });

  it("take shadows and durations from tokens", () => {
    const raw = found.filter(
      (d) => (d.prop === "box-shadow" && !/^(none|var\(--shadow-[\w-]+\))$/.test(d.value)) || (TIMED.test(d.prop) && /(?<![\w-])\d*\.?\d+m?s\b/.test(d.value)),
    );
    expect(raw.map((d) => d.at)).toEqual([]);
  });

  it("name tokens that exist", () => {
    const unknown = found.flatMap((d) =>
      [...d.value.matchAll(/var\((--[\w-]+)/g)].map((match) => match[1]).filter((name) => !tokens.has(name) && !d.local.has(name)).map((name) => `${d.at} (${name})`),
    );
    expect(unknown).toEqual([]);
  });
});
