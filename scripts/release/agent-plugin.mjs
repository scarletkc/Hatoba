import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { ROOT } from "./version.mjs";

export const PACKAGE_DIR = "packaging/agent-plugin";
export const PACKAGE_FILES = ["cordis.patch.yml", "index.mjs", "README.md"];
export const MANIFESTS = ["package.json", ".claude-plugin/plugin.json", ".codex-plugin/plugin.json"];

export function versionedManifest(root, manifest) {
  const { name, ...fields } = JSON.parse(readFileSync(join(root, PACKAGE_DIR, manifest), "utf8"));
  const { version } = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
  return { name, version, ...fields };
}

export function writePackage(root, output) {
  if (existsSync(output) && readdirSync(output).length > 0) throw new Error(`${output} is not empty.`);
  for (const file of PACKAGE_FILES) cpSync(join(root, PACKAGE_DIR, file), join(output, file));
  for (const manifest of MANIFESTS) {
    mkdirSync(dirname(join(output, manifest)), { recursive: true });
    writeFileSync(join(output, manifest), `${JSON.stringify(versionedManifest(root, manifest), null, 2)}\n`);
  }
  cpSync(join(root, "skills/hatoba-cli"), join(output, "skills/hatoba-cli"), { recursive: true });
  cpSync(join(root, "LICENSE"), join(output, "LICENSE"));
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [output, ...rest] = process.argv.slice(2);
  if (output === undefined || rest.length > 0) {
    console.error("Usage: node scripts/release/agent-plugin.mjs OUTPUT");
    process.exit(2);
  }
  try {
    writePackage(ROOT, resolve(output));
    console.log(`Wrote the hatoba-agent-plugin package to ${output}`);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
