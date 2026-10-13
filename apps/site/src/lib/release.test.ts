import { describe, expect, it } from "vitest";
import { downloadsOf, parseChecksums, selectRelease, type GitHubRelease } from "./release";

const release = (tag: string, prerelease: boolean, extra: Partial<GitHubRelease> = {}): GitHubRelease => ({
  tag_name: tag,
  html_url: `https://github.com/scarletkc/Hatoba/releases/tag/${tag}`,
  draft: false,
  prerelease,
  published_at: "2026-10-12T18:37:05Z",
  assets: [],
  ...extra,
});

describe("selectRelease", () => {
  it("offers the newest stable release, and a newer prerelease as the preview", () => {
    const { current, preview } = selectRelease([release("v0.2.0-beta.1", true), release("v0.1.0", false), release("v0.1.0-alpha.2", true)]);
    expect(current.tag_name).toBe("v0.1.0");
    expect(preview?.tag_name).toBe("v0.2.0-beta.1");
  });

  it("offers the newest prerelease while there is no stable release", () => {
    const { current, preview } = selectRelease([release("v0.1.0-alpha.2", true), release("v0.1.0-alpha.1", true)]);
    expect(current.tag_name).toBe("v0.1.0-alpha.2");
    expect(preview).toBeNull();
  });

  it("skips drafts and unpublished releases", () => {
    const { current } = selectRelease([release("v0.3.0", false, { draft: true }), release("v0.2.0", false, { published_at: null }), release("v0.1.0", false)]);
    expect(current.tag_name).toBe("v0.1.0");
  });

  it("refuses a repository without a release", () => {
    expect(() => selectRelease([])).toThrow(/no published release/);
  });
});

describe("parseChecksums", () => {
  it("reads text and binary mode lines", () => {
    const a = "a".repeat(64);
    const b = "B".repeat(64);
    const sums = parseChecksums(`${a}  Hatoba_0.1.0_linux-x64.deb\r\n${b} *Hatoba_0.1.0_windows-x64-setup.exe\n\n`);
    expect(sums.get("Hatoba_0.1.0_linux-x64.deb")).toBe(a);
    expect(sums.get("Hatoba_0.1.0_windows-x64-setup.exe")).toBe("b".repeat(64));
  });
});

describe("downloadsOf", () => {
  const names = [
    "Hatoba_0.1.0_windows-x64-setup.exe",
    "Hatoba_0.1.0_linux-x64.deb",
    "Hatoba_0.1.0_linux-x64.AppImage",
    "Hatoba_0.1.0_macos-apple-silicon.dmg",
    "Hatoba_0.1.0_macos-apple-silicon-update.app.tar.gz",
  ];
  const assets = [...names, "Hatoba_0.1.0_linux-x64.deb.sig", "latest.json", "SHA256SUMS.txt"].map((name, i) => ({
    name,
    size: 1000 + i,
    browser_download_url: `https://example.com/${name}`,
  }));
  const sums = new Map(names.map((name, i) => [name, String(i).repeat(64)]));

  it("offers each platform's installers people download, with their checksums", () => {
    const downloads = downloadsOf(release("v0.1.0", false, { assets }), sums);
    expect(downloads.map(({ platform, bundle, name }) => [platform, bundle, name])).toEqual([
      ["windows", "nsis", "Hatoba_0.1.0_windows-x64-setup.exe"],
      ["linux", "deb", "Hatoba_0.1.0_linux-x64.deb"],
      ["linux", "appimage", "Hatoba_0.1.0_linux-x64.AppImage"],
      ["macos", "dmg", "Hatoba_0.1.0_macos-apple-silicon.dmg"],
    ]);
    expect(downloads[3]).toMatchObject({ url: "https://example.com/Hatoba_0.1.0_macos-apple-silicon.dmg", sha256: "3".repeat(64), size: 1003 });
  });

  it("leaves out an installer the release does not have, and says so", () => {
    const withoutDeb = assets.filter((asset) => !asset.name.endsWith(".deb"));
    const warnings: string[] = [];
    const downloads = downloadsOf(release("v0.1.0", false, { assets: withoutDeb }), sums, (message) => warnings.push(message));
    expect(downloads.map((download) => download.bundle)).toEqual(["nsis", "appimage", "dmg"]);
    expect(warnings).toEqual([expect.stringMatching(/no Hatoba_0\.1\.0_linux-x64\.deb/)]);
  });

  it("refuses a release without any installer for a platform", () => {
    const withoutDmg = assets.filter((asset) => !asset.name.endsWith(".dmg"));
    expect(() => downloadsOf(release("v0.1.0", false, { assets: withoutDmg }), sums, () => {})).toThrow(/none of the macos installers/);
  });

  it("refuses an installer without a checksum", () => {
    const partial = new Map([...sums].filter(([name]) => !name.endsWith(".deb")));
    expect(() => downloadsOf(release("v0.1.0", false, { assets }), partial)).toThrow(/no checksum for Hatoba_0\.1\.0_linux-x64\.deb/);
  });
});
