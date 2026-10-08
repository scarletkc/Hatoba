// Release versions: their format and order, and the files that record them.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = fileURLToPath(new URL("../..", import.meta.url));
/** Handwritten release notes, one VERSION.md per release. */
export const NOTES_DIR = "docs/release-notes";

export const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(alpha|beta|rc)\.(0|[1-9]\d*))?$/;
const STAGES = { alpha: 0, beta: 1, rc: 2 };

export function parseVersion(version) {
  const match = VERSION.exec(version);
  if (match === null) {
    throw new Error(`'${version}' is not a release version. Use MAJOR.MINOR.PATCH, optionally followed by -alpha.N, -beta.N, or -rc.N.`);
  }
  const [, major, minor, patch, stage, number] = match;
  return { core: [Number(major), Number(minor), Number(patch)], stage: stage ?? null, number: stage ? Number(number) : 0 };
}

export function isPrerelease(version) {
  return parseVersion(version).stage !== null;
}

/** Orders versions as SemVer does: 1.2.3-alpha.9 < 1.2.3-alpha.10 < 1.2.3-beta.1 < 1.2.3-rc.1 < 1.2.3. */
export function compareVersions(a, b) {
  const key = (version) => {
    const { core, stage, number } = parseVersion(version);
    return [...core, stage === null ? 3 : STAGES[stage], number];
  };
  const [ka, kb] = [key(a), key(b)];
  const i = ka.findIndex((part, index) => part !== kb[index]);
  return i === -1 ? 0 : ka[i] - kb[i];
}

/** `patch` from a prerelease moves to that version's final release. */
export function nextVersion(current, requested) {
  const {
    core: [major, minor, patch],
    stage,
  } = parseVersion(current);
  const next =
    requested === "major"
      ? `${major + 1}.0.0`
      : requested === "minor"
        ? `${major}.${minor + 1}.0`
        : requested === "patch"
          ? `${major}.${minor}.${stage === null ? patch + 1 : patch}`
          : requested.replace(/^v/, "");
  if (compareVersions(next, current) <= 0) {
    throw new Error(`The new version must be greater than ${current}; ${next} is not.`);
  }
  return next;
}

function replaceOnce(text, pattern, replacement, what) {
  let count = 0;
  const updated = text.replace(pattern, (...args) => {
    count += 1;
    return replacement(...args);
  });
  if (count !== 1) throw new Error(`Expected exactly one ${what}; found ${count}.`);
  return updated;
}

// The version line within [workspace.package], before the next section header.
const WORKSPACE_VERSION = /^(\[workspace\.package\]\n(?:(?!\[)[^\n]*\n)*?version\s*=\s*")([^"]*)(")/gm;

const cargoManifest = {
  read: (text) => [...text.matchAll(WORKSPACE_VERSION)].map((m) => m[2]),
  write: (text, to) => replaceOnce(text, WORKSPACE_VERSION, (_, head, _old, tail) => `${head}${to}${tail}`, "version in [workspace.package]"),
};

// Packages without a `source` are the workspace's own crates; registry and git packages keep theirs.
const lockPackages = (text) => text.split(/^(?=\[\[package\]\])/m);
const isLocal = (block) => block.startsWith("[[package]]") && !/^source\s*=/m.test(block);
const LOCK_VERSION = /^(version\s*=\s*")([^"]*)(")/m;

const cargoLock = {
  read: (text) => lockPackages(text).filter(isLocal).map((block) => LOCK_VERSION.exec(block)?.[2]),
  write: (text, to) =>
    lockPackages(text)
      .map((block) => (isLocal(block) ? block.replace(LOCK_VERSION, `$1${to}$3`) : block))
      .join(""),
};

// npm writes package.json and package-lock.json as two-space JSON with a final newline. Rewriting a
// file in that form keeps every other line intact, which parse() checks before any change.
function parseNpmJson(text) {
  const json = JSON.parse(text);
  if (`${JSON.stringify(json, null, 2)}\n` !== text) {
    throw new Error("The file is not formatted as npm writes it (two-space JSON with a final newline); reformat it, then retry.");
  }
  return json;
}
const npmJson = (json) => `${JSON.stringify(json, null, 2)}\n`;

const packageJson = {
  read: (text) => [parseNpmJson(text).version],
  write: (text, to) => npmJson({ ...parseNpmJson(text), version: to }),
};

const packageLock = {
  read: (text) => {
    const json = parseNpmJson(text);
    return [json.version, json.packages?.[""]?.version];
  },
  write: (text, to) => {
    const json = parseNpmJson(text);
    json.version = to;
    json.packages[""].version = to;
    return npmJson(json);
  },
};

const WORKER_VERSION = /^(export const VERSION = ")([^"]*)(";)$/gm;

const workerConfig = {
  read: (text) => [...text.matchAll(WORKER_VERSION)].map((m) => m[2]),
  write: (text, to) => replaceOnce(text, WORKER_VERSION, (_, head, _old, tail) => `${head}${to}${tail}`, "export const VERSION"),
};

/**
 * Every file that records the release version, which the app and the crates share. Tauri reads the
 * app version from the workspace Cargo.toml.
 */
export const VERSION_FILES = [
  { path: "Cargo.toml", ...cargoManifest },
  { path: "Cargo.lock", ...cargoLock },
  { path: "package.json", ...packageJson },
  { path: "apps/desktop/package.json", ...packageJson },
];

/**
 * Every file that records the sync Worker version, which /v1/health reports from config.ts. A change
 * to the Worker raises it (docs/hatoba-spec.md §6.7, Versions), and a release leaves it alone.
 */
export const WORKER_VERSION_FILES = [
  { path: "workers/sync/package.json", ...packageJson },
  { path: "workers/sync/package-lock.json", ...packageLock },
  { path: "workers/sync/src/config.ts", ...workerConfig },
];

function readFile(root, path) {
  return readFileSync(join(root, path), "utf8");
}

function versionsIn(file, text) {
  try {
    return file.read(text);
  } catch (error) {
    throw new Error(`${file.path}: ${error.message}`);
  }
}

/** Returns the version that every one of `files` records, or explains where they differ. */
export function readVersion(root, files = VERSION_FILES) {
  const found = files.map((file) => ({ path: file.path, versions: versionsIn(file, readFile(root, file.path)) }));
  const all = new Set(found.flatMap((f) => f.versions));
  if (all.size !== 1 || found.some((f) => f.versions.length === 0)) {
    const lines = found.map((f) => `  ${f.path}: ${f.versions.map((v) => v ?? "(no version)").join(", ") || "(no version)"}`);
    throw new Error(`The version files disagree. Give them all the same version:\n${lines.join("\n")}`);
  }
  const [version] = all;
  parseVersion(version);
  return version;
}

/** Plans the new contents of every one of `files`, so that a bump writes all of them or none. */
export function planBump(root, to, files = VERSION_FILES) {
  parseVersion(to);
  const changes = new Map();
  for (const file of files) {
    const updated = file.write(readFile(root, file.path), to);
    if (versionsIn(file, updated).some((v) => v !== to)) {
      throw new Error(`${file.path}: could not set the version to ${to}.`);
    }
    changes.set(file.path, updated);
  }
  return changes;
}
