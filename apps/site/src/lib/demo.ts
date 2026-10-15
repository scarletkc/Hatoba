import { existsSync } from "node:fs";
import { join } from "node:path";
import { repoRoot } from "./docs";

/** Where scripts/demo.mjs builds the app's demo, which the site serves at /demo/. */
const DEMO_DIR = "apps/site/public/demo";

/** Whether scripts/demo.mjs built the demo, so the home page offers it. */
export function demoBuilt(): boolean {
  return existsSync(join(repoRoot(), DEMO_DIR, "index.html"));
}
