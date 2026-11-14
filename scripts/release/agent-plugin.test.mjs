import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { MANIFESTS, PACKAGE_DIR, writePackage } from "./agent-plugin.mjs";
import { ROOT } from "./version.mjs";

const readJson = (path) => JSON.parse(readFileSync(path, "utf8"));

function output(t) {
  const dir = mkdtempSync(join(tmpdir(), "hatoba-agent-plugin-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  return dir;
}

test("the npm package holds the plugins and the skill, at the release version", (t) => {
  const dir = output(t);
  writePackage(ROOT, dir);
  const version = readJson(join(ROOT, "package.json")).version;
  for (const manifest of MANIFESTS) {
    const source = readJson(join(ROOT, PACKAGE_DIR, manifest));
    assert.deepEqual(readJson(join(dir, manifest)), { ...source, version }, manifest);
  }
  assert.deepEqual(readdirSync(dir).sort(), [
    ".claude-plugin",
    ".codex-plugin",
    "LICENSE",
    "README.md",
    "cordis.patch.yml",
    "index.mjs",
    "package.json",
    "skills",
  ]);
  assert.ok(existsSync(join(dir, "skills/hatoba-cli/SKILL.md")));
});

test("the DeepSeek Harness patch mounts the plugin by the package's name", (t) => {
  const dir = output(t);
  writePackage(ROOT, dir);
  const manifest = readJson(join(dir, "package.json"));
  const patch = readFileSync(join(dir, manifest.dsh.bundle.patch), "utf8");
  assert.ok(patch.includes(`name: "${manifest.name}/index.mjs"`), patch);
});

test("the Claude Code and Codex marketplaces install the plugin from the npm package", () => {
  const packageName = readJson(join(ROOT, PACKAGE_DIR, "package.json")).name;
  const pluginNames = [".claude-plugin/plugin.json", ".codex-plugin/plugin.json"].map(
    (manifest) => readJson(join(ROOT, PACKAGE_DIR, manifest)).name,
  );
  for (const marketplace of [".claude-plugin/marketplace.json", ".agents/plugins/marketplace.json"]) {
    const { plugins } = readJson(join(ROOT, marketplace));
    assert.equal(plugins.length, 1, marketplace);
    assert.deepEqual(plugins[0].source, { source: "npm", package: packageName }, marketplace);
    for (const name of pluginNames) assert.equal(plugins[0].name, name, marketplace);
  }
});

test("the package is written only to an empty folder", (t) => {
  const dir = output(t);
  writeFileSync(join(dir, "left-over.txt"), "");
  assert.throws(() => writePackage(ROOT, dir), /is not empty/);
});
