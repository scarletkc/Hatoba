// Checks, describes, packages, and publishes the release of the version in the version files.
// The Release workflow (.github/workflows/release.yml) runs each command, and docs/releasing.md
// describes the steps.
// Usage: node scripts/release/release.mjs <command> [options]
//   check                            Check the version files, the release tag, and the release note
//   gate                             Require a CI pass and an approval-gated release environment
//   notes --output FILE              Write the release notes
//   dist --bundle DIR --output DIR   Collect the installer and its checksum from the NSIS bundle
//   publish --notes FILE --dist DIR  Create the GitHub Release, or upload the files it is missing
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { copyFileSync, existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { basename, dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import { parseArgs } from "node:util";
import { NOTES_DIR, ROOT, VERSION, compareVersions, isPrerelease, readVersion } from "./version.mjs";

/** Keep in sync with the environment of the publish job in release.yml. */
export const ENVIRONMENT = "release";
export const CHECKSUMS = "SHA256SUMS.txt";

/** Tauri names the NSIS installer <productName>_<version>_<arch>-setup.exe. */
export function installerName(version) {
  return `Hatoba_${version}_x64-setup.exe`;
}

export function git(args, root = ROOT) {
  return execFileSync("git", args, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

/** Returns the release tag and HEAD, and refuses a tag that already points to another commit. */
export function releaseTarget(version, root = ROOT) {
  const tag = `v${version}`;
  const sha = git(["rev-parse", "HEAD"], root);
  if (git(["tag", "--list", tag], root)) {
    const tagged = git(["rev-parse", `refs/tags/${tag}^{commit}`], root);
    if (tagged !== sha) {
      throw new Error(`${tag} already points to ${tagged.slice(0, 7)}, not to ${sha.slice(0, 7)}. Bump to a new version instead of moving the tag.`);
    }
  }
  return { tag, sha };
}

/** Returns the handwritten release note of `version`, or null when there is none. */
export function handwrittenNote(version, root = ROOT) {
  const path = `${NOTES_DIR}/${version}.md`;
  if (!existsSync(join(root, path))) return null;
  const text = readFileSync(join(root, path), "utf8").trim();
  const [heading, ...body] = text.split("\n");
  if (!/^## \S/.test(heading)) throw new Error(`${path} must start with a '## <title>' heading.`);
  if (!body.join("\n").trim()) throw new Error(`${path} has a heading but no body. Write the release note below the heading.`);
  return text;
}

/**
 * Returns the tag of the previous release: the highest vVERSION tag that HEAD contains and that is
 * lower than `version`. A stable release skips prereleases, so its changelog covers the whole
 * prerelease cycle.
 */
export function previousRelease(version, root = ROOT) {
  const stable = !isPrerelease(version);
  const earlier = git(["tag", "--merged", "HEAD"], root)
    .split("\n")
    .filter((tag) => tag.startsWith("v") && VERSION.test(tag.slice(1)))
    .map((tag) => tag.slice(1))
    .filter((v) => compareVersions(v, version) < 0 && !(stable && isPrerelease(v)));
  const previous = earlier.sort(compareVersions).at(-1);
  return previous === undefined ? null : `v${previous}`;
}

export function commitsSince(tag, root = ROOT) {
  return git(["log", "--reverse", "--format=%h%x1f%s%x1f%b%x1e", `${tag}..HEAD`], root)
    .split("\x1e")
    .map((record) => record.trim())
    .filter(Boolean)
    .map((record) => {
      const [hash, subject, body = ""] = record.split("\x1f");
      return { hash, subject, body };
    });
}

const HEADER = /^(?<type>[A-Za-z]+)(?:\((?<scope>[^()]*)\))?(?<bang>!)?: (?<description>.+)$/;
const BREAKING_FOOTER = /^BREAKING[ -]CHANGE: /m;
const PULL_REQUEST = /\(#\d+\)$/;
const SECTIONS = [
  ["breaking", "Breaking changes"],
  ["feat", "Features"],
  ["fix", "Fixes"],
  ["perf", "Performance"],
];

/**
 * Sorts commits into SECTIONS by their Conventional Commits header. Other types, and commits
 * without such a header, go to `other`.
 */
export function groupCommits(commits) {
  const groups = Object.fromEntries([...SECTIONS.map(([key]) => [key, []]), ["other", []]]);
  for (const { hash, subject, body } of commits) {
    const header = HEADER.exec(subject)?.groups;
    let group = "other";
    if (header && (header.bang || BREAKING_FOOTER.test(body))) group = "breaking";
    else if (header && ["feat", "fix", "perf"].includes(header.type)) group = header.type;
    const text = group === "other" ? subject : `${header.scope ? `**${header.scope}:** ` : ""}${header.description}`;
    // Squash-merged pull requests end with (#N), which GitHub links; other commits get their hash.
    groups[group].push(`- ${text}${PULL_REQUEST.test(subject) ? "" : ` (${hash})`}`);
  }
  return groups;
}

function changelog(previous, tag, repository, root) {
  const groups = groupCommits(commitsSince(previous, root));
  const parts = ["## Changelog", `Changes since ${previous}:`];
  for (const [key, title] of SECTIONS) {
    if (groups[key].length > 0) parts.push(`### ${title}`, groups[key].join("\n"));
  }
  if (groups.other.length > 0) {
    parts.push("<details>\n<summary>Other changes</summary>", groups.other.join("\n"), "</details>");
  }
  parts.push(`[Full diff](https://github.com/${repository}/compare/${previous}...${tag})`);
  return parts.join("\n\n");
}

function installSection(version) {
  return [
    "## Install",
    `Download \`${installerName(version)}\` and run it. It installs Hatoba for the current user and needs no administrator rights. The installer is not code-signed, so Windows SmartScreen may stop it: choose **More info**, then **Run anyway**. \`${CHECKSUMS}\` has its SHA-256 checksum.`,
  ].join("\n\n");
}

/** Composes the release notes: the handwritten note, install instructions, and the changelog. */
export function composeNotes(version, repository, root = ROOT) {
  const { tag } = releaseTarget(version, root);
  const note = handwrittenNote(version, root);
  const previous = previousRelease(version, root);
  if (previous === null && note === null) {
    throw new Error(
      `${NOTES_DIR}/${version}.md is missing. With no earlier release to list changes from, the release notes come only from that file; create it with a '## <title>' heading and a body.`,
    );
  }
  const parts = [...(note ? [note] : []), installSection(version)];
  if (previous !== null) parts.push(changelog(previous, tag, repository, root));
  return `${parts.join("\n\n")}\n`;
}

// Keep in sync with paths-ignore under push in ci.yml: CI does not run on commits that change only
// these files.
export const skippedByCi = (path) => path.endsWith(".md") || path.startsWith("docs/");

/** Whether a CI pass on `tested` covers `sha`: the same commit, or an ancestor that differs only in files CI skips. */
export function ciCovers(tested, sha, root = ROOT) {
  if (tested === sha) return true;
  try {
    git(["merge-base", "--is-ancestor", tested, sha], root);
  } catch {
    return false;
  }
  return git(["diff", "--name-only", "--no-renames", tested, sha], root)
    .split("\n")
    .filter(Boolean)
    .every(skippedByCi);
}

async function github(path, { allowMissing = false } = {}) {
  const token = process.env.GH_TOKEN;
  if (!token) throw new Error("Set GH_TOKEN to a GitHub token that can read this repository.");
  const response = await fetch(`https://api.github.com/repos/${path}`, {
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": "2022-11-28",
      "User-Agent": "hatoba-release",
    },
  });
  if (response.status === 404 && allowMissing) return null;
  if (!response.ok) throw new Error(`GitHub API request for ${path} failed with HTTP ${response.status}: ${await response.text()}`);
  return response.json();
}

async function gate(repository, root = ROOT) {
  const sha = git(["rev-parse", "HEAD"], root);
  const { workflow_runs: runs } = await github(`${repository}/actions/workflows/ci.yml/runs?status=success&per_page=100`);
  const passed = runs.find((run) => ciCovers(run.head_sha, sha, root));
  if (!passed) {
    throw new Error(
      `CI has not passed on ${sha.slice(0, 7)}, or on an earlier commit that differs from it only in files CI skips. Wait for CI on this commit to pass, or run CI on this branch from the Actions tab, then rerun this workflow.`,
    );
  }
  console.log(`CI passed on ${passed.head_sha.slice(0, 7)}: ${passed.html_url}`);
  const environment = await github(`${repository}/environments/${ENVIRONMENT}`, { allowMissing: true });
  if (!environment?.protection_rules?.some((rule) => rule.type === "required_reviewers")) {
    throw new Error(
      `The ${ENVIRONMENT} environment must require a reviewer, so that nothing is published until someone approves the run. Set it up as docs/releasing.md describes, then rerun this workflow.`,
    );
  }
  console.log(`Publishing waits for approval in the ${ENVIRONMENT} environment.`);
}

/** Copies the installer out of the NSIS bundle directory and writes its checksum next to it. */
export function collectDist(version, bundle, output) {
  const name = installerName(version);
  if (!existsSync(join(bundle, name))) {
    const found = existsSync(bundle) ? readdirSync(bundle).filter((file) => file.endsWith(".exe")) : [];
    throw new Error(`Expected the installer ${name} in ${bundle}; found ${found.length > 0 ? found.join(", ") : "no installer"}.`);
  }
  mkdirSync(output, { recursive: true });
  copyFileSync(join(bundle, name), join(output, name));
  const hash = createHash("sha256").update(readFileSync(join(output, name))).digest("hex");
  writeFileSync(join(output, CHECKSUMS), `${hash}  ${name}\n`);
  return [name, CHECKSUMS];
}

function run(command, args, root) {
  console.log(`+ ${[command, ...args].join(" ")}`);
  execFileSync(command, args, { cwd: root, stdio: "inherit" });
}

async function publish(version, repository, notes, dist, root = ROOT) {
  const { tag, sha } = releaseTarget(version, root);
  if (!existsSync(notes)) throw new Error(`${notes} does not exist; write it with the notes command.`);
  const assets = [installerName(version), CHECKSUMS].map((name) => join(dist, name));
  const absent = assets.filter((path) => !existsSync(path));
  if (absent.length > 0) throw new Error(`Missing release files: ${absent.join(", ")}. Collect them with the dist command.`);
  const prerelease = isPrerelease(version);
  // The list includes drafts, which have no tag yet and so are not found by tag.
  const existing = (await github(`${repository}/releases?per_page=100`)).find((release) => release.tag_name === tag);
  if (!existing) {
    // A prerelease never becomes the latest release, which update checks read.
    const flags = prerelease ? ["--prerelease", "--latest=false"] : [];
    run("gh", ["release", "create", tag, ...assets, "--repo", repository, "--target", sha, "--title", `Hatoba ${tag}`, "--notes-file", notes, ...flags], root);
    return;
  }
  if (existing.draft) {
    throw new Error(`A draft release already uses ${tag}. Review it on GitHub and delete or publish it, then rerun this workflow.`);
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

async function main() {
  const [command, ...args] = process.argv.slice(2);
  const { values } = parseArgs({
    args,
    options: { output: { type: "string" }, bundle: { type: "string" }, notes: { type: "string" }, dist: { type: "string" } },
  });
  const option = (name) => {
    if (!values[name]) throw new Error(`${command} needs --${name}.`);
    return values[name];
  };
  const repository = process.env.GITHUB_REPOSITORY ?? "scarletkc/Hatoba";
  if (!/^[\w.-]+\/[\w.-]+$/.test(repository)) throw new Error(`GITHUB_REPOSITORY must be OWNER/REPO; got '${repository}'.`);
  const version = readVersion(ROOT);
  switch (command) {
    case "check": {
      const { tag, sha } = releaseTarget(version);
      composeNotes(version, repository);
      const previous = previousRelease(version);
      const source = previous ? `changes since ${previous}` : `notes only from ${NOTES_DIR}/${version}.md`;
      console.log(`Ready to release Hatoba ${tag} from ${sha.slice(0, 7)}, with ${source}.`);
      break;
    }
    case "gate":
      await gate(repository);
      break;
    case "notes": {
      const output = option("output");
      mkdirSync(dirname(output), { recursive: true });
      writeFileSync(output, composeNotes(version, repository));
      console.log(`Wrote ${output}`);
      break;
    }
    case "dist": {
      const output = option("output");
      console.log(`Collected ${collectDist(version, option("bundle"), output).join(" and ")} in ${output}`);
      break;
    }
    case "publish":
      await publish(version, repository, option("notes"), option("dist"));
      break;
    default:
      throw new Error(`Unknown command '${command ?? ""}'. Use check, gate, notes, dist, or publish.`);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
