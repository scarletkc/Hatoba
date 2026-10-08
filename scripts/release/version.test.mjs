import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { planNote } from "./bump.mjs";
import { ROOT, WORKER_VERSION_FILES, compareVersions, nextVersion, parseVersion, planBump, readVersion } from "./version.mjs";

const npm = (json) => `${JSON.stringify(json, null, 2)}\n`;

// Each file also holds an unrelated 0.1.0 that a bump must leave alone.
const APP_FIXTURE = {
  "Cargo.toml": '[workspace]\nmembers = ["crates/hatoba-core"]\n\n[workspace.package]\nversion = "0.1.0"\nedition = "2024"\n\n[workspace.dependencies]\nserde = { version = "0.1.0" }\n',
  "Cargo.lock":
    'version = 4\n\n[[package]]\nname = "hatoba-core"\nversion = "0.1.0"\ndependencies = [\n "serde",\n]\n\n[[package]]\nname = "serde"\nversion = "0.1.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\n',
  "package.json": npm({ name: "hatoba", private: true, version: "0.1.0" }),
  "apps/desktop/package.json": npm({ name: "@hatoba/desktop", version: "0.1.0", dependencies: { react: "^0.1.0" } }),
};

// The sync Worker has a version of its own.
const WORKER_FIXTURE = {
  "workers/sync/package.json": npm({ name: "@hatoba/sync-worker", version: "0.5.0" }),
  "workers/sync/package-lock.json": npm({
    name: "@hatoba/sync-worker",
    version: "0.5.0",
    lockfileVersion: 3,
    packages: { "": { name: "@hatoba/sync-worker", version: "0.5.0" }, "node_modules/hono": { version: "0.1.0" } },
  }),
  "workers/sync/src/config.ts": 'export const API_VERSION = 1;\n/** Keep in sync with package.json (enforced by a test). */\nexport const VERSION = "0.5.0";\n',
};

const FIXTURE = { ...APP_FIXTURE, ...WORKER_FIXTURE };

function fixture(t, overrides = {}) {
  const root = mkdtempSync(join(tmpdir(), "hatoba-version-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  for (const [path, contents] of Object.entries({ ...FIXTURE, ...overrides })) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), contents);
  }
  return root;
}

test("rejects versions outside MAJOR.MINOR.PATCH with an optional alpha, beta, or rc number", () => {
  for (const version of ["1.2", "01.2.3", "v1.2.3", "1.2.3-beta", "1.2.3-pre.1", "1.2.3+build.1"]) {
    assert.throws(() => parseVersion(version), /not a release version/, version);
  }
});

test("orders prereleases before their final release", () => {
  const versions = ["1.2.3", "1.2.3-rc.1", "1.2.3-alpha.10", "1.2.2", "1.2.3-beta.1", "1.2.3-alpha.9"];
  assert.deepEqual(versions.sort(compareVersions), ["1.2.2", "1.2.3-alpha.9", "1.2.3-alpha.10", "1.2.3-beta.1", "1.2.3-rc.1", "1.2.3"]);
});

test("computes the next version", () => {
  assert.equal(nextVersion("1.2.3", "patch"), "1.2.4");
  assert.equal(nextVersion("1.2.3", "minor"), "1.3.0");
  assert.equal(nextVersion("1.2.3", "major"), "2.0.0");
  assert.equal(nextVersion("1.2.3-rc.1", "patch"), "1.2.3");
  assert.equal(nextVersion("1.2.3", "v1.3.0-alpha.1"), "1.3.0-alpha.1");
  assert.throws(() => nextVersion("1.2.3", "1.2.3"), /greater than 1\.2\.3/);
  assert.throws(() => nextVersion("1.2.3", "1.2.3-rc.1"), /greater than 1\.2\.3/);
});

test("the repository's app and Worker version files each agree", () => {
  assert.match(readVersion(ROOT), /^\d+\.\d+\.\d+/);
  assert.match(readVersion(ROOT, WORKER_VERSION_FILES), /^\d+\.\d+\.\d+/);
});

test("a bump changes only the release version in every app file", (t) => {
  const root = fixture(t);
  const changes = planBump(root, "0.2.0");
  assert.deepEqual([...changes.keys()].sort(), Object.keys(APP_FIXTURE).sort());
  for (const [path, contents] of changes) writeFileSync(join(root, path), contents);
  assert.equal(readVersion(root), "0.2.0");
  assert.equal(readVersion(root, WORKER_VERSION_FILES), "0.5.0");
  const read = (path) => readFileSync(join(root, path), "utf8");
  assert.equal(read("Cargo.toml"), FIXTURE["Cargo.toml"].replace('version = "0.1.0"\nedition', 'version = "0.2.0"\nedition'));
  assert.equal(read("Cargo.lock"), FIXTURE["Cargo.lock"].replace('"hatoba-core"\nversion = "0.1.0"', '"hatoba-core"\nversion = "0.2.0"'));
  assert.match(read("apps/desktop/package.json"), /"react": "\^0\.1\.0"/);
});

test("a Worker bump changes only the Worker version", (t) => {
  const root = fixture(t);
  const changes = planBump(root, "0.5.1", WORKER_VERSION_FILES);
  assert.deepEqual([...changes.keys()].sort(), Object.keys(WORKER_FIXTURE).sort());
  for (const [path, contents] of changes) writeFileSync(join(root, path), contents);
  assert.equal(readVersion(root, WORKER_VERSION_FILES), "0.5.1");
  assert.equal(readVersion(root), "0.1.0");
  const read = (path) => readFileSync(join(root, path), "utf8");
  assert.equal(read("workers/sync/src/config.ts"), FIXTURE["workers/sync/src/config.ts"].replace('"0.5.0"', '"0.5.1"'));
  assert.equal(JSON.parse(read("workers/sync/package-lock.json")).packages["node_modules/hono"].version, "0.1.0");
});

test("names each file when the versions disagree", (t) => {
  const root = fixture(t, { "apps/desktop/package.json": npm({ name: "@hatoba/desktop", version: "0.1.1" }) });
  assert.throws(() => readVersion(root), (error) => error.message.includes("apps/desktop/package.json: 0.1.1") && error.message.includes("Cargo.toml: 0.1.0"));
});

test("refuses JSON that a rewrite would reformat", (t) => {
  const root = fixture(t, { "package.json": '{ "name": "hatoba", "version": "0.1.0" }\n' });
  assert.throws(() => readVersion(root), /^Error: package\.json: The file is not formatted as npm writes it/);
});

test("starts a release note without overwriting one", (t) => {
  const root = fixture(t);
  const changes = new Map();
  assert.equal(planNote(root, changes, "0.2.0", " Better links "), "docs/release-notes/0.2.0.md");
  assert.equal(changes.get("docs/release-notes/0.2.0.md"), "## Better links\n\n");
  assert.throws(() => planNote(root, changes, "0.2.0", "  "), /title on one line/);
  mkdirSync(join(root, "docs/release-notes"), { recursive: true });
  writeFileSync(join(root, "docs/release-notes/0.3.0.md"), "## Done\n\nBody.\n");
  assert.throws(() => planNote(root, new Map(), "0.3.0", "Again"), /already exists/);
});
