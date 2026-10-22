import { readdirSync, readFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

/**
 * Screens take their text sizes, radii and gaps from tokens.css, so the same role looks the same
 * everywhere. A raw px value in a feature stylesheet fails here: use a token, or add one to
 * tokens.css when no step fits.
 */

const SRC = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const GUARDED = ["features", "app"];
const PROPS = ["font-size", "border-radius", "gap", "row-gap", "column-gap"];

function cssFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(dir, entry.name);
    if (entry.isDirectory()) return cssFiles(path);
    return entry.name.endsWith(".css") ? [path] : [];
  });
}

function declarations(css: string): { prop: string; value: string; line: number }[] {
  const text = css.replace(/\/\*[\s\S]*?\*\//g, (comment) => comment.replace(/[^\n]/g, " "));
  const found: { prop: string; value: string; line: number }[] = [];
  const pattern = new RegExp(`(?<![-\\w])(${PROPS.join("|")})\\s*:\\s*([^;}]*)`, "g");
  for (const match of text.matchAll(pattern)) {
    found.push({
      prop: match[1],
      value: match[2].trim(),
      line: text.slice(0, match.index).split("\n").length,
    });
  }
  return found;
}

const tokens = new Set(
  [...readFileSync(resolve(SRC, "styles/tokens.css"), "utf8").matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]),
);

describe("feature stylesheets", () => {
  const files = GUARDED.flatMap((dir) => cssFiles(resolve(SRC, dir)));

  it("finds the stylesheets", () => {
    expect(files.length).toBeGreaterThan(20);
  });

  it("use tokens for text sizes, radii and gaps", () => {
    const raw = files.flatMap((file) =>
      declarations(readFileSync(file, "utf8"))
        .filter((d) => /(?<![\w.-])(?!0px)\d*\.?\d+px/.test(d.value))
        .map((d) => `${relative(SRC, file).replace(/\\/g, "/")}:${d.line} ${d.prop}: ${d.value}`),
    );
    expect(raw).toEqual([]);
  });

  it("name tokens that exist", () => {
    const unknown = files.flatMap((file) =>
      declarations(readFileSync(file, "utf8")).flatMap((d) =>
        [...d.value.matchAll(/var\((--[\w-]+)/g)]
          .map((m) => m[1])
          .filter((name) => !tokens.has(name))
          .map((name) => `${relative(SRC, file).replace(/\\/g, "/")}:${d.line} ${d.prop}: ${name}`),
      ),
    );
    expect(unknown).toEqual([]);
  });
});
