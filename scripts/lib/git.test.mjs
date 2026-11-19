import assert from "node:assert/strict";
import { existsSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { ROOT, git, gitRaw, lsFiles, nulSeparated } from "./git.mjs";

test("ROOT points to the repository root", () => {
  assert.ok(existsSync(join(ROOT, "Cargo.toml")));
  assert.ok(existsSync(join(ROOT, "package.json")));
});

test("git trims output, gitRaw keeps raw output, and lsFiles skips deleted tracked files", (t) => {
  const root = mkdtempSync(join(tmpdir(), "hatoba-git-lib-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  git(["init", "--quiet"], root);
  git(["config", "user.name", "Git Lib Tests"], root);
  git(["config", "user.email", "tests@example.com"], root);
  git(["config", "commit.gpgsign", "false"], root);
  writeFileSync(join(root, "a.txt"), "hello\n");
  writeFileSync(join(root, "b.rs"), "// rust\n");
  git(["add", "a.txt"], root);
  git(["commit", "--quiet", "-m", "init"], root);

  assert.equal(gitRaw(["show", "HEAD:a.txt"], root), "hello\n");
  assert.equal(git(["rev-parse", "--is-inside-work-tree"], root), "true");
  assert.deepEqual(nulSeparated("a.txt\0b.rs\0"), ["a.txt", "b.rs"]);

  assert.deepEqual(lsFiles([], root).sort(), ["a.txt", "b.rs"]);
  assert.deepEqual(lsFiles("*.rs", root), ["b.rs"]);
  rmSync(join(root, "a.txt"));
  assert.deepEqual(lsFiles([], root), ["b.rs"]);
});
