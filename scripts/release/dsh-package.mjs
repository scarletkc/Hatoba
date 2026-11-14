import { cpSync, existsSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import { ROOT } from "./version.mjs";

export const BUNDLE_FILES = [".dsh-plugin/index.mjs", ".dsh-plugin/cordis.patch.yml", "skills/hatoba-cli", "LICENSE"];

export function bundleManifest(root) {
  const { name, version, license, dsh } = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
  return {
    name,
    version,
    description: "Lets DeepSeek Harness run commands and copy files on the SSH hosts saved in Hatoba",
    keywords: ["dsh-plugin", "deepseek-harness", "hatoba", "ssh", "skill"],
    homepage: "https://hatoba.fog.moe",
    repository: { type: "git", url: "git+https://github.com/scarletkc/Hatoba.git" },
    license,
    dsh,
  };
}

export function writeBundle(root, output) {
  if (existsSync(output) && readdirSync(output).length > 0) throw new Error(`${output} is not empty.`);
  for (const path of BUNDLE_FILES) cpSync(join(root, path), join(output, path), { recursive: true });
  cpSync(join(root, ".dsh-plugin/README.md"), join(output, "README.md"));
  writeFileSync(join(output, "package.json"), `${JSON.stringify(bundleManifest(root), null, 2)}\n`);
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [output, ...rest] = process.argv.slice(2);
  if (output === undefined || rest.length > 0) {
    console.error("Usage: node scripts/release/dsh-package.mjs OUTPUT");
    process.exit(2);
  }
  try {
    writeBundle(ROOT, resolve(output));
    console.log(`Wrote the dsh-hatoba package to ${output}`);
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
