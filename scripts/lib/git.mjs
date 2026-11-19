import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const MAX_BUFFER = 64 * 1024 * 1024;

export const nulSeparated = (output) => output.split("\0").filter(Boolean);

export function gitRaw(args, root = ROOT, { stderr = "pipe" } = {}) {
  return execFileSync("git", args, {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", stderr],
    maxBuffer: MAX_BUFFER,
  });
}

export function git(args, root = ROOT) {
  return gitRaw(args, root).trim();
}

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
  // --cached still lists tracked files deleted in the working tree.
  return nulSeparated(gitRaw(args, root)).filter((path) => existsSync(join(root, path)));
}
