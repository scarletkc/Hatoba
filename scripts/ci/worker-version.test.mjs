import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { WORKER_VERSION_FILES, planBump } from "../release/version.mjs";
import { checkWorkerVersion } from "./worker-version.mjs";

const npm = (json) => `${JSON.stringify(json, null, 2)}\n`;

const packageJson = (version, { dependencies = { hono: "^4.0.0" }, devDependencies = { vitest: "^4.0.0" } } = {}) =>
  npm({ name: "@hatoba/sync-worker", version, private: true, scripts: { test: "vitest run" }, dependencies, devDependencies });

const packageLock = (version, packages = {}) =>
  npm({
    name: "@hatoba/sync-worker",
    version,
    lockfileVersion: 3,
    requires: true,
    packages: {
      "": { name: "@hatoba/sync-worker", version, dependencies: { hono: "^4.0.0" }, devDependencies: { vitest: "^4.0.0" } },
      "node_modules/hono": { version: "4.0.0", license: "MIT" },
      "node_modules/vitest": { version: "4.0.0", dev: true, license: "MIT" },
      ...packages,
    },
  });

const tsconfig = (compilerOptions = {}, include = ["src", "test"]) =>
  npm({ compilerOptions: { target: "ES2022", strict: true, verbatimModuleSyntax: true, noEmit: true, ...compilerOptions }, include });

const configTs = (version) => `export const API_VERSION = 1;\n/** Keep in sync with package.json (enforced by a test). */\nexport const VERSION = "${version}";\n`;

const WORKER = {
  "workers/sync/package.json": packageJson("0.1.0"),
  "workers/sync/package-lock.json": packageLock("0.1.0"),
  "workers/sync/src/config.ts": configTs("0.1.0"),
  "workers/sync/src/index.ts": 'export default { fetch: () => new Response("ok") };\n',
  "workers/sync/migrations/0001_init.sql": "CREATE TABLE items (id TEXT);\n",
  "workers/sync/wrangler.toml": 'name = "hatoba-sync"\nmain = "src/index.ts"\n',
  "workers/sync/tsconfig.json": tsconfig(),
  "workers/sync/test/index.test.ts": "// test\n",
  "workers/sync/README.md": "# Sync Worker\n",
};

/** A repository whose main branch holds `files`, checked out on a feature branch. */
function repository(t, files = WORKER) {
  const root = mkdtempSync(join(tmpdir(), "hatoba-worker-version-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const run = (...args) => execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] });
  const write = (path, contents) => {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), contents);
  };
  const commit = (message) => {
    run("add", "--all");
    run("commit", "--quiet", "-m", message);
  };
  run("init", "--quiet", "--initial-branch", "main");
  run("config", "user.name", "Worker Version Tests");
  run("config", "user.email", "tests@example.com");
  run("config", "commit.gpgsign", "false");
  run("config", "core.autocrlf", "false");
  for (const [path, contents] of Object.entries(files)) write(path, contents);
  commit("Initial commit");
  run("switch", "--quiet", "--create", "feature");
  const raise = (to) => {
    for (const [path, contents] of planBump(root, to, WORKER_VERSION_FILES)) write(path, contents);
  };
  const check = () => checkWorkerVersion(root, "main");
  return { root, run, write, commit, raise, check };
}

test("passes when the Worker did not change", (t) => {
  const repo = repository(t);
  repo.write("workers/sync/README.md", "# Sync Worker\n\nMore.\n");
  repo.write("workers/sync/test/index.test.ts", "// another test\n");
  repo.write("README.md", "# Hatoba\n");
  const result = repo.check();
  assert.ok(result.ok, result.message);
  assert.match(result.message, /has not changed since main \([0-9a-f]{7}\), and its version stays 0\.1\.0/);
});

test("fails when the Worker changes without a version raise, and names each change", (t) => {
  const changes = {
    "workers/sync/src/index.ts": ["workers/sync/src/index.ts", 'export default { fetch: () => new Response("changed") };\n'],
    "a new source file": ["workers/sync/src/routes/new.ts", "export {};\n"],
    "a new migration": ["workers/sync/migrations/0002_more.sql", "ALTER TABLE items ADD COLUMN v INTEGER;\n"],
    "wrangler.toml": ["workers/sync/wrangler.toml", 'name = "hatoba-sync"\nmain = "src/index.ts"\ncompatibility_date = "2026-10-01"\n'],
    "config.ts outside VERSION": ["workers/sync/src/config.ts", configTs("0.1.0").replace("API_VERSION = 1", "API_VERSION = 2")],
  };
  for (const [name, [path, contents]] of Object.entries(changes)) {
    const repo = repository(t);
    repo.write(path, contents);
    const result = repo.check();
    assert.equal(result.ok, false, name);
    assert.ok(result.message.startsWith("The sync Worker changed since main ("), result.message);
    assert.ok(result.message.includes(`but its version is still 0.1.0. Changed:\n  ${path}\nRaise the Worker version`), result.message);
    assert.ok(result.message.endsWith("node scripts/release/bump.mjs --worker patch"), result.message);
  }
});

