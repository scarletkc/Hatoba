import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import {
  CHECKSUMS,
  LATEST,
  TAURI_CONFIG,
  ciCovers,
  collectDist,
  composeNotes,
  git,
  installerName,
  releaseTarget,
  signatureName,
  updateNotes,
  updaterPubkey,
  verifyUpdaterSignature,
} from "./release.mjs";

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


// A throwaway minisign key pair, made with `tauri signer generate`; its private half was deleted
// after signing INSTALLER for version 0.2.0. The desktop app's update tests use the same fixture.
const PUBKEY =
  "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDZBOTg3OEJBMDI2RjkxODEKUldTQmtXOEN1bmlZYWxXbEhiTXYrT0VCVmR2U2RCUHpMWEltV3RkOEJxRlZScDBOTytPc2xkcGMK";
const SIGNATURE =
  "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTQmtXOEN1bmlZYXZDMTNSbGdqcU5sWjRFSUVZY0twQTI2NThPRmYxR2FuaGhRRDJOemVET0Y0TDR3ejlEQ0ZqTlpBLzFzcGRWSjEwT2s2WWVhMHI4Mi9oTnIySTNZTFFNPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzkxNTM2Nzc5CWZpbGU6aW5zdGFsbGVyLmV4ZQl2ZXJzaW9uOjAuMi4wCm1IdlhLdVlYMFFHMzIwSFdpU0VIVDMvUzZkNUtEdHg5QkRveitqbFFtWGtuUkl1VGM5QnZYejVtU25hUThsTXI5alFnU1ROdTg4RXJRbFgxZ0g0REJ3PT0K";
const INSTALLER = "Hatoba 0.2.0 installer stand-in\n";
/** The public half of another throwaway key. */
const OTHER_PUBKEY =
  "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEYzRjZFMzFCMjA5M0U4QTAKUldTZzZKTWdHK1AyOC9SWm5TRFcyQk01WjZGMmVhdGw5M2x4YUJYd3NJWkR2eHdUVGRyWkFCZkwK";

