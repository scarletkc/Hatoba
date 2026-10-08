// Sets a new release version in every version file and can start its release note.
// Usage: node scripts/release/bump.mjs [patch|minor|major|VERSION] [--note TITLE] [--dry-run]
// patch is the default. docs/releasing.md describes the release steps.
import { existsSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { NOTES_DIR, ROOT, nextVersion, planBump, readVersion } from "./version.mjs";

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
    options: { note: { type: "string" }, "dry-run": { type: "boolean", default: false } },
  });
  if (positionals.length > 1) throw new Error("Pass one version: patch, minor, major, or VERSION.");
  const current = readVersion(ROOT);
  const version = nextVersion(current, positionals[0] ?? "patch");
  const changes = planBump(ROOT, version);
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
