import assert from "node:assert/strict";
import { test } from "node:test";
import { FORMAT, buildPackage } from "./bundle.mjs";

const CONFIG = {
  name: "hatoba-sync",
  compatibility_date: "2026-09-01",
  compatibility_flags: [],
  d1_databases: [{ binding: "DB", database_name: "hatoba", database_id: "00000000-0000-0000-0000-000000000000", migrations_dir: "migrations" }],
  ratelimits: [{ name: "AUTH_LIMITER", namespace_id: "1001", simple: { limit: 10, period: 60 } }],
};
const OUTPUT_FILES = { "index.js": "export default {};", "index.js.map": "{}", "README.md": "wrangler output" };
const MIGRATIONS = [
  { name: "0002_more.sql", sql: "CREATE TABLE b (x);" },
  { name: "0001_init.sql", sql: "CREATE TABLE a (x);" },
];

const build = (overrides = {}) => buildPackage({ config: CONFIG, version: "0.1.0", outputFiles: OUTPUT_FILES, migrations: MIGRATIONS, ...overrides });

test("packages the module, the migrations in file-name order, and the binding settings", () => {
  assert.deepEqual(build(), {
    format: FORMAT,
    version: "0.1.0",
    name: "hatoba-sync",
    compatibility_date: "2026-09-01",
    compatibility_flags: [],
    module: { name: "index.js", content: "export default {};" },
    migrations: [
      { name: "0001_init.sql", sql: "CREATE TABLE a (x);" },
      { name: "0002_more.sql", sql: "CREATE TABLE b (x);" },
    ],
    d1: { binding: "DB", database_name: "hatoba" },
    ratelimit: { name: "AUTH_LIMITER", namespace_id: "1001", limit: 10, period: 60 },
  });
});

test("refuses output with more than one module, or without index.js", () => {
  assert.throws(() => build({ outputFiles: { ...OUTPUT_FILES, "chunk.wasm": "" } }), /more than one module \(chunk\.wasm\)/);
  assert.throws(() => build({ outputFiles: { "index.js.map": "{}" } }), /did not produce index\.js/);
});

test("refuses a configuration with other or missing bindings", () => {
  const db = CONFIG.d1_databases[0];
  assert.throws(() => build({ config: { ...CONFIG, d1_databases: [] } }), /exactly one D1 binding, named DB/);
  assert.throws(() => build({ config: { ...CONFIG, d1_databases: [{ ...db, binding: "STORE" }] } }), /exactly one D1 binding, named DB/);
  assert.throws(() => build({ config: { ...CONFIG, d1_databases: [db, { ...db, binding: "OTHER" }] } }), /exactly one D1 binding/);
  assert.throws(() => build({ config: { ...CONFIG, ratelimits: undefined } }), /exactly one rate limit binding, named AUTH_LIMITER/);
});

test("refuses a Worker without migrations", () => {
  assert.throws(() => build({ migrations: [] }), /no \.sql files/);
});
