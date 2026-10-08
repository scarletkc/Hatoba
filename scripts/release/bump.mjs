// Sets a new release version in every version file and can start its release note. With --worker,
// sets the sync Worker version instead, which a change to the Worker raises.
// Usage: node scripts/release/bump.mjs [patch|minor|major|VERSION] [--note TITLE | --worker] [--dry-run]
// patch is the default. docs/releasing.md describes the release steps.
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { NOTES_DIR, ROOT, VERSION_FILES, WORKER_VERSION_FILES, nextVersion, planBump, readVersion } from "./version.mjs";

/** Adds the release note for `version`, which starts with `## TITLE`, to the planned changes. */
export function planNote(root, changes, version, title) {
  const heading = title.trim();
  if (!heading || heading.includes("\n")) throw new Error("--note needs a title on one line.");
  const path = `${NOTES_DIR}/${version}.md`;
  if (existsSync(join(root, path))) throw new Error(`${path} already exists; edit it instead.`);
  changes.set(path, `## ${heading}\n\n`);
  return path;
}

function main() {
  const { values, positionals } = parseArgs({
    allowPositionals: true,
    options: {
      note: { type: "string" },
      worker: { type: "boolean", default: false },
      "dry-run": { type: "boolean", default: false },
    },
  });
  if (positionals.length > 1) throw new Error("Pass one version: patch, minor, major, or VERSION.");
  if (values.worker && values.note !== undefined) throw new Error("--note starts a release note; a Worker version has none.");
  const files = values.worker ? WORKER_VERSION_FILES : VERSION_FILES;
  const current = readVersion(ROOT, files);
  const version = nextVersion(current, positionals[0] ?? "patch");
  const changes = planBump(ROOT, version, files);
  const note = values.note === undefined ? null : planNote(ROOT, changes, version, values.note);
  const dryRun = values["dry-run"];
  for (const [path, contents] of changes) {
    if (!dryRun) {
      mkdirSync(dirname(join(ROOT, path)), { recursive: true });
      writeFileSync(join(ROOT, path), contents);
    }
    const verb = path === note ? (dryRun ? "Would create" : "Created") : dryRun ? "Would update" : "Updated";
    console.log(`${verb} ${path}`);
  }
  console.log(`${current} -> ${version}`);
  if (note && !dryRun) console.log(`Write the body of ${note} before releasing.`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
