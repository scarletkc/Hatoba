// Git command helpers and repository root path shared by CI and release scripts.
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const MAX_BUFFER = 64 * 1024 * 1024;

export const nulSeparated = (output) => output.split("\0").filter(Boolean);

/** Runs git in `root` and returns raw stdout without trimming. */
export function gitRaw(args, root = ROOT, { stderr = "pipe" } = {}) {
  return execFileSync("git", args, {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", stderr],
    maxBuffer: MAX_BUFFER,
  });
}

/** Runs git in `root` and returns trimmed stdout. */
export function git(args, root = ROOT) {
  return gitRaw(args, root).trim();
}

/** Lists existing tracked and untracked (non-ignored) files matching `pathspecs` in `root`. */
export function lsFiles(pathspecs = [], root = ROOT) {
  const paths = Array.isArray(pathspecs) ? pathspecs : [pathspecs];
  const args = [
    "ls-files",
    "-z",
    "--cached",
    "--others",
    "--exclude-standard",
    ...(paths.length > 0 ? ["--", ...paths] : []),
  ];
  return nulSeparated(gitRaw(args, root)).filter((path) => existsSync(join(root, path)));
}
