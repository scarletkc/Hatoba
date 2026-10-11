import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import {
  CHECKSUMS,
  LATEST,
  TAURI_CONFIG,
  allowsOnlyMain,
  assembleRelease,
  ciCovers,
  collectDist,
  composeNotes,
  git,
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
  assert.ok(notes.startsWith("## Version display\n\nThe settings show the version.\n\n## Install\n\n**Windows:** download `Hatoba_0.2.0_windows-x64-setup.exe`"), notes);
  assert.ok(notes.includes("`sudo apt install ./Hatoba_0.2.0_linux-x64.deb`") && notes.includes("`chmod +x Hatoba_0.2.0_linux-x64.AppImage`"), notes);
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

/** The names that `tauri build` gives the Windows and Linux installers, by bundle type. */
const BUILT = {
  nsis: (version) => `Hatoba_${version}_x64-setup.exe`,
  deb: (version) => `Hatoba_${version}_amd64.deb`,
  appimage: (version) => `Hatoba_${version}_amd64.AppImage`,
};

/**
 * Tauri's bundle directory as `tauri build` leaves it on each platform, with every installer
 * signed. The fixture signature covers only INSTALLER, so each installer is a copy of it.
 */
function bundled(t, version, { installer = INSTALLER, signature = SIGNATURE } = {}) {
  const root = mkdtempSync(join(tmpdir(), "hatoba-dist-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const bundle = join(root, "bundle");
  for (const [type, built] of Object.entries(BUILT)) {
    mkdirSync(join(bundle, type), { recursive: true });
    writeFileSync(join(bundle, type, built(version)), installer);
    if (signature !== null) writeFileSync(join(bundle, type, `${built(version)}.sig`), signature);
  }
  return { bundle, output: join(root, "dist") };
}

const dist = (version, platform, { bundle, output }, pubkey = PUBKEY) => collectDist(version, platform, bundle, output, { pubkey });

const DMG = "Hatoba 0.2.0 disk image stand-in\n";

/**
 * Adds the macOS bundles to a `bundled` directory, as `tauri build --bundles app,dmg` leaves them:
 * the disk image under its versioned name, and the app tarball, which installed apps update from,
 * under an unversioned one.
 */
function withMacos(release, version, { tarball = INSTALLER, signature = SIGNATURE } = {}) {
  mkdirSync(join(release.bundle, "dmg"), { recursive: true });
  writeFileSync(join(release.bundle, "dmg", `Hatoba_${version}_aarch64.dmg`), DMG);
  mkdirSync(join(release.bundle, "macos"), { recursive: true });
  writeFileSync(join(release.bundle, "macos", "Hatoba.app.tar.gz"), tarball);
  if (signature !== null) writeFileSync(join(release.bundle, "macos", "Hatoba.app.tar.gz.sig"), signature);
  return release;
}

const MACOS = ["Hatoba_0.2.0_macos-apple-silicon.dmg", "Hatoba_0.2.0_macos-apple-silicon-update.app.tar.gz", "Hatoba_0.2.0_macos-apple-silicon-update.app.tar.gz.sig"];

/** Release notes as the notes command writes them. */
function releaseNotes(t, version) {
  const repo = repository(t);
  repo.commit("Initial commit");
  repo.run("tag", "v0.1.0");
  repo.commit("feat(sync): send only what changed (#7)");
  repo.note(version, "## Faster sync\n\nSync sends only what changed.\n");
  return composeNotes(version, REPOSITORY, repo.root);
}

const assemble = (version, output, notes) =>
  assembleRelease(version, output, { notes, repository: REPOSITORY, now: new Date("2026-10-09T08:30:15.123Z") });

test("collects each platform's installers with their updater signatures under their names in the release", (t) => {
  const release = bundled(t, "0.2.0");
  assert.deepEqual(dist("0.2.0", "windows", release), ["Hatoba_0.2.0_windows-x64-setup.exe", "Hatoba_0.2.0_windows-x64-setup.exe.sig"]);
  assert.deepEqual(readdirSync(release.output).sort(), ["Hatoba_0.2.0_windows-x64-setup.exe", "Hatoba_0.2.0_windows-x64-setup.exe.sig"]);
  const linux = ["Hatoba_0.2.0_linux-x64.deb", "Hatoba_0.2.0_linux-x64.deb.sig", "Hatoba_0.2.0_linux-x64.AppImage", "Hatoba_0.2.0_linux-x64.AppImage.sig"];
  assert.deepEqual(dist("0.2.0", "linux", release), linux);
  assert.equal(readFileSync(join(release.output, "Hatoba_0.2.0_linux-x64.AppImage"), "utf8"), INSTALLER);
  assert.equal(readFileSync(join(release.output, "Hatoba_0.2.0_linux-x64.AppImage.sig"), "utf8"), SIGNATURE);
  assert.throws(() => dist("0.2.0", "freebsd", release), /Unknown platform 'freebsd'\. Use windows, linux, or macos\./);
});

test("writes the checksums and latest.json of every installer", (t) => {
  const release = withMacos(bundled(t, "0.2.0"), "0.2.0");
  dist("0.2.0", "windows", release);
  dist("0.2.0", "linux", release);
  dist("0.2.0", "macos", release);
  const notes = releaseNotes(t, "0.2.0");
  const installers = ["Hatoba_0.2.0_windows-x64-setup.exe", "Hatoba_0.2.0_linux-x64.deb", "Hatoba_0.2.0_linux-x64.AppImage"];
  assert.deepEqual(assemble("0.2.0", release.output, notes), [...installers.flatMap((name) => [name, `${name}.sig`]), ...MACOS, CHECKSUMS, LATEST]);
  const hash = createHash("sha256").update(INSTALLER).digest("hex");
  const macos = `${createHash("sha256").update(DMG).digest("hex")}  Hatoba_0.2.0_macos-apple-silicon.dmg\n${hash}  Hatoba_0.2.0_macos-apple-silicon-update.app.tar.gz\n`;
  assert.equal(readFileSync(join(release.output, CHECKSUMS), "utf8"), installers.map((name) => `${hash}  ${name}\n`).join("") + macos);
  const latest = JSON.parse(readFileSync(join(release.output, LATEST), "utf8"));
  const download = (name) => ({ signature: SIGNATURE, url: `https://github.com/${REPOSITORY}/releases/download/v0.2.0/${name}` });
  assert.deepEqual(latest, {
    version: "0.2.0",
    notes: updateNotes(notes, "0.2.0", REPOSITORY),
    pub_date: "2026-10-09T08:30:15Z",
    platforms: {
      "windows-x86_64": download("Hatoba_0.2.0_windows-x64-setup.exe"),
      "linux-x86_64-deb": download("Hatoba_0.2.0_linux-x64.deb"),
      "linux-x86_64-appimage": download("Hatoba_0.2.0_linux-x64.AppImage"),
      // The updater installs the app tarball; the disk image is only for people to download.
      "darwin-aarch64": download("Hatoba_0.2.0_macos-apple-silicon-update.app.tar.gz"),
    },
  });
  assert.ok(latest.notes.startsWith("## Faster sync\n\nSync sends only what changed.\n\n## Changelog\n\n"), latest.notes);
  assert.ok(latest.notes.includes(`- **sync:** send only what changed ([#7](https://github.com/${REPOSITORY}/pull/7))\n`), latest.notes);
  assert.ok(!latest.notes.includes("## Install") && !latest.notes.includes("SmartScreen"), latest.notes);
  assert.ok(!latest.notes.includes("Open Anyway"), latest.notes);
});

test("refuses to assemble a release without every platform's files", (t) => {
  const release = withMacos(bundled(t, "0.2.0"), "0.2.0");
  dist("0.2.0", "windows", release);
  dist("0.2.0", "macos", release);
  assert.throws(
    () => assemble("0.2.0", release.output, releaseNotes(t, "0.2.0")),
    /Missing release files in .*: Hatoba_0\.2\.0_linux-x64\.deb, Hatoba_0\.2\.0_linux-x64\.deb\.sig, Hatoba_0\.2\.0_linux-x64\.AppImage, Hatoba_0\.2\.0_linux-x64\.AppImage\.sig\. Collect them with the dist command of each platform\./,
  );
  assert.ok(!existsSync(join(release.output, LATEST)) && !existsSync(join(release.output, CHECKSUMS)));
});

test("leaves only the install section out of the updater's notes", (t) => {
  const repo = repository(t);
  repo.commit("Initial commit");
  repo.note("0.1.0", "## First release\n\nHello.\n");
  assert.equal(updateNotes(composeNotes("0.1.0", REPOSITORY, repo.root), "0.1.0", REPOSITORY), "## First release\n\nHello.");
  repo.run("tag", "v0.1.0");
  repo.commit("fix: keep the active tab (#8)");
  const changelogOnly = updateNotes(composeNotes("0.1.1", REPOSITORY, repo.root), "0.1.1", REPOSITORY);
  assert.ok(changelogOnly.startsWith("## Changelog\n\nChanges since v0.1.0:") && changelogOnly.endsWith(")"), changelogOnly);
  assert.throws(() => updateNotes(composeNotes("0.1.1", REPOSITORY, repo.root), "0.1.2", REPOSITORY), /no install section for 0\.1\.2/);
});

test("links pull request numbers at the end of a line in the updater's notes", (t) => {
  const repo = repository(t);
  repo.commit("Initial commit");
  repo.run("tag", "v0.1.0");
  const unlinked = repo.commit("fix: keep the active tab");
  repo.commit("fix: restore the window size (#8)");
  repo.note("0.1.1", "## Window fixes\n\nThe window keeps its size, which #8 fixed (#8)\n");
  const notes = updateNotes(composeNotes("0.1.1", REPOSITORY, repo.root), "0.1.1", REPOSITORY);
  const link = `([#8](https://github.com/${REPOSITORY}/pull/8))`;
  assert.ok(notes.startsWith(`## Window fixes\n\nThe window keeps its size, which #8 fixed ${link}\n\n`), notes);
  assert.ok(notes.includes(`- restore the window size ${link}\n`), notes);
  assert.ok(notes.includes(`- keep the active tab (${unlinked.slice(0, 7)}`), notes);
});

test("accepts a signing environment only when main alone can use it", () => {
  const custom = { deployment_branch_policy: { protected_branches: false, custom_branch_policies: true } };
  const main = { name: "main", type: "branch" };
  assert.ok(allowsOnlyMain(custom, [main]));
  assert.ok(!allowsOnlyMain(null, []));
  assert.ok(!allowsOnlyMain({ deployment_branch_policy: null }, []));
  assert.ok(!allowsOnlyMain({ deployment_branch_policy: { protected_branches: true, custom_branch_policies: false } }, []));
  assert.ok(!allowsOnlyMain(custom, []));
  assert.ok(!allowsOnlyMain(custom, [{ name: "*", type: "branch" }]));
  assert.ok(!allowsOnlyMain(custom, [{ name: "release/*", type: "branch" }]));
  assert.ok(!allowsOnlyMain(custom, [{ name: "main", type: "tag" }]));
  assert.ok(!allowsOnlyMain(custom, [main, { name: "release/*", type: "branch" }]));
});

test("refuses a bundle without the installer or its updater signature", (t) => {
  const release = bundled(t, "0.2.0", { signature: null });
  writeFileSync(join(release.bundle, "nsis", "Hatoba_0.2.0-rc.1_x64-setup.exe"), "old");
  assert.throws(() => dist("0.2.1", "windows", release), /Hatoba_0\.2\.1_x64-setup\.exe in .*found Hatoba_0\.2\.0-rc\.1_x64-setup\.exe, Hatoba_0\.2\.0_x64-setup\.exe/);
  assert.throws(() => dist("0.2.0", "windows", release), /Expected the updater signature Hatoba_0\.2\.0_x64-setup\.exe\.sig .*TAURI_SIGNING_PRIVATE_KEY/);
  assert.ok(!existsSync(release.output));

  const linux = bundled(t, "0.2.0");
  rmSync(join(linux.bundle, "appimage", "Hatoba_0.2.0_amd64.AppImage.sig"));
  assert.throws(() => dist("0.2.0", "linux", linux), /Expected the updater signature Hatoba_0\.2\.0_amd64\.AppImage\.sig/);
  assert.ok(!existsSync(linux.output), "nothing is copied while a file is missing");
});

test("refuses a signature that installed apps would refuse", (t) => {
  assert.throws(() => dist("0.2.0", "windows", bundled(t, "0.2.0"), OTHER_PUBKEY), /signed with another key .*release-signing/);
  assert.throws(() => dist("0.2.0", "linux", bundled(t, "0.2.0", { installer: "tampered" })), /does not match the installer/);
  assert.throws(() => dist("0.2.0", "windows", bundled(t, "0.2.0", { signature: "bm90IGEgc2lnbmF0dXJl" })), /not a minisign signature/);
  // The fixture was signed for 0.2.0.
  assert.throws(() => dist("0.3.0", "linux", bundled(t, "0.3.0")), /Hatoba_0\.3\.0_amd64\.deb\.sig was made for version 0\.2\.0, not 0\.3\.0/);
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

test("collects the macOS disk image, and the app tarball with its signature, under their names in the release", (t) => {
  const release = withMacos(bundled(t, "0.2.0"), "0.2.0");
  assert.deepEqual(dist("0.2.0", "macos", release), MACOS);
  assert.deepEqual(readdirSync(release.output).sort(), [...MACOS].sort());
  assert.equal(readFileSync(join(release.output, "Hatoba_0.2.0_macos-apple-silicon.dmg"), "utf8"), DMG);
  assert.equal(readFileSync(join(release.output, "Hatoba_0.2.0_macos-apple-silicon-update.app.tar.gz"), "utf8"), INSTALLER);
  assert.equal(readFileSync(join(release.output, "Hatoba_0.2.0_macos-apple-silicon-update.app.tar.gz.sig"), "utf8"), SIGNATURE);
  assert.equal(signatureName("app", "0.2.0"), "Hatoba_0.2.0_macos-apple-silicon-update.app.tar.gz.sig");
  assert.throws(() => signatureName("dmg", "0.2.0"), /Hatoba_0\.2\.0_macos-apple-silicon\.dmg has no updater signature/);
});

test("refuses a macOS bundle without the disk image or the app tarball, or with a tarball that installed apps would refuse", (t) => {
  const noArchive = withMacos(bundled(t, "0.2.0"), "0.2.0");
  rmSync(join(noArchive.bundle, "macos", "Hatoba.app.tar.gz"));
  assert.throws(
    () => dist("0.2.0", "macos", noArchive),
    /Expected the updater archive Hatoba\.app\.tar\.gz in .*createUpdaterArtifacts.*TAURI_SIGNING_PRIVATE_KEY/,
  );
  assert.ok(!existsSync(noArchive.output), "nothing is copied while a file is missing");

  const unsigned = withMacos(bundled(t, "0.2.0"), "0.2.0", { signature: null });
  assert.throws(() => dist("0.2.0", "macos", unsigned), /Expected the updater signature Hatoba\.app\.tar\.gz\.sig .*TAURI_SIGNING_PRIVATE_KEY/);

  const noImage = withMacos(bundled(t, "0.2.0"), "0.2.0");
  rmSync(join(noImage.bundle, "dmg", "Hatoba_0.2.0_aarch64.dmg"));
  assert.throws(() => dist("0.2.0", "macos", noImage), /Expected the installer Hatoba_0\.2\.0_aarch64\.dmg in .*found no installer/);

  assert.throws(() => dist("0.2.0", "macos", withMacos(bundled(t, "0.2.0"), "0.2.0", { tarball: "tampered" })), /does not match the installer/);
  // The tarball's name has no version, so only its signature tells a tarball left from an earlier build.
  assert.throws(() => dist("0.3.0", "macos", withMacos(bundled(t, "0.3.0"), "0.3.0")), /Hatoba\.app\.tar\.gz\.sig was made for version 0\.2\.0, not 0\.3\.0/);
});

test("refuses to assemble a release without every macOS file", (t) => {
  const notes = releaseNotes(t, "0.2.0");
  const release = withMacos(bundled(t, "0.2.0"), "0.2.0");
  dist("0.2.0", "windows", release);
  dist("0.2.0", "linux", release);
  assert.throws(
    () => assemble("0.2.0", release.output, notes),
    /Missing release files in .*: Hatoba_0\.2\.0_macos-apple-silicon\.dmg, Hatoba_0\.2\.0_macos-apple-silicon-update\.app\.tar\.gz, Hatoba_0\.2\.0_macos-apple-silicon-update\.app\.tar\.gz\.sig\. Collect them/,
  );
  dist("0.2.0", "macos", release);
  rmSync(join(release.output, "Hatoba_0.2.0_macos-apple-silicon.dmg"));
  assert.throws(() => assemble("0.2.0", release.output, notes), /Missing release files in .*: Hatoba_0\.2\.0_macos-apple-silicon\.dmg\. Collect them/);
  assert.ok(!existsSync(join(release.output, LATEST)) && !existsSync(join(release.output, CHECKSUMS)));
});

test("writes install instructions for each platform, with macOS marked experimental", (t) => {
  const repo = repository(t);
  repo.commit("Initial commit");
  repo.note("0.2.0", "## First release\n\nHello.\n");
  const notes = composeNotes("0.2.0", REPOSITORY, repo.root);
  const install = [
    "## Install",
    "**Windows:** download `Hatoba_0.2.0_windows-x64-setup.exe` and run it. It installs Hatoba for the current user and needs no administrator rights. The installer is not code-signed, so Windows SmartScreen may stop it: choose **More info**, then **Run anyway**.",
    "**Linux (x86_64):** on Debian, Ubuntu, and distributions based on them, download `Hatoba_0.2.0_linux-x64.deb` and install it with `sudo apt install ./Hatoba_0.2.0_linux-x64.deb`. On other distributions, download `Hatoba_0.2.0_linux-x64.AppImage`, make it executable with `chmod +x Hatoba_0.2.0_linux-x64.AppImage`, and run it.",
    "**macOS (Apple Silicon, experimental):** on macOS 13 or later, download `Hatoba_0.2.0_macos-apple-silicon.dmg`, open it, and drag Hatoba to Applications. The app is not signed with an Apple Developer ID or notarized, so macOS blocks it the first time you open it: close the warning, go to **System Settings → Privacy & Security**, choose **Open Anyway** under **Security**, and enter your login password. After an update, macOS may ask whether Hatoba can use its keychain items: enter your login password and choose **Always Allow**, because sync fails if you deny it.",
    "`SHA256SUMS.txt` has the SHA-256 checksum of each file. The `.sig` files, `Hatoba_0.2.0_macos-apple-silicon-update.app.tar.gz`, and `latest.json` are for the app's updater, so you don't need to download them.",
  ];
  assert.equal(notes, `## First release\n\nHello.\n\n${install.join("\n\n")}\n`);
  assert.equal(updateNotes(notes, "0.2.0", REPOSITORY), "## First release\n\nHello.");
});
