// Checks that no comment line in the Rust sources is longer than 100 characters, the width
// rustfmt keeps the code within. rustfmt leaves comments alone: `wrap_comments` is nightly-only.
// Usage: node scripts/ci/comment-width.mjs
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { ROOT } from "../release/version.mjs";

export const MAX_WIDTH = 100;

/** The comment lines in `text` that are longer than MAX_WIDTH, by 1-based line number. */
export function longComments(text) {
  const found = [];
  text.split(/\r?\n/).forEach((line, index) => {
    // Characters, not UTF-16 units or bytes: an arrow such as → is one character but three bytes.
    const width = [...line].length;
    if (width > MAX_WIDTH && line.trimStart().startsWith("//")) found.push({ line: index + 1, width });
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
