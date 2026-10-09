# Settings

Open Settings with the gear in the sidebar header or with Ctrl+, (⌘, on macOS). It is a window over the app with six tabs. Changes apply at once; there is no Save button. Esc or the close button closes it. Terminal settings are in `references/terminal.md` and AI settings in `references/ai-settings.md`.

| en | zh-CN | ja |
|---|---|---|
| Settings | 设置 | 設定 |
| General | 通用 | 一般 |
| Appearance | 外观 | 外観 |
| Terminal | 终端 | ターミナル |
| Security | 安全 | セキュリティ |
| AI | AI | AI |
| About | 关于 | 情報 |

## General

| en | zh-CN | ja |
|---|---|---|
| Right-Click in Terminal | 终端中的右键 | ターミナルでの右クリック |
| Copy if selected, otherwise paste | 有选中则复制，否则粘贴 | 選択があればコピー、なければ貼り付け |
| Show context menu | 弹出菜单 | コンテキストメニューを表示 |
| Confirm before pasting multiple lines | 粘贴多行文本前确认 | 複数行を貼り付ける前に確認 |
| Show host reachability | 显示主机在线状态 | ホストの接続状態を表示 |
| Export Encrypted Backup | 导出加密备份 | 暗号化バックアップをエクスポート |
| Export… | 导出… | エクスポート… |

- **Right-Click in Terminal**: **Copy if selected, otherwise paste** (default) or **Show context menu**.
- **Confirm before pasting multiple lines**: on by default. Pasting text that contains line breaks shows a preview first.
- **Show host reachability**: on by default. Every 60 seconds Hatoba tries a TCP connection to the hosts in the list (no login) and colors the dot.
- **Export Encrypted Backup** → **Export…**: saves the vault as one file that only the master password opens. There is no import in the app.

## Appearance

| en | zh-CN | ja |
|---|---|---|
| Light | 浅色 | ライト |
| Dark | 深色 | ダーク |
| Auto | 跟随系统 | システムに合わせる |
| Terminal Colors | 终端配色 | ターミナルの配色 |
| Always Dark | 始终深色 | 常にダーク |
| Match Appearance | 跟随外观 | 外観に合わせる |
| List Density | 列表密度 | リストの密度 |
| Regular | 标准 | 標準 |
| Compact | 紧凑 | コンパクト |
| Language | 语言 | 言語 |
| System Default | 跟随系统 | システムの設定 |
| 简体中文 | 简体中文 | 简体中文 |
| English | English | English |
| 日本語 | 日本語 | 日本語 |

- **Appearance**: **Light**, **Dark**, or **Auto** (follows the system and changes live).
- **Terminal Colors**: **Always Dark** (default) or **Match Appearance**.
- **List Density**: **Regular** or **Compact** rows in lists.
- **Language**: **System Default** (the system language if it is Chinese, Japanese or English, otherwise English), **简体中文**, **English** or **日本語**. It takes effect immediately. Host names, commands and terminal output are never translated.

## Security

| en | zh-CN | ja |
|---|---|---|
| Auto-Lock | 自动锁定 | 自動ロック |
| Disconnect all sessions when locked | 锁定时断开所有会话 | ロック時にすべてのセッションを切断 |
| Unlock with Windows Hello | 使用 Windows Hello 解锁 | Windows Hello でロック解除 |
| Master Password | 主密码 | マスターパスワード |
| Change… | 更改主密码… | 変更… |
| Recovery Code | 恢复码 | リカバリーコード |
| Generate New Code… | 生成新的恢复码… | 新しいコードを生成… |
| Lock Now | 立即锁定 | 今すぐロック |
| Lock | 锁定 | ロック |

Auto-Lock (default 15 minutes), **Disconnect all sessions when locked** (off by default), **Unlock with Windows Hello** (Windows only), the master password, the recovery code, and **Lock Now**. Details are in `references/keys-vault-and-lock.md`.

## About

| en | zh-CN | ja |
|---|---|---|
| Software Update | 软件更新 | ソフトウェアアップデート |
| Check for Updates | 检查更新 | アップデートを確認 |
| Download and Install | 下载并安装 | ダウンロードしてインストール |
| Install and Restart | 安装并重启 | インストールして再起動 |
| View on GitHub | 在 GitHub 上查看 | GitHub で見る |
| Automatically check for updates at startup | 启动时自动检查更新 | 起動時にアップデートを自動で確認 |
| Checking… | 正在检查… | 確認しています… |
| Hatoba is up to date. | 已是最新版本。 | 最新バージョンです。 |
| Copy version | 复制版本号 | バージョンをコピー |
| Source Code | 源代码 | ソースコード |
| MIT License | MIT 许可证 | MIT ライセンス |
| Report a Problem | 反馈问题 | 問題を報告 |
| Settings (update available) | 设置（有新版本） | 設定（アップデートあり） |
| Enjoying Hatoba? | 喜欢 Hatoba 吗？ | Hatoba は気に入りましたか？ |
| Star | 点 Star | スターを付ける |

- The page shows the version; click it to copy.
- **Software Update** → **Check for Updates** asks GitHub whether a newer version exists. The result is "Checking…", "Hatoba is up to date.", "Hatoba vX is available.", or an offline or failure message. A newer version's release notes show below the setting, with **View on GitHub** for its release page.
- **Download and Install** asks first, and says how many open SSH sessions and running file transfers closing Hatoba will end. After **Install and Restart**, Hatoba downloads the installer and checks its signature, then closes, which disconnects every session, and opens again once the installer finishes. An installer without a valid signature from Hatoba is refused and not run.
- **Automatically check for updates at startup** is off by default. When on, Hatoba asks GitHub once after each unlock; when off, it contacts GitHub only when **Check for Updates** or **Download and Install** is pressed.
- When a newer release is found, the sidebar gear shows a dot, its tooltip becomes **Settings (update available)**, and it opens the About tab.
- **Source Code** and **MIT License** open the GitHub repository and the license. **Report a Problem** opens GitHub's bug report form with the Hatoba version and the operating system filled in; the user reviews and submits it.
- A card in the sidebar (**Enjoying Hatoba?**) asks once for a GitHub star, a day after first use and after a session has connected. **Star** opens the repository, **Report a Problem** opens the bug form, and the × closes it for good.

## Which settings sync

| Synced with the vault | Kept on this device |
|---|---|
| Terminal settings (font, size, cursor, scrollback, terminal colors, right-click, multi-line paste check), Auto-Lock, Disconnect all sessions when locked, the default AI model, the search provider, the built-in skill's switch | Language, appearance, list density, host reachability, the update check switch, the AI permission mode, the tool call limit, the AI panel width and whether it is open, MCP on/off and Always allow |
