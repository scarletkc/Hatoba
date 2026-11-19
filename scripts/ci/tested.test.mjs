import assert from "node:assert/strict";
import { test } from "node:test";
import { mergedPull, uncovered } from "./tested.mjs";

const SHA = "094fb5f379982d034c625168d15c3224d4f64ee9";
const TREE = "1c95339fb0ca5e6cdbce854d4aa9cd92dc59a6df";
const pull = { number: 450, merge_commit_sha: SHA, head: { sha: "bb08643e7fa49b4e8f63e890a3ebb3169c298e65" } };
const run = (fields = {}) => ({
  event: "pull_request",
  status: "completed",
  conclusion: "success",
  created_at: "2026-11-19T19:37:57Z",
  html_url: "https://github.com/scarletkc/Hatoba/actions/runs/1",
  head_commit: { tree_id: TREE },
  ...fields,
});
const check = (fields = {}) => uncovered({ pull, tree: TREE, behindBy: 0, runs: [run()], ...fields });

test("finds the pull request whose merge made the commit", () => {
  const other = { number: 449, merge_commit_sha: "3cac1fe402e8dfd4063338f359b4be5ee77bc1f9" };
  assert.equal(mergedPull([other, pull], SHA), pull);
  assert.equal(mergedPull([other], SHA), null);
  assert.equal(mergedPull([], SHA), null);
});

test("a pull request that passed CI on the same files covers the commit", () => {
  assert.equal(check(), null);
});

test("a pull request merged behind main does not", () => {
  assert.match(check({ behindBy: 2 }), /#450 was merged 2 commits behind main/);
  assert.match(check({ behindBy: undefined }), /behind main/);
});

test("nor does one whose last run has not passed", () => {
  assert.match(check({ runs: [] }), /CI has not run on the last commit of #450/);
  assert.match(check({ runs: [run({ event: "push" })] }), /CI has not run/);
  assert.match(check({ runs: [run({ conclusion: "failure" })] }), /has not passed: https:/);
  assert.match(check({ runs: [run({ status: "in_progress", conclusion: null })] }), /has not passed/);
  const rerun = run({ created_at: "2026-11-19T19:40:00Z", conclusion: "cancelled" });
  assert.match(check({ runs: [run(), rerun] }), /has not passed/);
  assert.equal(check({ runs: [run({ created_at: "2026-11-19T19:30:00Z", conclusion: "failure" }), run()] }), null);
});

test("nor one whose run was on other files", () => {
  const other = { tree_id: "0123456789abcdef0123456789abcdef01234567" };
  assert.match(check({ runs: [run({ head_commit: other })] }), /was on other files/);
  assert.match(check({ runs: [run({ head_commit: null })] }), /was on other files/);
});
