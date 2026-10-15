# Settings

Open Settings with the gear in the sidebar header or with Ctrl+, (⌘, on macOS). It is a window over the app with seven tabs. Changes apply at once; there is no Save button. Esc or the close button closes it. Terminal settings are in `references/terminal.md` and AI settings in `references/ai-settings.md`.

| en | zh-CN | ja |
|---|---|---|
| Settings | 设置 | 設定 |
| General | 通用 | 一般 |
| Appearance | 外观 | 外観 |
| Terminal | 终端 | ターミナル |
| Proxies | 代理 | プロキシ |
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
| Close Button | 关闭窗口时 | ウィンドウを閉じるとき |
| Ask every time | 每次询问 | 毎回確認する |
| Minimize to tray | 最小化到托盘 | トレイに最小化 |
| Quit Hatoba | 退出 Hatoba | Hatoba を終了 |
| Notify when the AI assistant finishes or needs approval | AI 助手完成或需要确认时通知 | AI アシスタントの完了や確認待ちを通知 |
| Refresh interval | 刷新间隔 | 更新間隔 |
| Export Encrypted Backup | 导出加密备份 | 暗号化バックアップをエクスポート |
| Export… | 导出… | エクスポート… |

- **Right-Click in Terminal**: **Copy if selected, otherwise paste** (default, except on macOS) or **Show context menu** (default on macOS).
- **Confirm before pasting multiple lines**: on by default. Pasting text that contains line breaks shows a preview first.
- **Show host reachability**: on by default. Every 60 seconds Hatoba tries a TCP connection to the hosts in the list (no login) and colors the dot. A host with a proxy is tried through it, and counts as reachable once its SSH server answers.
- **Close Button** (Windows and Linux only): what the window's close button, Alt+F4 and the taskbar's Close do. **Ask every time** (default) asks whether to minimize Hatoba to the system tray or quit, and the question's **Remember my choice** sets this row. **Minimize to tray** hides the window while SSH sessions, port forwards and sync keep running; the tray icon brings it back. **Quit Hatoba** quits. Without a tray, on a Linux desktop with no StatusNotifier host such as GNOME without an extension, closing always quits. macOS has no such row: its red button hides the window, the Dock icon brings it back, and ⌘Q quits.
- **Notify when the AI assistant finishes or needs approval**: on by default. While Hatoba is not the active window, also while it is minimized or in the tray, a conversation that finishes a turn, stops on an error, or waits for approval of a tool call shows a system notification, also one in a tab's background. Its title is the tab's name (**AI Assistant** for the home tab), and its text says what happened and names the conversation. Clicking it brings the window back and shows the conversation with the AI panel open. The system's own notification settings for Hatoba, and Do Not Disturb, can still hide them.
- **Refresh interval** (3 s by default) and the three thresholds below it (CPU, memory and disk use, 90% by default, or **Off**) belong to the dashboard; see `references/dashboard.md`.
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

## Proxies

SOCKS5 and HTTP proxies that hosts connect through. Choosing one for a host is in `references/hosts-and-connecting.md`.

- **Default proxy on this device**: **No proxy** or a saved proxy. Hosts whose proxy is **Device default**, and quick connections, use it. The choice stays on this device and does not sync, so a proxy that runs on this computer (such as Clash on 127.0.0.1) can be the default here and not on the others.
- **Saved Proxies** lists each proxy with its type, address and the number of hosts that name it; the device's default has a **Default** badge. Click a row to edit it; the trash icon deletes it after showing which hosts switch to the device default. Proxies sync with their usernames and passwords, encrypted like the rest of the vault.
- **Add Proxy** asks for **Name**, **Type** (SOCKS5 or HTTP; an HTTP proxy must support CONNECT), **Address** and **Port**, and a **Username** and **Password** when the proxy needs a sign-in. Pasting a proxy URL such as `socks5://127.0.0.1:7890` or `http://user:pass@proxy.example.com:3128` into **Address** fills in the other fields. A saved password shows **Saved** and can be replaced but not viewed. SOCKS5 and HTTP send the proxy's username and password to the proxy in the clear.

| en | zh-CN | ja |
|---|---|---|
| Default proxy on this device | 这台设备的默认代理 | このデバイスの既定のプロキシ |
| No proxy | 不使用代理 | プロキシなし |
| Saved Proxies | 已保存的代理 | 保存したプロキシ |
| Add Proxy | 添加代理 | プロキシを追加 |
| Edit Proxy | 编辑代理 | プロキシを編集 |
| Type | 类型 | 種類 |
| Default | 默认 | 既定 |

## Security

