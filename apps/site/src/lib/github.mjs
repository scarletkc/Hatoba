// The releases the site offers, read from GitHub. Plain JavaScript, so that scripts/demo.mjs can
// build the demo from the release that release.ts describes.

export const REPO = "scarletkc/Hatoba";

/**
 * The fields of a GitHub release that the site reads.
 * @typedef {object} GitHubRelease
 * @property {string} tag_name
 * @property {string} html_url
 * @property {boolean} draft
 * @property {boolean} prerelease
 * @property {string | null} published_at
 * @property {{ name: string; size: number; browser_download_url: string }[]} assets
 */

/**
 * A GET of `path` under the repository in GitHub's API, without the unauthenticated rate limit when
 * GITHUB_TOKEN or GH_TOKEN is set.
 * @param {string} path
 * @returns {Promise<Response>}
 */
export async function github(path) {
  const token = process.env.GITHUB_TOKEN ?? process.env.GH_TOKEN;
  const response = await fetch(`https://api.github.com/repos/${REPO}${path}`, {
    headers: { accept: "application/vnd.github+json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
  });
  if (!response.ok) throw new Error(`GitHub answered ${response.status} for ${path}: ${await response.text()}`);
  return response;
}

/**
 * The release the site offers: the newest stable release, or the newest prerelease while there is no
 * stable one. A prerelease published after it is the preview. GitHub lists releases newest first.
 * @param {GitHubRelease[]} releases
 * @returns {{ current: GitHubRelease; preview: GitHubRelease | null }}
 */
export function selectRelease(releases) {
  const published = releases.filter((release) => !release.draft && release.published_at);
  const current = published.find((release) => !release.prerelease) ?? published[0];
  if (!current) throw new Error(`${REPO} has no published release.`);
  const newer = published.slice(0, published.indexOf(current));
  return { current, preview: newer.find((release) => release.prerelease) ?? null };
}

/**
 * The newest releases, and the one the site offers among them.
 * @returns {Promise<{ current: GitHubRelease; preview: GitHubRelease | null }>}
 */
export async function offeredRelease() {
  return selectRelease(/** @type {GitHubRelease[]} */ (await (await github("/releases?per_page=30")).json()));
}
