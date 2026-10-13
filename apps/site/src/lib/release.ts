import { CHECKSUMS, INSTALL_STEPS as STEPS, installerNames } from "../../../../scripts/release/release.mjs";
import { REPO } from "../consts";

export type Platform = keyof typeof STEPS;
/** INSTALL_STEPS of scripts/release/release.mjs, typed: each `steps` takes the installer names by bundle type. */
export const INSTALL_STEPS = STEPS as unknown as Record<Platform, { heading: string; bundles: string[]; steps: (names: Record<string, string>) => string }>;
export const PLATFORMS = Object.keys(INSTALL_STEPS) as Platform[];

/** The fields of a GitHub release that the site reads. */
export interface GitHubRelease {
  tag_name: string;
  html_url: string;
  draft: boolean;
  prerelease: boolean;
  published_at: string | null;
  assets: { name: string; size: number; browser_download_url: string }[];
}

export interface Download {
  bundle: string;
  platform: Platform;
  name: string;
  url: string;
  size: number;
  sha256: string;
}

export interface SiteRelease {
  version: string;
  tag: string;
  url: string;
  publishedAt: string;
  downloads: Download[];
  checksumsUrl: string;
  /** Installer names by bundle type, for the install steps. */
  names: Record<string, string>;
  /** A prerelease published after `version`, offered as a preview. */
  preview: { version: string; url: string } | null;
}

const versionOf = (tag: string) => tag.replace(/^v/, "");

/**
 * The release the site offers: the newest stable release, or the newest prerelease while there is no
 * stable one. A prerelease published after it is the preview. GitHub lists releases newest first.
 */
export function selectRelease(releases: GitHubRelease[]): { current: GitHubRelease; preview: GitHubRelease | null } {
  const published = releases.filter((release) => !release.draft && release.published_at);
  const current = published.find((release) => !release.prerelease) ?? published[0];
  if (!current) throw new Error(`${REPO} has no published release.`);
  const newer = published.slice(0, published.indexOf(current));
  return { current, preview: newer.find((release) => release.prerelease) ?? null };
}

/** Reads SHA256SUMS.txt: one `<hash>  <file name>` line per file. */
export function parseChecksums(text: string): Map<string, string> {
  const sums = new Map<string, string>();
  for (const line of text.split(/\r?\n/)) {
    const match = /^([0-9a-f]{64}) [ *](.+)$/i.exec(line.trim());
    if (match) sums.set(match[2], match[1].toLowerCase());
  }
  return sums;
}

/**
 * The installers people download from `release`, with their checksums. The names come from
 * INSTALLERS on this commit, which may name an installer that the release predates, such as a new
 * package: `warn` hears of each one left out. Throws when a platform has no installer at all, or
 * when an installer has no checksum.
 */
export function downloadsOf(release: GitHubRelease, sums: Map<string, string>, warn: (message: string) => void = console.warn): Download[] {
  const names = installerNames(versionOf(release.tag_name));
  return PLATFORMS.flatMap((platform) => {
    const downloads = INSTALL_STEPS[platform].bundles.flatMap((bundle) => {
      const name = names[bundle];
      const asset = release.assets.find((candidate) => candidate.name === name);
      if (!asset) {
        warn(`${release.tag_name} has no ${name}, the ${bundle} installer that INSTALLERS in scripts/release/release.mjs names, so the download page leaves it out.`);
        return [];
      }
      const sha256 = sums.get(name);
      if (!sha256) throw new Error(`${CHECKSUMS} of ${release.tag_name} has no checksum for ${name}.`);
      return [{ bundle, platform, name, url: asset.browser_download_url, size: asset.size, sha256 }];
    });
    if (downloads.length === 0) throw new Error(`${release.tag_name} has none of the ${platform} installers that INSTALLERS in scripts/release/release.mjs names.`);
    return downloads;
  });
}

async function github(path: string): Promise<Response> {
  const token = process.env.GITHUB_TOKEN ?? process.env.GH_TOKEN;
  const response = await fetch(`https://api.github.com/repos/${REPO}${path}`, {
    headers: { accept: "application/vnd.github+json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
  });
  if (!response.ok) throw new Error(`GitHub answered ${response.status} for ${path}: ${await response.text()}`);
  return response;
}

let cached: Promise<SiteRelease> | undefined;

/** The release the site offers, read from GitHub once per build. */
export function loadRelease(): Promise<SiteRelease> {
  cached ??= (async () => {
    const releases = (await (await github("/releases?per_page=30")).json()) as GitHubRelease[];
    const { current, preview } = selectRelease(releases);
    const checksums = current.assets.find((asset) => asset.name === CHECKSUMS);
    if (!checksums) throw new Error(`${current.tag_name} has no ${CHECKSUMS}.`);
    const response = await fetch(checksums.browser_download_url);
    if (!response.ok) throw new Error(`Downloading ${CHECKSUMS} of ${current.tag_name} failed with ${response.status}.`);
    const version = versionOf(current.tag_name);
    return {
      version,
      tag: current.tag_name,
      url: current.html_url,
      publishedAt: current.published_at!,
      downloads: downloadsOf(current, parseChecksums(await response.text())),
      checksumsUrl: checksums.browser_download_url,
      names: installerNames(version),
      preview: preview && { version: versionOf(preview.tag_name), url: preview.html_url },
    };
  })();
  return cached;
}

