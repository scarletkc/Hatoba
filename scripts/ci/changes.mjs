// Says whether a pull request changes anything that the Rust jobs in ci.yml build or check, so that
// CI can skip those jobs when it does not.
// Usage: node scripts/ci/changes.mjs BASE
// Compares HEAD with BASE, prints rust=true or rust=false, and in GitHub Actions also appends it to
// $GITHUB_OUTPUT.
import { execFileSync } from "node:child_process";
import { appendFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
import { ROOT } from "../release/version.mjs";

/** The files that the Rust jobs compile, read, or check. A path ending in / covers a directory. */
const RUST_INPUTS = [
  "Cargo.toml",
  "Cargo.lock",
  "crates/",
  "apps/desktop/src-tauri/",
  // The Linux job checks that the tests leave this generated file unchanged.
  "apps/desktop/src/ipc/bindings.ts",
  // A hatoba-core test reads the migrations.
  "workers/sync/migrations/",
  // comment-width.mjs runs in the Linux job, and this script decides which jobs run.
  "scripts/ci/",
  ".github/workflows/ci.yml",
];

/** Whether a change to the file at `path`, relative to the repository root, needs the Rust jobs. */
export const affectsRust = (path) => RUST_INPUTS.some((input) => (input.endsWith("/") ? path.startsWith(input) : path === input));

function main(argv) {
  if (argv.length !== 1) {
    console.error("usage: node scripts/ci/changes.mjs BASE");
    return 2;
  }
  // Without rename detection, a file moved out of a Rust input counts as a change to it.
  const changed = execFileSync("git", ["diff", "--name-only", "-z", "--no-renames", argv[0], "HEAD"], { cwd: ROOT, encoding: "utf8" })
    .split("\0")
    .filter(Boolean);
  const rust = changed.filter(affectsRust);
  console.log(rust.length > 0 ? `Rust inputs changed:\n${rust.map((path) => `  ${path}`).join("\n")}` : "No Rust inputs changed.");
  const output = `rust=${rust.length > 0}`;
  console.log(output);
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `${output}\n`);
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = main(process.argv.slice(2));
}