test("counts the dependencies the bundle includes, and not devDependencies", (t) => {
  const runtime = repository(t);
  runtime.write("workers/sync/package.json", packageJson("0.1.0", { dependencies: { hono: "^4.1.0" } }));
  runtime.write("workers/sync/package-lock.json", packageLock("0.1.0", { "node_modules/hono": { version: "4.1.0", license: "MIT" } }));
  const result = runtime.check();
  assert.equal(result.ok, false);
  assert.ok(result.message.includes("Changed:\n  workers/sync/package-lock.json (dependencies)\n  workers/sync/package.json (dependencies)\n"), result.message);

  const dev = repository(t);
  dev.write("workers/sync/package.json", packageJson("0.1.0", { devDependencies: { vitest: "^4.1.0", wrangler: "^4.0.0" } }));
  dev.write(
    "workers/sync/package-lock.json",
    packageLock("0.1.0", {
      "node_modules/vitest": { version: "4.1.0", dev: true, license: "MIT" },
      "node_modules/wrangler": { version: "4.0.0", dev: true, license: "MIT" },
      "node_modules/fsevents": { version: "2.3.3", devOptional: true, license: "MIT" },
    }),
  );
  assert.ok(dev.check().ok, dev.check().message);
});

test("counts the tsconfig.json options esbuild reads, and not the type-checking ones", (t) => {
  for (const [name, contents] of Object.entries({
    "a bundling option": tsconfig({ useDefineForClassFields: false }),
    "extends": npm({ extends: "./base.json", ...JSON.parse(tsconfig()) }),
    "comments": `// Bundling\n${tsconfig()}`,
  })) {
    const repo = repository(t);
    repo.write("workers/sync/tsconfig.json", contents);
    const result = repo.check();
    assert.equal(result.ok, false, name);
    assert.ok(result.message.includes("Changed:\n  workers/sync/tsconfig.json (bundling options)\n"), result.message);
  }

  const typeCheck = repository(t);
  typeCheck.write("workers/sync/tsconfig.json", tsconfig({ noUncheckedIndexedAccess: true, noEmit: false }, ["src", "test", "vitest.config.ts"]));
  assert.ok(typeCheck.check().ok, typeCheck.check().message);
});

test("passes once the Worker version is raised", (t) => {
  const repo = repository(t);
  repo.write("workers/sync/src/index.ts", 'export default { fetch: () => new Response("changed") };\n');
  repo.raise("0.1.1");
  repo.commit("Change the Worker");
  repo.write("workers/sync/migrations/0002_more.sql", "ALTER TABLE items ADD COLUMN v INTEGER;\n");
  const result = repo.check();
  assert.ok(result.ok, result.message);
  assert.equal(result.message, "The sync Worker version goes from 0.1.0 to 0.1.1.");
});

test("ignores line endings", (t) => {
  const repo = repository(t);
  repo.write("workers/sync/src/index.ts", WORKER["workers/sync/src/index.ts"].replaceAll("\n", "\r\n"));
  repo.write("workers/sync/src/config.ts", configTs("0.1.0").replaceAll("\n", "\r\n"));
  assert.ok(repo.check().ok, repo.check().message);
});

test("compares with where the branch left the base, not with the base's tip", (t) => {
  const repo = repository(t);
  repo.write("workers/sync/src/index.ts", 'export default { fetch: () => new Response("feature") };\n');
  repo.commit("Change the Worker on the feature branch");
  repo.run("switch", "--quiet", "main");
  repo.raise("0.2.0");
  repo.commit("Raise the Worker version on main");
  repo.run("switch", "--quiet", "feature");
  const result = repo.check();
  assert.equal(result.ok, false);
  assert.ok(result.message.includes("but its version is still 0.1.0. Changed:\n  workers/sync/src/index.ts\n"), result.message);
});

test("refuses a lower version", (t) => {
  const repo = repository(t);
  repo.raise("0.2.0");
  repo.commit("Raise the Worker version");
  repo.run("branch", "--force", "main");
  for (const [path, contents] of Object.entries(WORKER)) repo.write(path, contents);
  const result = repo.check();
  assert.equal(result.ok, false);
  assert.ok(result.message.startsWith("The sync Worker version goes down from 0.2.0 at main ("), result.message);
  assert.ok(result.message.endsWith("node scripts/release/bump.mjs --worker 0.2.1"), result.message);
});

test("names each Worker version file when they disagree", (t) => {
  const repo = repository(t);
  repo.write("workers/sync/package.json", packageJson("0.1.1"));
  assert.throws(
    () => repo.check(),
    (error) => error.message.includes("workers/sync/package.json: 0.1.1") && error.message.includes("workers/sync/src/config.ts: 0.1.0"),
  );
});

test("explains a base it cannot find", (t) => {
  const repo = repository(t);
  assert.throws(() => checkWorkerVersion(repo.root, "upstream/main"), /^Error: Cannot find where HEAD branched from upstream\/main\. Fetch upstream\/main/);
});

test("passes when the base has no Worker", (t) => {
  const repo = repository(t, { "README.md": "# Hatoba\n" });
  for (const [path, contents] of Object.entries(WORKER)) repo.write(path, contents);
  const result = repo.check();
  assert.ok(result.ok, result.message);
  assert.match(result.message, /^main \([0-9a-f]{7}\) has no sync Worker to compare with\.$/);
});
