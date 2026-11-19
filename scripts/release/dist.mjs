import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, extname, join } from "node:path";
import { ROOT } from "../lib/git.mjs";
import { github } from "./gate.mjs";
import { CHECKSUMS, INSTALLERS, LATEST, distFiles, installerName, signatureName } from "./installers.mjs";
import { verifyUpdaterSignature } from "./minisign.mjs";
import { updateNotes } from "./notes.mjs";
import { releaseTarget } from "./target.mjs";
import { isPrerelease } from "./version.mjs";

/** The bundle types that the build job of `platform` builds. */
function bundlesOf(platform) {
  const bundles = Object.keys(INSTALLERS).filter((bundle) => INSTALLERS[bundle].platform === platform);
  if (bundles.length === 0) {
    const platforms = [...new Set(Object.values(INSTALLERS).map((installer) => installer.platform))];
    throw new Error(
      `Unknown platform '${platform}'. Use ${new Intl.ListFormat("en", { type: "disjunction" }).format(platforms)}.`,
    );
  }
  return bundles;
}

/**
 * Copies the installers of `platform` and their updater signatures out of `bundles`, Tauri's bundle
 * directory (target/release/bundle), into `output` under their names in the release, after checking
 * each signature as installed apps will. Copies nothing when an installer or a signature is missing
 * or refused.
 */
export function collectDist(version, platform, bundles, output, { pubkey }) {
  const files = bundlesOf(platform).flatMap((bundle) => {
    const { dir: subdir = bundle, archive, built, updater } = INSTALLERS[bundle];
    const dir = join(bundles, subdir);
    const name = installerName(bundle, version);
    const file = built(version);
    if (!existsSync(join(dir, file))) {
      if (archive) {
        throw new Error(
          `Expected the updater archive ${file} in ${dir}. \`pnpm tauri build\` writes it only when it makes updater artifacts: with \`bundle.createUpdaterArtifacts\` on in tauri.conf.json and not turned off by --config, and with TAURI_SIGNING_PRIVATE_KEY and TAURI_SIGNING_PRIVATE_KEY_PASSWORD set, as docs/releasing.md describes.`,
        );
      }
      const found = existsSync(dir) ? readdirSync(dir).filter((entry) => entry.endsWith(extname(file))) : [];
      throw new Error(
        `Expected the installer ${file} in ${dir}; found ${found.length > 0 ? found.join(", ") : "no installer"}.`,
      );
    }
    if (!updater) return [{ from: join(dir, file), name }];
    const sig = `${file}.sig`;
    if (!existsSync(join(dir, sig))) {
      throw new Error(
        `Expected the updater signature ${sig} in ${dir}. \`pnpm tauri build\` writes it when TAURI_SIGNING_PRIVATE_KEY and TAURI_SIGNING_PRIVATE_KEY_PASSWORD are set, as docs/releasing.md describes.`,
      );
    }
    const signed = verifyUpdaterSignature(
      readFileSync(join(dir, file)),
      readFileSync(join(dir, sig), "utf8").trim(),
      pubkey,
    );
    if (signed !== version)
      throw new Error(`${sig} was made for version ${signed ?? "(none)"}, not ${version}. Rebuild the installer.`);
    return [
      { from: join(dir, file), name },
      { from: join(dir, sig), name: signatureName(bundle, version) },
    ];
  });
  mkdirSync(output, { recursive: true });
  for (const { from, name } of files) copyFileSync(from, join(output, name));
  return files.map(({ name }) => name);
}

/**
 * Writes the checksums of every installer and the updater's latest.json into `dist`, which holds
 * what the dist command collected on each platform, and returns the files of the release in the
 * order it lists them. `notes` is the text the notes command wrote.
 */
export function assembleRelease(version, dist, { notes, repository, now = new Date() }) {
  const bundles = Object.keys(INSTALLERS);
  const files = bundles.flatMap((bundle) => distFiles(bundle, version));
  const absent = files.filter((name) => !existsSync(join(dist, name)));
  if (absent.length > 0) {
    throw new Error(
      `Missing release files in ${dist}: ${absent.join(", ")}. Collect them with the dist command of each platform.`,
    );
  }
  const checksums = bundles.map((bundle) => {
    const name = installerName(bundle, version);
    return `${createHash("sha256")
      .update(readFileSync(join(dist, name)))
      .digest("hex")}  ${name}\n`;
  });
  writeFileSync(join(dist, CHECKSUMS), checksums.join(""));
  const platforms = bundles
    .filter((bundle) => INSTALLERS[bundle].updater)
    .map((bundle) => [
      INSTALLERS[bundle].updater,
      {
        signature: readFileSync(join(dist, signatureName(bundle, version)), "utf8").trim(),
        url: `https://github.com/${repository}/releases/download/v${version}/${installerName(bundle, version)}`,
      },
    ]);
  const latest = {
    version,
    notes: updateNotes(notes, version, repository),
    pub_date: now.toISOString().replace(/\.\d{3}Z$/, "Z"),
    platforms: Object.fromEntries(platforms),
  };
  writeFileSync(join(dist, LATEST), `${JSON.stringify(latest, null, 2)}\n`);
  return [...files, CHECKSUMS, LATEST];
}

function run(command, args, root) {
  console.log(`+ ${[command, ...args].join(" ")}`);
  execFileSync(command, args, { cwd: root, stdio: "inherit" });
}

export async function publish(version, repository, notes, dist, root = ROOT) {
  const { tag, sha } = releaseTarget(version, root);
  if (!existsSync(notes)) throw new Error(`${notes} does not exist; write it with the notes command.`);
  const assets = assembleRelease(version, dist, { notes: readFileSync(notes, "utf8"), repository }).map((name) =>
    join(dist, name),
  );
  const prerelease = isPrerelease(version);
  // The list includes drafts, which have no tag yet and so are not found by tag.
  const existing = (await github(`${repository}/releases?per_page=100`)).find((release) => release.tag_name === tag);
  if (!existing) {
    // A prerelease never becomes the latest release, whose latest.json installed stable versions read.
    const flags = prerelease ? ["--prerelease", "--latest=false"] : [];
    run(
      "gh",
      [
        "release",
        "create",
        tag,
        ...assets,
        "--repo",
        repository,
        "--target",
        sha,
        "--title",
        `Hatoba ${tag}`,
        "--notes-file",
        notes,
        ...flags,
      ],
      root,
    );
    return;
  }
  if (existing.draft) {
    throw new Error(
      `A draft release already uses ${tag}. Review it on GitHub and delete or publish it, then rerun this workflow.`,
    );
  }
  if (existing.prerelease !== prerelease) {
    throw new Error(
      `The ${tag} release is ${existing.prerelease ? "" : "not "}marked as a pre-release, but ${version} is ${prerelease ? "" : "not "}a prerelease. Correct the release on GitHub, then rerun this workflow.`,
    );
  }
  const uploaded = new Set(existing.assets.map((asset) => asset.name));
  const missing = assets.filter((path) => !uploaded.has(basename(path)));
  if (missing.length === 0) {
    console.log(`${tag} already has every file: ${existing.html_url}`);
    return;
  }
  console.log(`${tag} already exists; uploading only the files it is missing.`);
  run("gh", ["release", "upload", tag, ...missing, "--repo", repository], root);
}