| en | zh-CN | ja |
|---|---|---|
| Auto-Lock | 自动锁定 | 自動ロック |
| Lock when the computer sleeps | 电脑睡眠时锁定 | コンピュータのスリープ時にロック |
| Disconnect all sessions when locked | 锁定时断开所有会话 | ロック時にすべてのセッションを切断 |
| Clear Clipboard | 清除剪贴板 | クリップボードの消去 |
| Unlock with Windows Hello | 使用 Windows Hello 解锁 | Windows Hello でロック解除 |
| Unlock with PIN | 使用 PIN 解锁 | PIN でロック解除 |
| Require the master password after a restart | 重启后需要输入主密码 | 再起動後はマスターパスワードを要求 |
| Master Password | 主密码 | マスターパスワード |
| Change… | 更改主密码… | 変更… |
| Recovery Code | 恢复码 | リカバリーコード |
| Generate New Code… | 生成新的恢复码… | 新しいコードを生成… |
| Lock Now | 立即锁定 | 今すぐロック |
| Lock | 锁定 | ロック |

Auto-Lock (default 15 minutes, up to 8 hours or Never), **Lock when the computer sleeps** (on by default), **Disconnect all sessions when locked** (off by default), **Clear Clipboard** for copied host passwords (default 90 seconds), **Unlock with Windows Hello** (Windows only), **Unlock with PIN** with **Require the master password after a restart** (on by default), the master password, the recovery code, and **Lock Now**. Details are in `references/keys-vault-and-lock.md`.

## About

| en | zh-CN | ja |
|---|---|---|
| Software Update | 软件更新 | ソフトウェアアップデート |
| Check for Updates | 检查更新 | アップデートを確認 |
| Download and Install | 下载并安装 | ダウンロードしてインストール |
| Install and Restart | 安装并重启 | インストールして再起動 |
| View on GitHub | 在 GitHub 上查看 | GitHub で見る |
| Automatically check for updates | 自动检查更新 | アップデートを自動で確認 |
| Checking… | 正在检查… | 確認しています… |
| Hatoba is up to date. | 已是最新版本。 | 最新バージョンです。 |
| Copy version | 复制版本号 | バージョンをコピー |
| Website | 官网 | Web サイト |
| Source Code | 源代码 | ソースコード |
| MIT License | MIT 许可证 | MIT ライセンス |
| Report a Problem | 反馈问题 | 問題を報告 |
| Settings (update available) | 设置（有新版本） | 設定（アップデートあり） |
| Enjoying Hatoba? | 喜欢 Hatoba 吗？ | Hatoba は気に入りましたか？ |
| Star | 点 Star | スターを付ける |

- The page shows the version; click it to copy.
- **Software Update** → **Check for Updates** asks GitHub whether a newer version exists. The result is "Checking…", "Hatoba is up to date.", "Hatoba vX is available.", or an offline or failure message. A newer version's release notes show below the setting, with **View on GitHub** for its release page.
- **Download and Install** asks first only when SSH sessions are open or file transfers are running, and says how many closing Hatoba will end. After **Install and Restart** (or right away when nothing would end), Hatoba downloads the installer and checks its signature, then closes, which disconnects every session, and opens again once the installer finishes. An installer without a valid signature from Hatoba is refused and not run. Update error messages are in `references/troubleshooting.md`.
- **Automatically check for updates** is on by default. When on, Hatoba asks GitHub after the first unlock of each launch and then once a day while it keeps running, including hidden in the tray; when off, it contacts GitHub only when **Check for Updates** or **Download and Install** is pressed.
- When a newer release is found, the sidebar gear shows a dot, its tooltip becomes **Settings (update available)**, and it opens the About tab. On Windows and Linux the tray menu also shows **Update to vX.Y.Z**, which opens the About tab and starts **Download and Install**.
- **Website** opens https://hatoba.fog.moe, with downloads and these docs. **Source Code** and **MIT License** open the GitHub repository and the license. **Report a Problem** opens GitHub's bug report form with the Hatoba version and the operating system filled in; the user reviews and submits it.
- A card in the sidebar (**Enjoying Hatoba?**) asks once for a GitHub star, a day after first use and after a session has connected. **Star** opens the repository, **Report a Problem** opens the bug form, and the × closes it for good.

## Which settings sync

| Synced with the vault | Kept on this device |
|---|---|
| Terminal settings (font, size, cursor, scrollback, terminal colors, right-click, multi-line paste check), the default AI model, the judge of Auto mode, the search provider, the built-in skill's switch, the AI custom instructions and Terminal Screen default, saved proxies | Language, appearance, list density, host reachability, the default proxy, the local terminal shell, the Security tab's Auto-Lock, sleep, disconnect and clipboard settings, the PIN, the update check switch, the AI permission mode, the saved AI approvals, the AI panel width and whether it is open, MCP on/off and Always allow |
