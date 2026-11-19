export const CHECKSUMS = "SHA256SUMS.txt";
/** The update manifest that installed apps read from the latest stable release (spec §11). */
export const LATEST = "latest.json";

/**
 * The installers of a release, by Tauri bundle type: the name passed to `tauri build --bundles` and,
 * unless `dir` names another, the directory under target/release/bundle that holds the installer.
 * Each platform's build job in release.yml builds the bundle types of its platform. `built` is the
 * file name that `tauri build` writes, and `name` the file name in the release, which names the
 * platform in words people know so that they can tell which file to download. `updater` is the key
 * of the installer in latest.json: the updater looks for `<os>-<arch>-<bundle type>`, then for
 * `<os>-<arch>`. Only installers with that key have an updater signature: on macOS people download
 * the disk image, while the updater installs the app bundle from an `archive`, which `tauri build`
 * writes only when it makes updater artifacts. The package managers also find the Windows installer
 * and the deb package by `name` (docs/releasing.md, Package managers).
 */
export const INSTALLERS = {
  nsis: {
    platform: "windows",
    updater: "windows-x86_64",
    built: (version) => `Hatoba_${version}_x64-setup.exe`,
    name: (version) => `Hatoba_${version}_windows-x64-setup.exe`,
  },
  deb: {
    platform: "linux",
    updater: "linux-x86_64-deb",
    built: (version) => `Hatoba_${version}_amd64.deb`,
    name: (version) => `Hatoba_${version}_linux-x64.deb`,
  },
  appimage: {
    platform: "linux",
    updater: "linux-x86_64-appimage",
    built: (version) => `Hatoba_${version}_amd64.AppImage`,
    name: (version) => `Hatoba_${version}_linux-x64.AppImage`,
  },
  dmg: {
    platform: "macos",
    built: (version) => `Hatoba_${version}_aarch64.dmg`,
    name: (version) => `Hatoba_${version}_macos-apple-silicon.dmg`,
  },
  app: {
    platform: "macos",
    updater: "darwin-aarch64",
    dir: "macos",
    archive: true,
    built: () => "Hatoba.app.tar.gz",
    name: (version) => `Hatoba_${version}_macos-apple-silicon-update.app.tar.gz`,
  },
};

export function installerName(bundle, version) {
  return INSTALLERS[bundle].name(version);
}

/** The name in the release of the updater signature that `tauri build` writes next to the installer. */
export function signatureName(bundle, version) {
  if (!INSTALLERS[bundle].updater) throw new Error(`${installerName(bundle, version)} has no updater signature.`);
  return `${installerName(bundle, version)}.sig`;
}

/** The files the dist command collects for `bundle`: the installer, and its updater signature if it has one. */
export function distFiles(bundle, version) {
  const name = installerName(bundle, version);
  return INSTALLERS[bundle].updater ? [name, signatureName(bundle, version)] : [name];
}

/** The release's installer names by bundle type. */
export function installerNames(version) {
  return Object.fromEntries(Object.keys(INSTALLERS).map((bundle) => [bundle, installerName(bundle, version)]));
}
