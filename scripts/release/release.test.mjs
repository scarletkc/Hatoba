import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { CHECKSUMS, ciCovers, collectDist, composeNotes, git, releaseTarget } from "./release.mjs";

const REPOSITORY = "scarletkc/Hatoba";

function repository(t) {
  const root = mkdtempSync(join(tmpdir(), "hatoba-release-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const run = (...args) => git(args, root);
  run("init", "--quiet");
  run("config", "user.name", "Release Tests");
  run("config", "user.email", "tests@example.com");
  run("config", "commit.gpgsign", "false");
  run("config", "tag.gpgsign", "false");
  let count = 0;
  const write = (path, contents) => {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), contents);
  };
  const commit = (subject, { body, path = "src/main.rs" } = {}) => {
    write(path, `${(count += 1)}\n`);
    run("add", "--all");
    run("commit", "--quiet", "-m", subject, ...(body ? ["-m", body] : []));
    return run("rev-parse", "HEAD");
  };
  const note = (version, text) => write(`docs/release-notes/${version}.md`, text);
  return { root, run, commit, note };
}

test("composes the note, install instructions, and a changelog grouped by type", (t) => {
  const repo = repository(t);
  repo.commit("Initial commit");
  repo.run("tag", "v0.1.0");
  repo.commit("feat(desktop): show the app version (#7)");
  repo.commit("docs: describe releases (#9)", { path: "docs/releasing.md" });
  const perf = repo.commit("perf(ssh): render output in batches");
  repo.commit("fix: keep the active tab (#8)");
  repo.commit("Add a WIP terminal view");
  repo.note("0.2.0", "## Version display\n\nThe settings show the version.\n");

  const notes = composeNotes("0.2.0", REPOSITORY, repo.root);
  assert.ok(notes.startsWith("## Version display\n\nThe settings show the version.\n\n## Install\n\nDownload `Hatoba_0.2.0_x64-setup.exe`"));
  const features = notes.indexOf("### Features\n\n- **desktop:** show the app version (#7)\n");
  const fixes = notes.indexOf("### Fixes\n\n- keep the active tab (#8)\n");
  const performance = notes.indexOf(`### Performance\n\n- **ssh:** render output in batches (${perf.slice(0, 7)}`);
  assert.ok(features > 0 && fixes > features && performance > fixes, notes);
  assert.match(notes, /<summary>Other changes<\/summary>\n\n- docs: describe releases \(#9\)\n- Add a WIP terminal view \([0-9a-f]{7,}\)\n\n<\/details>/);
  assert.ok(notes.includes("Changes since v0.1.0:"));
  assert.ok(notes.endsWith(`[Full diff](https://github.com/${REPOSITORY}/compare/v0.1.0...v0.2.0)\n`));
  assert.ok(!notes.includes("Initial commit"));
});

test("lists breaking changes first", (t) => {
  const repo = repository(t);
  repo.commit("Initial commit");
  repo.run("tag", "v0.1.0");
  repo.commit("feat: add a feature (#3)");
  repo.commit("feat(worker)!: require API 2 (#4)");
  repo.commit("fix(core): store keys in a new format (#5)", { body: "BREAKING CHANGE: older apps cannot read the new format." });

  const notes = composeNotes("0.2.0", REPOSITORY, repo.root);
  assert.ok(notes.includes("### Breaking changes\n\n- **worker:** require API 2 (#4)\n- **core:** store keys in a new format (#5)\n\n### Features\n\n- add a feature (#3)"), notes);
});

test("compares a stable release with the previous stable release and a prerelease with any release", (t) => {
  const repo = repository(t);
  repo.commit("Initial commit");
  repo.run("tag", "v0.1.0");
  repo.run("switch", "--quiet", "-c", "side");
  repo.commit("fix: unreleased side branch");
  repo.run("tag", "v0.1.5");
  repo.run("switch", "--quiet", "-");
  repo.commit("feat: preview (#2)");
  repo.run("tag", "v0.2.0-rc.1");
  repo.commit("fix: finish (#3)");
  repo.run("tag", "v9.0.0");

  const stable = composeNotes("0.2.0", REPOSITORY, repo.root);
  assert.ok(stable.includes("Changes since v0.1.0:") && stable.includes("- preview (#2)") && stable.includes("- finish (#3)"), stable);
  const prerelease = composeNotes("0.2.0-rc.2", REPOSITORY, repo.root);
  assert.ok(prerelease.includes("Changes since v0.2.0-rc.1:") && !prerelease.includes("- preview (#2)"), prerelease);
});

test("takes the first release's notes only from its handwritten note", (t) => {
  const repo = repository(t);
  repo.commit("Initial commit");
  assert.throws(() => composeNotes("0.1.0", REPOSITORY, repo.root), /docs\/release-notes\/0\.1\.0\.md is missing/);
  repo.note("0.1.0", "## First release\n\nHello.\n");
  const notes = composeNotes("0.1.0", REPOSITORY, repo.root);
  assert.ok(notes.startsWith("## First release\n\nHello.\n\n## Install") && !notes.includes("## Changelog"), notes);
});

test("rejects a note without a level-two heading or a body", (t) => {
  const repo = repository(t);
  repo.commit("Initial commit");
  repo.note("0.1.0", "# Title\n\nBody.\n");
  assert.throws(() => composeNotes("0.1.0", REPOSITORY, repo.root), /must start with a '## <title>' heading/);
  repo.note("0.1.0", "## Title\n\n");
  assert.throws(() => composeNotes("0.1.0", REPOSITORY, repo.root), /has a heading but no body/);
});

test("refuses a release tag that points to another commit", (t) => {
  const repo = repository(t);
  repo.commit("Initial commit");
  repo.run("tag", "v0.2.0");
  const head = repo.commit("fix: later (#2)");
  assert.throws(() => releaseTarget("0.2.0", repo.root), /v0\.2\.0 already points to/);
  repo.run("tag", "--force", "v0.2.0");
  assert.deepEqual(releaseTarget("0.2.0", repo.root), { tag: "v0.2.0", sha: head });
});

test("counts a CI pass on an ancestor only when the later commits change files CI skips", (t) => {
  const repo = repository(t);
  const tested = repo.commit("feat: code");
  repo.commit("docs: guide", { path: "docs/guide.md" });
  const docs = repo.commit("docs: readme", { path: "workers/sync/README.md" });
  const code = repo.commit("fix: code", { path: "src/lib.rs" });
  assert.ok(ciCovers(tested, tested, repo.root));
  assert.ok(ciCovers(tested, docs, repo.root));
  assert.ok(!ciCovers(tested, code, repo.root));
  assert.ok(!ciCovers(code, tested, repo.root));
  assert.ok(!ciCovers("0123456789abcdef0123456789abcdef01234567", docs, repo.root));
});

test("collects the installer of the release version with its checksum", (t) => {
  const root = mkdtempSync(join(tmpdir(), "hatoba-dist-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const bundle = join(root, "nsis");
  mkdirSync(bundle);
  writeFileSync(join(bundle, "Hatoba_0.2.0-rc.1_x64-setup.exe"), "old");
  assert.throws(() => collectDist("0.2.0", bundle, join(root, "dist")), /Hatoba_0\.2\.0_x64-setup\.exe in .*found Hatoba_0\.2\.0-rc\.1_x64-setup\.exe/);

  writeFileSync(join(bundle, "Hatoba_0.2.0_x64-setup.exe"), "installer");
  assert.deepEqual(collectDist("0.2.0", bundle, join(root, "dist")), ["Hatoba_0.2.0_x64-setup.exe", CHECKSUMS]);
  const hash = createHash("sha256").update("installer").digest("hex");
  assert.equal(readFileSync(join(root, "dist", CHECKSUMS), "utf8"), `${hash}  Hatoba_0.2.0_x64-setup.exe\n`);
});
