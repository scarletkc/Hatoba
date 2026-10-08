// Checks that a change to the sync Worker raises the Worker version (docs/hatoba-spec.md §6.7,
// Versions). `node scripts/release/bump.mjs --worker` raises it.
// Usage: node scripts/ci/worker-version.mjs [BASE]
// Compares the working tree with the commit where HEAD branched from BASE, origin/main by default.
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { ROOT, WORKER_VERSION_FILES, compareVersions, nextVersion, readVersion } from "../release/version.mjs";

const WORKER = "workers/sync";
/** Every file in these goes into the bundle or the migrations. */
const INPUT_DIRS = [`${WORKER}/src`, `${WORKER}/migrations`];
const WRANGLER = `${WORKER}/wrangler.toml`;
const PACKAGE = `${WORKER}/package.json`;
const LOCK = `${WORKER}/package-lock.json`;
const TSCONFIG = `${WORKER}/tsconfig.json`;
/**
 * The tsconfig.json fields that esbuild reads when wrangler bundles the Worker
 * (https://esbuild.github.io/content-types/#tsconfig-json). It ignores the type-checking options.
 */
const BUNDLING_OPTIONS = [
  "alwaysStrict",
  "baseUrl",
  "experimentalDecorators",
  "importsNotUsedAsValues",
  "jsx",
  "jsxFactory",
  "jsxFragmentFactory",
  "jsxImportSource",
  "paths",
  "preserveValueImports",
  "strict",
  "target",
  "useDefineForClassFields",
  "verbatimModuleSyntax",
];

function git(args, root) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"], maxBuffer: 64 * 1024 * 1024 });
}

const nulSeparated = (output) => output.split("\0").filter(Boolean);

/** The files in the working tree that git tracks or would track. */
function workingTree(root) {
  const read = (path) => (existsSync(join(root, path)) ? readFileSync(join(root, path), "utf8") : null);
  const list = (dir) =>
    nulSeparated(git(["ls-files", "-z", "--cached", "--others", "--exclude-standard", "--", dir], root)).filter((path) => existsSync(join(root, path)));
  return { read, list };
}

/** The files in a commit. */
function commitTree(root, commit) {
  const list = (dir) => nulSeparated(git(["ls-tree", "-r", "-z", "--name-only", commit, "--", dir], root));
  const files = new Set(list(WORKER));
  const read = (path) => (files.has(path) ? git(["show", `${commit}:${path}`], root) : null);
  return { read, list };
}

const versionFiles = new Map(WORKER_VERSION_FILES.map((file) => [file.path, file]));

/** The part of tsconfig.json that can change the bundle, or the whole file when it is not plain JSON. */
function bundlingOptions(text) {
  if (text === null) return null;
  let config;
  try {
    config = JSON.parse(text);
  } catch {
    // tsconfig.json may hold comments, which JSON.parse rejects.
    return text.replaceAll("\r\n", "\n");
  }
  const options = config.compilerOptions ?? {};
  return JSON.stringify({ extends: config.extends, ...Object.fromEntries(BUNDLING_OPTIONS.filter((key) => key in options).map((key) => [key, options[key]])) });
}

/**
 * Returns what the Worker's bundle and migrations are made from in `tree`, by path: every file in
 * INPUT_DIRS, wrangler.toml, the dependencies in package.json, the packages in package-lock.json
 * that are not only for devDependencies, and the BUNDLING_OPTIONS in tsconfig.json. Version fields
 * read as 0.0.0, so that raising the version changes nothing here, and line endings as LF.
 */
function workerInputs(tree) {
  const inputs = new Map();
  for (const path of [...INPUT_DIRS.flatMap((dir) => tree.list(dir)), WRANGLER]) {
    let contents = tree.read(path)?.replaceAll("\r\n", "\n") ?? null;
    const versionFile = versionFiles.get(path);
    if (contents !== null && versionFile) {
      try {
        contents = versionFile.write(contents, "0.0.0");
      } catch (error) {
        throw new Error(`${path}: ${error.message}`);
      }
    }
    inputs.set(path, contents);
  }
  const json = (path) => JSON.parse(tree.read(path) ?? "{}");
  inputs.set(`${PACKAGE} (dependencies)`, JSON.stringify(json(PACKAGE).dependencies ?? {}));
  // npm marks the packages that only devDependencies need, such as wrangler and vitest, as dev.
  const packages = Object.entries(json(LOCK).packages ?? {}).filter(([path, entry]) => path !== "" && !entry.dev && !entry.devOptional);
  inputs.set(`${LOCK} (dependencies)`, JSON.stringify(packages));
  inputs.set(`${TSCONFIG} (bundling options)`, bundlingOptions(tree.read(TSCONFIG)));
  return inputs;
}

/** Lists, in order, the keys whose values differ between two results of workerInputs. */
function changedInputs(before, after) {
  return [...new Set([...before.keys(), ...after.keys()])].filter((key) => before.get(key) !== after.get(key)).sort();
}

/**
 * Compares the Worker in the working tree at `root` with the commit where HEAD branched from `base`.
 * Returns whether its version follows the rule, and a message that says why.
 */
export function checkWorkerVersion(root, base) {
  let commit;
  try {
    commit = git(["merge-base", "HEAD", base], root).trim();
  } catch {
    throw new Error(`Cannot find where HEAD branched from ${base}. Fetch ${base}, or pass the branch or commit to compare with, such as upstream/main.`);
  }
  const since = `${base} (${commit.slice(0, 7)})`;
  const version = readVersion(root, WORKER_VERSION_FILES);
  const before = commitTree(root, commit);
  const basePackage = before.read(PACKAGE);
  if (basePackage === null) return { ok: true, message: `${since} has no sync Worker to compare with.` };
  const baseVersion = JSON.parse(basePackage).version;

  const order = compareVersions(version, baseVersion);
  if (order > 0) return { ok: true, message: `The sync Worker version goes from ${baseVersion} to ${version}.` };
  if (order < 0) {
    return {
      ok: false,
      message: `The sync Worker version goes down from ${baseVersion} at ${since} to ${version}. Set a version above ${baseVersion}, for example with:\n  node scripts/release/bump.mjs --worker ${nextVersion(baseVersion, "patch")}`,
    };
  }
  const changed = changedInputs(workerInputs(before), workerInputs(workingTree(root)));
  if (changed.length === 0) return { ok: true, message: `The sync Worker has not changed since ${since}, and its version stays ${version}.` };
  return {
    ok: false,
    message: [
      `The sync Worker changed since ${since}, but its version is still ${version}. Changed:`,
      ...changed.map((path) => `  ${path}`),
      "Raise the Worker version in the same pull request, for example with:",
      "  node scripts/release/bump.mjs --worker patch",
    ].join("\n"),
  };
}

function main(argv) {
  const { positionals } = parseArgs({ args: argv, allowPositionals: true });
  if (positionals.length > 1) {
    console.error("usage: node scripts/ci/worker-version.mjs [BASE]");
    return 2;
  }
  let result;
  try {
    result = checkWorkerVersion(ROOT, positionals[0] ?? "origin/main");
  } catch (error) {
    result = { ok: false, message: error.message };
  }
  if (result.ok) {
    console.log(result.message);
    return 0;
  }
  if (process.env.GITHUB_ACTIONS === "true") {
    const data = result.message.replaceAll("%", "%25").replaceAll("\r", "%0D").replaceAll("\n", "%0A");
    console.log(`::error title=Worker version::${data}`);
  } else {
    console.error(result.message);
  }
  return 1;
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  process.exitCode = main(process.argv.slice(2));
}
