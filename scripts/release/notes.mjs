// Release notes, changelog grouping, and install instructions.
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { ROOT, git } from "../lib/git.mjs";
import { CHECKSUMS, LATEST, installerNames } from "./installers.mjs";
import { releaseTarget } from "./target.mjs";
import { NOTES_DIR, VERSION, compareVersions, isPrerelease } from "./version.mjs";

/** Returns the handwritten release note of `version`, or null when there is none. */
export function handwrittenNote(version, root = ROOT) {
  const path = `${NOTES_DIR}/${version}.md`;
  if (!existsSync(join(root, path))) return null;
  const text = readFileSync(join(root, path), "utf8").trim();
  const [heading, ...body] = text.split("\n");
  if (!/^## \S/.test(heading)) throw new Error(`${path} must start with a '## <title>' heading.`);
  if (!body.join("\n").trim())
    throw new Error(`${path} has a heading but no body. Write the release note below the heading.`);
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

/**
 * How to install Hatoba on each platform, which the release notes and the website's download page
 * give. `bundles` are the installers people download for the platform, and `steps` takes the
 * release's installer names by bundle type. The website translates the steps in
 * apps/site/src/i18n/install.ts, whose test fails when the English here changes.
 */
export const INSTALL_STEPS = {
  windows: {
    heading: "Windows",
    bundles: ["nsis"],
    steps: ({ nsis }) =>
      `download \`${nsis}\` and run it. It installs Hatoba for the current user and needs no administrator rights. The installer is not code-signed, so Windows SmartScreen may stop it: choose **More info**, then **Run anyway**.`,
  },
  linux: {
    heading: "Linux (x86_64)",
    bundles: ["deb", "appimage"],
    steps: ({ deb, appimage }) =>
      `on Debian, Ubuntu, and distributions based on them, download \`${deb}\` and install it with \`sudo apt install ./${deb}\`. On other distributions, download \`${appimage}\`, make it executable with \`chmod +x ${appimage}\`, and run it.`,
  },
  macos: {
    heading: "macOS (Apple Silicon, experimental)",
    bundles: ["dmg"],
    steps: ({ dmg }) =>
      `on macOS 13 or later, download \`${dmg}\`, open it, and drag Hatoba to Applications. The app is not signed with an Apple Developer ID or notarized, so macOS blocks it the first time you open it: close the warning, go to **System Settings → Privacy & Security**, choose **Open Anyway** under **Security**, and enter your login password. After an update, macOS may ask whether Hatoba can use its keychain items: enter your login password and choose **Always Allow**, because sync fails if you deny it.`,
  },
};

function installSection(version) {
  const names = installerNames(version);
  return [
    "## Install",
    ...Object.values(INSTALL_STEPS).map(({ heading, steps }) => `**${heading}:** ${steps(names)}`),
    `\`${CHECKSUMS}\` has the SHA-256 checksum of each file. The \`.sig\` files, \`${names.app}\`, and \`${LATEST}\` are for the app's updater, so you don't need to download them.`,
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

/**
 * The release notes for the updater: without the install instructions, which describe the manual
 * download, and with each trailing (#N) as a link, which GitHub adds only on the release page.
 */
export function updateNotes(notes, version, repository) {
  const install = installSection(version);
  const at = notes.indexOf(install);
  if (at === -1)
    throw new Error(`The release notes have no install section for ${version}. Write them with the notes command.`);
  return [notes.slice(0, at).trimEnd(), notes.slice(at + install.length).trim()]
    .filter(Boolean)
    .join("\n\n")
    .replace(/\(#(\d+)\)$/gm, `([#$1](https://github.com/${repository}/pull/$1))`);
}
