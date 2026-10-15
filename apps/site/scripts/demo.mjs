// Builds the app's in-browser demo (`pnpm build:demo` in apps/desktop) into public/demo, which the
// site serves at /demo/ and the home page offers in place of its first screenshot. Like the docs, it
// comes from the release the site offers, so the demo is the version the download button gives:
// the release's tag is checked out in a temporary worktree with its own dependencies.
// HATOBA_DEMO_REF names another git ref, or `worktree` to build apps/desktop as it is, whose
// dependencies must be installed. A ref from before the demo has no build:demo script, so nothing is
// built, and the home page shows only its screenshots.
// Usage: node scripts/demo.mjs
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { offeredRelease } from "../src/lib/github.mjs";

const ROOT = fileURLToPath(new URL("../../..", import.meta.url));
/** Keep in sync with DEMO_DIR in src/lib/demo.ts. */
const OUT = fileURLToPath(new URL("../public/demo/", import.meta.url));

/** Runs `command`. pnpm is a .cmd script on Windows, which only a shell runs, so paths with spaces are quoted for it. */
function run(command, args, cwd) {
  const shell = process.platform === "win32" && command === "pnpm";
  execFileSync(command, shell ? args.map((arg) => (/\s/.test(arg) ? `"${arg}"` : arg)) : args, { cwd, stdio: "inherit", shell });
}

rmSync(OUT, { recursive: true, force: true });
const ref = process.env.HATOBA_DEMO_REF || (await offeredRelease()).current.tag_name;
let checkout = ROOT;
if (ref !== "worktree") {
  checkout = mkdtempSync(join(tmpdir(), "hatoba-demo-"));
  run("git", ["worktree", "add", "--detach", checkout, ref], ROOT);
}
try {
  const desktop = join(checkout, "apps", "desktop");
  const scripts = JSON.parse(readFileSync(join(desktop, "package.json"), "utf8")).scripts ?? {};
  if (!scripts["build:demo"]) {
    console.log(`${ref} has no build:demo script in apps/desktop, so the site has no demo.`);
  } else {
    if (checkout !== ROOT) run("pnpm", ["install", "--frozen-lockfile", "--filter", "@hatoba/desktop"], checkout);
    run("pnpm", ["run", "build:demo", "--base", "/demo/", "--outDir", OUT, "--emptyOutDir"], desktop);
    if (!existsSync(join(OUT, "index.html"))) throw new Error(`Building the demo of ${ref} left no index.html in ${OUT}.`);
    console.log(`Built the demo of ${ref} into ${OUT}.`);
  }
} finally {
  if (checkout !== ROOT) {
    // git worktree remove can't delete the links in node_modules on Windows.
    rmSync(checkout, { recursive: true, force: true });
    run("git", ["worktree", "prune"], ROOT);
  }
}
