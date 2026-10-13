import { INSTALL_STEPS, type Platform } from "../lib/release";
import type { Locale } from "./index";

type Names = Record<string, string>;

/**
 * The install steps of INSTALL_STEPS in scripts/release/release.mjs, which the release notes give in
 * English, translated. `source` is the English they translate: install.test.ts fails when the
 * English in release.mjs no longer matches it, so that a change there is translated here too.
 */
export const TRANSLATED_STEPS: { source: Record<Platform, (names: Names) => string> } & Record<"zh" | "ja", Record<Platform, (names: Names) => string>> = {
  source: {
    windows: ({ nsis }) =>
      `download \`${nsis}\` and run it. It installs Hatoba for the current user and needs no administrator rights. The installer is not code-signed, so Windows SmartScreen may stop it: choose **More info**, then **Run anyway**.`,
    linux: ({ deb, appimage }) =>
      `on Debian, Ubuntu, and distributions based on them, download \`${deb}\` and install it with \`sudo apt install ./${deb}\`. On other distributions, download \`${appimage}\`, make it executable with \`chmod +x ${appimage}\`, and run it.`,
    macos: ({ dmg }) =>
      `on macOS 13 or later, download \`${dmg}\`, open it, and drag Hatoba to Applications. The app is not signed with an Apple Developer ID or notarized, so macOS blocks it the first time you open it: close the warning, go to **System Settings → Privacy & Security**, choose **Open Anyway** under **Security**, and enter your login password. After an update, macOS may ask whether Hatoba can use its keychain items: enter your login password and choose **Always Allow**, because sync fails if you deny it.`,
  },
  zh: {
    windows: ({ nsis }) =>
      `下载 \`${nsis}\` 并运行。它为当前用户安装 Hatoba，不需要管理员权限。安装程序没有代码签名，Windows SmartScreen 可能会拦截它：选择**更多信息**，再选择**仍要运行**。`,
    linux: ({ deb, appimage }) =>
      `在 Debian、Ubuntu 及其衍生发行版上，下载 \`${deb}\`，然后用 \`sudo apt install ./${deb}\` 安装。在其他发行版上，下载 \`${appimage}\`，用 \`chmod +x ${appimage}\` 赋予执行权限后运行。`,
    macos: ({ dmg }) =>
      `在 macOS 13 或更高版本上，下载 \`${dmg}\`，打开后把 Hatoba 拖到“应用程序”文件夹。这个应用没有 Apple Developer ID 签名，也没有经过公证，所以第一次打开时 macOS 会阻止它：关闭警告，前往**系统设置 → 隐私与安全性**，在**安全性**下选择**仍要打开**，然后输入登录密码。更新之后，macOS 可能会询问 Hatoba 能否使用它的钥匙串项目：输入登录密码并选择**始终允许**，拒绝的话同步会失败。`,
  },
  ja: {
    windows: ({ nsis }) =>
      `\`${nsis}\` をダウンロードして実行します。現在のユーザー向けにインストールされ、管理者権限は必要ありません。インストーラーはコード署名されていないため、Windows SmartScreen に止められることがあります。その場合は**詳細情報**を選び、**実行**を選びます。`,
    linux: ({ deb, appimage }) =>
      `Debian、Ubuntu とその派生ディストリビューションでは、\`${deb}\` をダウンロードし、\`sudo apt install ./${deb}\` でインストールします。その他のディストリビューションでは、\`${appimage}\` をダウンロードし、\`chmod +x ${appimage}\` で実行権限を付けてから実行します。`,
    macos: ({ dmg }) =>
      `macOS 13 以降で \`${dmg}\` をダウンロードして開き、Hatoba を「アプリケーション」フォルダにドラッグします。このアプリは Apple Developer ID で署名されておらず、公証も受けていないため、初めて開くときに macOS にブロックされます。警告を閉じて**システム設定 → プライバシーとセキュリティ**を開き、**セキュリティ**の下の**このまま開く**を選んで、ログインパスワードを入力します。アップデート後、Hatoba がキーチェーン項目を使用してよいか macOS に尋ねられることがあります。同期に失敗しないよう、ログインパスワードを入力して**常に許可**を選んでください。`,
  },
};

/** The install steps for `platform` in `locale`, as inline Markdown. */
export function installSteps(locale: Locale, platform: Platform, names: Names): string {
  if (locale !== "en") return TRANSLATED_STEPS[locale][platform](names);
  const text = INSTALL_STEPS[platform].steps(names);
  return text.charAt(0).toUpperCase() + text.slice(1);
}
