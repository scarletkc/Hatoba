import assert from "node:assert/strict";
import { test } from "node:test";
import { affectsRust } from "./changes.mjs";

test("Rust sources, manifests and the files they read need the Rust jobs", () => {
  for (const path of [
    "Cargo.toml",
    "Cargo.lock",
    "crates/hatoba-core/src/lib.rs",
    "crates/hatoba-ai/src/skills/builtin/hatoba/SKILL.md",
    "apps/desktop/src-tauri/src/lib.rs",
    "apps/desktop/src-tauri/tauri.conf.json",
    "apps/desktop/src/ipc/bindings.ts",
    "workers/sync/migrations/0001_init.sql",
    "scripts/ci/comment-width.mjs",
    ".github/workflows/ci.yml",
  ]) {
    assert.ok(affectsRust(path), path);
  }
});

test("the frontend, the Worker's code, docs and other workflows do not", () => {
  for (const path of [
    "apps/desktop/src/App.tsx",
    "apps/desktop/src/ipc/client.ts",
    "apps/desktop/package.json",
    "pnpm-lock.yaml",
    "workers/sync/src/index.ts",
    "docs/status.md",
    "README.md",
    "scripts/release/release.mjs",
    ".github/workflows/codeql.yml",
  ]) {
    assert.ok(!affectsRust(path), path);
  }
});

test("a directory input does not cover names that only start with it", () => {
  assert.ok(!affectsRust("crates"));
  assert.ok(!affectsRust("apps/desktop/src-tauri-notes.md"));
  assert.ok(!affectsRust("Cargo.toml.bak"));
});
