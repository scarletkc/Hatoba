// Packages the sync Worker for in-app deployment (spec §6.7, Worker bundle): the module that
// `wrangler deploy --dry-run` builds, the D1 migrations, and a manifest from wrangler.toml. The Rust
// shell embeds the package, so it is generated at build time and never committed.
//
//   node scripts/worker/bundle.mjs [--no-install]
//
// --no-install skips `npm ci` in workers/sync and uses the dependencies already installed there.
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = fileURLToPath(new URL("../..", import.meta.url));
const WORKER_DIR = join(ROOT, "workers/sync");
const MIGRATIONS_DIR = join(WORKER_DIR, "migrations");
/** Read by apps/desktop/src-tauri/build.rs. */
export const OUTPUT = join(ROOT, "apps/desktop/src-tauri/gen/worker/bundle.json");

export const FORMAT = 1;
/** The Worker code reads its bindings by these names, and the app recognizes a Hatoba Worker by them. */
export const D1_BINDING = "DB";
export const RATE_LIMIT_BINDING = "AUTH_LIMITER";
const MAIN_MODULE = "index.js";

function only(list = [], key, name, what) {
  const matches = list.filter((entry) => entry[key] === name);
  if (matches.length !== 1 || list.length !== 1) {
    throw new Error(`wrangler.toml must have exactly one ${what}, named ${name}; the app deploys no other.`);
  }
  return matches[0];
}

/**
 * Builds the package from the parsed wrangler configuration, the Worker version, the files that
 * wrangler wrote to its output directory, and the migrations as `{ name, sql }`.
 */
export function buildPackage({ config, version, outputFiles, migrations }) {
  const extra = Object.keys(outputFiles).filter((name) => name !== MAIN_MODULE && name !== "README.md" && !name.endsWith(".map"));
  if (extra.length > 0) throw new Error(`wrangler produced more than one module (${extra.join(", ")}); the app uploads only ${MAIN_MODULE}.`);
  const content = outputFiles[MAIN_MODULE];
  if (!content) throw new Error(`wrangler did not produce ${MAIN_MODULE}.`);
  if (migrations.length === 0) throw new Error("workers/sync/migrations has no .sql files.");

  const d1 = only(config.d1_databases, "binding", D1_BINDING, "D1 binding");
  const limiter = only(config.ratelimits, "name", RATE_LIMIT_BINDING, "rate limit binding");
  return {
    format: FORMAT,
    version,
    name: config.name,
    compatibility_date: config.compatibility_date,
    compatibility_flags: config.compatibility_flags ?? [],
    module: { name: MAIN_MODULE, content },
    migrations: [...migrations].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0)),
    d1: { binding: d1.binding, database_name: d1.database_name },
    ratelimit: {
      name: limiter.name,
      namespace_id: limiter.namespace_id,
      limit: limiter.simple.limit,
      period: limiter.simple.period,
    },
  };
}

function run(command, args, options) {
  const result = spawnSync(command, args, { cwd: WORKER_DIR, stdio: "inherit", ...options });
  if (result.status !== 0) throw new Error(`${command} ${args.join(" ")} failed${result.error ? `: ${result.error.message}` : ""}.`);
}

function readMigrations() {
  return readdirSync(MIGRATIONS_DIR)
    .filter((name) => name.endsWith(".sql"))
    .map((name) => ({ name, sql: readFileSync(join(MIGRATIONS_DIR, name), "utf8") }));
}

function main(argv) {
  if (!argv.includes("--no-install")) {
    // npm is a .cmd script on Windows, which Node starts only through a shell.
    run("npm", ["ci", "--no-audit", "--no-fund"], { shell: process.platform === "win32" });
  }
  const outdir = mkdtempSync(join(tmpdir(), "hatoba-worker-"));
  try {
    const wrangler = join(WORKER_DIR, "node_modules/wrangler/bin/wrangler.js");
    run(process.execPath, [wrangler, "deploy", "--dry-run", "--outdir", outdir], {
      env: { ...process.env, WRANGLER_SEND_METRICS: "false" },
    });
    const outputFiles = Object.fromEntries(readdirSync(outdir).map((name) => [name, readFileSync(join(outdir, name), "utf8")]));
    const require = createRequire(join(WORKER_DIR, "package.json"));
    const config = require("wrangler").unstable_readConfig({ config: join(WORKER_DIR, "wrangler.toml") });
    const { version } = JSON.parse(readFileSync(join(WORKER_DIR, "package.json"), "utf8"));
    const pkg = buildPackage({ config, version, outputFiles, migrations: readMigrations() });
    mkdirSync(dirname(OUTPUT), { recursive: true });
    writeFileSync(OUTPUT, JSON.stringify(pkg));
    console.log(`Worker ${pkg.version} bundled to ${OUTPUT}`);
  } finally {
    rmSync(outdir, { recursive: true, force: true });
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  try {
    main(process.argv.slice(2));
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