/** A bundle directory as `tauri build` leaves it, and release notes as the notes command writes them. */
function bundled(t, version, { installer = INSTALLER, signature = SIGNATURE } = {}) {
  const root = mkdtempSync(join(tmpdir(), "hatoba-dist-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const bundle = join(root, "nsis");
  mkdirSync(bundle);
  writeFileSync(join(bundle, installerName(version)), installer);
  if (signature !== null) writeFileSync(join(bundle, signatureName(version)), signature);
  const repo = repository(t);
  repo.commit("Initial commit");
  repo.run("tag", "v0.1.0");
  repo.commit("feat(sync): send only what changed (#7)");
  repo.note(version, "## Faster sync\n\nSync sends only what changed.\n");
  const notes = composeNotes(version, REPOSITORY, repo.root);
  return { bundle, output: join(root, "dist"), notes };
}

const dist = (version, { bundle, output, notes }, pubkey = PUBKEY) =>
  collectDist(version, bundle, output, { notes, repository: REPOSITORY, pubkey, now: new Date("2026-10-09T08:30:15.123Z") });

test("collects the installer, its updater signature and checksum, and writes latest.json", (t) => {
  const release = bundled(t, "0.2.0");
  assert.deepEqual(dist("0.2.0", release), ["Hatoba_0.2.0_x64-setup.exe", "Hatoba_0.2.0_x64-setup.exe.sig", CHECKSUMS, LATEST]);
  const hash = createHash("sha256").update(INSTALLER).digest("hex");
  assert.equal(readFileSync(join(release.output, CHECKSUMS), "utf8"), `${hash}  Hatoba_0.2.0_x64-setup.exe\n`);
  assert.equal(readFileSync(join(release.output, "Hatoba_0.2.0_x64-setup.exe.sig"), "utf8"), SIGNATURE);
  const latest = JSON.parse(readFileSync(join(release.output, LATEST), "utf8"));
  assert.deepEqual(latest, {
    version: "0.2.0",
    notes: updateNotes(release.notes, "0.2.0"),
    pub_date: "2026-10-09T08:30:15Z",
    platforms: {
      "windows-x86_64": {
        signature: SIGNATURE,
        url: `https://github.com/${REPOSITORY}/releases/download/v0.2.0/Hatoba_0.2.0_x64-setup.exe`,
      },
    },
  });
  assert.ok(latest.notes.startsWith("## Faster sync\n\nSync sends only what changed.\n\n## Changelog\n\n"), latest.notes);
  assert.ok(!latest.notes.includes("## Install") && !latest.notes.includes("SmartScreen"), latest.notes);
});

test("leaves only the install section out of the updater's notes", (t) => {
  const repo = repository(t);
  repo.commit("Initial commit");
  repo.note("0.1.0", "## First release\n\nHello.\n");
  assert.equal(updateNotes(composeNotes("0.1.0", REPOSITORY, repo.root), "0.1.0"), "## First release\n\nHello.");
  repo.run("tag", "v0.1.0");
  repo.commit("fix: keep the active tab (#8)");
  const changelogOnly = updateNotes(composeNotes("0.1.1", REPOSITORY, repo.root), "0.1.1");
  assert.ok(changelogOnly.startsWith("## Changelog\n\nChanges since v0.1.0:") && changelogOnly.endsWith(")"), changelogOnly);
  assert.throws(() => updateNotes(composeNotes("0.1.1", REPOSITORY, repo.root), "0.1.2"), /no install section for 0\.1\.2/);
});

test("refuses a bundle without the installer or its updater signature", (t) => {
  const release = bundled(t, "0.2.0", { signature: null });
  writeFileSync(join(release.bundle, "Hatoba_0.2.0-rc.1_x64-setup.exe"), "old");
  assert.throws(() => dist("0.2.1", release), /Hatoba_0\.2\.1_x64-setup\.exe in .*found Hatoba_0\.2\.0-rc\.1_x64-setup\.exe, Hatoba_0\.2\.0_x64-setup\.exe/);
  assert.throws(() => dist("0.2.0", release), /Expected the updater signature Hatoba_0\.2\.0_x64-setup\.exe\.sig .*TAURI_SIGNING_PRIVATE_KEY/);
  assert.ok(!existsSync(release.output));
});

test("refuses a signature that installed apps would refuse", (t) => {
  assert.throws(() => dist("0.2.0", bundled(t, "0.2.0"), OTHER_PUBKEY), /signed with another key .*release-signing/);
  assert.throws(() => dist("0.2.0", bundled(t, "0.2.0", { installer: "tampered" })), /does not match the installer/);
  assert.throws(() => dist("0.2.0", bundled(t, "0.2.0", { signature: "bm90IGEgc2lnbmF0dXJl" })), /not a minisign signature/);
  // The fixture was signed for 0.2.0.
  assert.throws(() => dist("0.3.0", bundled(t, "0.3.0")), /made for version 0\.2\.0, not 0\.3\.0/);
});

test("checks the signature the way the updater does", () => {
  assert.equal(verifyUpdaterSignature(Buffer.from(INSTALLER), SIGNATURE, PUBKEY), "0.2.0");
  assert.throws(() => verifyUpdaterSignature(Buffer.from(`${INSTALLER} `), SIGNATURE, PUBKEY), /does not match/);
});

test("reads the updater's public key from tauri.conf.json", (t) => {
  const root = mkdtempSync(join(tmpdir(), "hatoba-config-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const configure = (pubkey) => {
    mkdirSync(dirname(join(root, TAURI_CONFIG)), { recursive: true });
    writeFileSync(join(root, TAURI_CONFIG), JSON.stringify({ plugins: { updater: { pubkey } } }));
  };
  configure("");
  assert.throws(() => updaterPubkey(root), /plugins\.updater\.pubkey in .* is empty/);
  configure("bm90IGEga2V5");
  assert.throws(() => updaterPubkey(root), /is not a minisign public key/);
  configure(PUBKEY);
  assert.equal(updaterPubkey(root), PUBKEY);
});
