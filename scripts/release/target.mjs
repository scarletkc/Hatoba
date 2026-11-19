// Git release targeting: locating the release commit, tag, and raised version outputs.
import { ROOT, git } from "../lib/git.mjs";
import { NOTES_DIR, VERSION_FILES, compareVersions, isPrerelease } from "./version.mjs";

/** Returns the release tag and HEAD, and refuses a tag that already points to another commit. */
export function releaseTarget(version, root = ROOT) {
  const tag = `v${version}`;
  const sha = git(["rev-parse", "HEAD"], root);
  if (git(["tag", "--list", tag], root)) {
    const tagged = git(["rev-parse", `refs/tags/${tag}^{commit}`], root);
    if (tagged !== sha) {
      throw new Error(
        `${tag} already points to ${tagged.slice(0, 7)}, not to ${sha.slice(0, 7)}. Bump to a new version instead of moving the tag.`,
      );
    }
  }
  return { tag, sha };
}

/**
 * The commit to release: where the release pull request set the version that HEAD records, the oldest
 * commit since then on HEAD's first-parent history to change Cargo.toml, or a later one that changed
 * the version's release note, such as a fix to it. Commits merged after it go into the next release.
 */
export function releaseCommit(version, root = ROOT) {
  const manifest = VERSION_FILES.find((file) => file.path === "Cargo.toml");
  let found = null;
  for (const sha of git(["log", "--first-parent", "--format=%H", "HEAD", "--", manifest.path], root)
    .split("\n")
    .filter(Boolean)) {
    if (manifest.read(git(["show", `${sha}:${manifest.path}`], root))[0] !== version) break;
    found = sha;
  }
  if (found === null) throw new Error(`No commit on HEAD's history sets version ${version} in ${manifest.path}.`);
  const noted = git(["log", "--first-parent", "-1", "--format=%H", "HEAD", "--", `${NOTES_DIR}/${version}.md`], root);
  return noted && git(["rev-list", "--count", `${found}..${noted}`], root) !== "0" ? noted : found;
}

/**
 * `version`, the one that HEAD records, when it is greater than the version that the commit `from`
 * recorded and has no release tag yet, as when a release pull request is merged; otherwise null.
 */
export function raisedVersion(version, from, root = ROOT) {
  const manifest = VERSION_FILES.find((file) => file.path === "Cargo.toml");
  const previous = manifest.read(git(["show", `${from}:${manifest.path}`], root))[0];
  if (compareVersions(version, previous) <= 0) return null;
  return git(["tag", "--list", `v${version}`], root) ? null : version;
}

/**
 * The release's version, tag, and whether it is a prerelease, as `name=value` lines for
 * $GITHUB_OUTPUT. The package manager jobs in release.yml run only for a stable release.
 */
export function releaseOutputs(version) {
  return `version=${version}\ntag=v${version}\nprerelease=${isPrerelease(version)}\n`;
}
