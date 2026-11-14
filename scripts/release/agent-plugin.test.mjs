import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { PACKAGE_DIR, writePackage } from "./agent-plugin.mjs";
import { ROOT } from "./version.mjs";

const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));

function output(t) {
  const dir = mkdtempSync(join(tmpdir(), "hatoba-agent-plugin-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

test("the npm package holds the bundle and the skill, at the release version", (t) => {
  const dir = output(t);
  writePackage(ROOT, dir);
  const source = readJson(join(ROOT, PACKAGE_DIR, "package.json"));
  const manifest = readJson(join(dir, "package.json"));
  assert.deepEqual(manifest, { ...source, version: readJson(join(ROOT, "package.json")).version });
  assert.deepEqual(readdirSync(dir).sort(), [
    "LICENSE",
    "README.md",
    "cordis.patch.yml",
    "index.mjs",
    "package.json",
    "skills",
  ]);
  assert.ok(existsSync(join(dir, "skills/hatoba-cli/SKILL.md")));
});

test("the bundle's patch mounts the plugin by the package's name", (t) => {
  const dir = output(t);
  writePackage(ROOT, dir);
  const manifest = readJson(join(dir, "package.json"));
  const patch = readFileSync(join(dir, manifest.dsh.bundle.patch), "utf8");
  assert.ok(patch.includes(`name: "${manifest.name}/index.mjs"`), patch);
});

test("the package is written only to an empty folder", (t) => {
  const dir = output(t);
  writeFileSync(join(dir, "left-over.txt"), "");
  assert.throws(() => writePackage(ROOT, dir), /is not empty/);
});
