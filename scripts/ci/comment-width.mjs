// Checks that no comment line in the Rust sources is longer than 100 characters, the width
// rustfmt keeps the code within. rustfmt leaves comments alone: `wrap_comments` is nightly-only.
// Usage: node scripts/ci/comment-width.mjs
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { ROOT } from "../release/version.mjs";

export const MAX_WIDTH = 100;

const isIdent = (char) => char !== undefined && /[\p{XID_Continue}]/u.test(char);

/**
 * The 1-based numbers of the lines in Rust source `text` that hold comment text, whether the
 * comment fills the line or trails code. Comment markers inside string and char literals do not
 * count.
 */
export function commentLines(text) {
  const chars = [...text.replaceAll("\r\n", "\n")];
  const lines = new Set();
  let line = 1;
  let i = 0;
  // Steps over one character, counting the lines it ends.
  const step = () => {
    if (chars[i] === "\n") line++;
    i++;
  };
  while (i < chars.length) {
    const char = chars[i];
    const next = chars[i + 1];
    if (char === "/" && next === "/") {
      lines.add(line);
      while (i < chars.length && chars[i] !== "\n") i++;
    } else if (char === "/" && next === "*") {
      // Block comments nest.
      let depth = 0;
      do {
        lines.add(line);
        if (chars[i] === "/" && chars[i + 1] === "*") {
          depth++;
          i += 2;
        } else if (chars[i] === "*" && chars[i + 1] === "/") {
          depth--;
          i += 2;
        } else {
          step();
        }
      } while (i < chars.length && depth > 0);
    } else if (char === '"') {
      i++;
      while (i < chars.length && chars[i] !== '"') {
        if (chars[i] === "\\") i++;
        step();
      }
      i++;
    } else if (char === "r" && !isIdent(chars[i - 1] === "b" || chars[i - 1] === "c" ? chars[i - 2] : chars[i - 1])) {
      // A raw string, r"…" or r#"…"#, also after b or c; otherwise an identifier such as r#type.
      let end = i + 1;
      while (chars[end] === "#") end++;
      if (chars[end] !== '"') {
        i++;
        continue;
      }
      const close = '"' + "#".repeat(end - i - 1);
      i = end + 1;
      while (i < chars.length && chars.slice(i, i + close.length).join("") !== close) step();
      i += close.length;
    } else if (char === "'" && next === "\\") {
      // An escaped char literal such as '\'' or '\u{2192}'.
      i += 3;
      while (i < chars.length && chars[i] !== "'") i++;
      i++;
    } else if (char === "'" && chars[i + 2] === "'") {
      // A char literal such as '"' or '/'; a lifetime such as 'a has no closing quote.
      i += 3;
    } else {
      step();
    }
  }
  return lines;
}

/** The lines in `text` that hold comment text and are longer than MAX_WIDTH. */
export function longComments(text) {
  const comments = commentLines(text);
  const found = [];
  text.split(/\r?\n/).forEach((line, index) => {
    // Characters, not UTF-16 units or bytes: an arrow such as → is one character but three bytes.
    const width = [...line].length;
    if (width > MAX_WIDTH && comments.has(index + 1)) found.push({ line: index + 1, width });
  });
  return found;
}

/** The Rust files in the working tree that git tracks or would track. */
function rustFiles(root) {
  const output = execFileSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", "*.rs"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 64 * 1024 * 1024,
  });
  return output.split("\0").filter((path) => path && existsSync(join(root, path)));
}

function main() {
  const files = rustFiles(ROOT);
  const found = files.flatMap((path) => longComments(readFileSync(join(ROOT, path), "utf8")).map((long) => ({ path, ...long })));
  if (found.length === 0) {
    console.log(`The comment lines in ${files.length} Rust files are within ${MAX_WIDTH} characters.`);
    return 0;
  }
  for (const { path, line, width } of found) {
    const message = `This comment line has ${width} characters; wrap it within ${MAX_WIDTH}.`;
    if (process.env.GITHUB_ACTIONS === "true") {
      console.log(`::error file=${path},line=${line},title=Comment width::${message}`);
    } else {
      console.error(`${path}:${line}: ${message}`);
    }
  }
  return 1;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = main();
}
