import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { writeBundle } from "./dsh-package.mjs";
import { ROOT } from "./version.mjs";

function output(t) {
  const dir = mkdtempSync(join(tmpdir(), "dsh-hatoba-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

test("the npm package holds the bundle and the skill, at the release version", (t) => {
  const dir = output(t);
  writeBundle(ROOT, dir);
  const root = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8"));
  const manifest = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
  assert.equal(manifest.name, root.name);
  assert.equal(manifest.version, root.version);
  assert.equal(manifest.private, undefined);
  assert.deepEqual(manifest.dsh, root.dsh);
  assert.deepEqual(readdirSync(dir).sort(), [".dsh-plugin", "LICENSE", "README.md", "package.json", "skills"]);
  assert.ok(existsSync(join(dir, "skills/hatoba-cli/SKILL.md")));
});

test("the bundle's patch mounts the plugin by the package's name", (t) => {
  const dir = output(t);
  writeBundle(ROOT, dir);
  const manifest = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
  const patch = readFileSync(join(dir, manifest.dsh.bundle.patch), "utf8");
  assert.ok(patch.includes(`name: "${manifest.name}/.dsh-plugin/index.mjs"`), patch);
  assert.ok(existsSync(join(dir, ".dsh-plugin/index.mjs")));
});

test("the package is written only to an empty folder", (t) => {
  const dir = output(t);
  writeFileSync(join(dir, "left-over.txt"), "");
  assert.throws(() => writeBundle(ROOT, dir), /is not empty/);
});
