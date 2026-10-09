# Terminal

Each SSH session is a tab with a full terminal (256 colors and truecolor, CJK wide characters and input methods, mouse support for programs such as vim and tmux). Inactive tabs keep running. There are no split panes, no session logging and no snippets.

## The status bar

The bar above the terminal shows the connection state, the target `user@host:port`, "via" and the jump host's name when one is used, and the latency in ms once connected. The buttons on the right:

| en | zh-CN | ja |
|---|---|---|
| Connecting… | 正在连接… | 接続中… |
| Connected | 已连接 | 接続済み |
| Connection failed | 连接失败 | 接続に失敗 |
| Disconnected | 已断开 | 切断済み |
| via | 经由 | 経由: |
| Find in terminal | 在终端中查找 | ターミナル内を検索 |
| Port Forwarding | 端口转发 | ポートフォワーディング |
| SFTP | SFTP | SFTP |
| Show or hide the SFTP file panel | 显示或隐藏 SFTP 文件面板 | SFTP ファイルパネルを表示/非表示 |
| More actions | 更多操作 | その他の操作 |

- **Find in terminal** opens the find bar.
- **Port Forwarding** (a badge shows how many forwards run) opens the forwards popover; see `references/port-forwarding.md`. It is enabled only when connected.
- **SFTP** shows or hides the file panel beside the terminal; see `references/sftp.md`. It is enabled only when connected.
- **More actions** (the … button) opens the tab menu below.

## Menus

The tab menu (**More actions**) and the right-click menu share most entries. Right-click shows the menu only when **Right-Click in Terminal** is set to **Show context menu** (see Settings below); by default right-click copies the selection, or pastes when nothing is selected. A program that tracks the mouse (vim, tmux) gets the right click itself unless Shift is held.

| en | zh-CN | ja |
|---|---|---|
| Reconnect | 重新连接 | 再接続 |
| Disconnect | 断开连接 | 切断 |
| Copy | 复制 | コピー |
| Paste | 粘贴 | 貼り付け |
| Select All | 全选 | すべて選択 |
| Find… | 查找… | 検索… |
| Ask AI | 询问 AI | AI に質問 |
| Clear Scrollback | 清除回滚缓冲 | スクロールバックを消去 |
| Edit Host | 编辑主机 | ホストを編集 |
| Save as Host… | 保存为主机… | ホストとして保存… |

- Right-click menu: Copy, Paste, Select All, Ask AI, Clear Scrollback, Find…
- **More actions** menu: Reconnect, Disconnect, Copy, Paste, Clear Scrollback, Select All, Find…, Ask AI, Edit Host. A quick connection (see `references/hosts-and-connecting.md`) has **Save as Host…** instead of Edit Host.
- **Ask AI** is enabled when text is selected. It opens the AI panel with the selection attached; see `references/ai-attachments.md`.

## Copy, paste, search and links

Plain Ctrl+letter always reaches the remote shell (Ctrl+L, Ctrl+W, Ctrl+K and so on). Hatoba's own shortcuts add Shift. See `references/shortcuts.md` for the full list.

| Action | Windows / Linux | macOS |
|---|---|---|
| Copy the selection | Ctrl+C when text is selected (otherwise Ctrl+C is ^C for the remote), Ctrl+Shift+C, or Ctrl+Insert | ⌘C |
| Paste | Ctrl+V, Ctrl+Shift+V, or Shift+Insert | ⌘V |
| Find in the terminal | Ctrl+Shift+F | ⌘F |

- Pasting text with line breaks first shows **Paste multiple lines?** with a preview, and **Paste** confirms. The check can be turned off in Settings → General (**Confirm before pasting multiple lines**). Pasting needs a connected tab.
- The find bar searches the visible screen and the scrollback. Enter goes to the next match, Shift+Enter to the previous one, Esc closes the bar. It shows "No results" or the position as i/n.
- Links in the output open in the default browser with Ctrl+click (⌘+click on macOS). A plain click only selects text.

| en | zh-CN | ja |
|---|---|---|
| Paste multiple lines? | 粘贴多行内容？ | 複数行を貼り付けますか？ |
| Find | 查找 | 検索 |
| No results | 无结果 | 一致なし |
| Previous (Shift+Enter) | 上一个（Shift+Enter） | 前へ（Shift+Enter） |
| Next (Enter) | 下一个（Enter） | 次へ（Enter） |
| Close find (Esc) | 关闭查找（Esc） | 検索を閉じる（Esc） |
| Confirm before pasting multiple lines | 粘贴多行文本前确认 | 複数行を貼り付ける前に確認 |
| General | 通用 | 一般 |
| Right-Click in Terminal | 终端中的右键 | ターミナルでの右クリック |
| Copy if selected, otherwise paste | 有选中则复制，否则粘贴 | 選択があればコピー、なければ貼り付け |
| Show context menu | 弹出菜单 | コンテキストメニューを表示 |

## Terminal settings

The terminal settings sync with the vault. They are in three places:

- Settings → Terminal: font, size, cursor and scrollback, with a live preview.
- Settings → Appearance: **Terminal Colors**.
- Settings → General: **Right-Click in Terminal** and **Confirm before pasting multiple lines** (labels above).

| en | zh-CN | ja |
|---|---|---|
| Terminal | 终端 | ターミナル |
| Font | 字体 | フォント |
| Font Size | 字号 | フォントサイズ |
| Decrease font size | 减小字号 | フォントサイズを小さく |
| Increase font size | 增大字号 | フォントサイズを大きく |
| Cursor Style | 光标样式 | カーソルの形 |
| Block | 方块 | ブロック |
| Bar | 竖线 | バー |
| Underline | 下划线 | アンダーライン |
| Scrollback | 回滚缓冲区 | スクロールバック |
| Appearance | 外观 | 外観 |
| Terminal Colors | 终端配色 | ターミナルの配色 |
| Always Dark | 始终深色 | 常にダーク |
| Match Appearance | 跟随外观 | 外観に合わせる |

- **Font**: the name of an installed font family (default Cascadia Mono). A font that is not installed falls back to the system monospace font.
- **Font Size**: 10 to 24 (default 13), changed with the minus and plus buttons.
- **Cursor Style**: Block (default), Bar or Underline.
- **Scrollback**: 1,000, 5,000, 10,000 (default), or 50,000 lines.
- **Terminal Colors**: **Always Dark** (default) keeps the terminal dark in both themes; **Match Appearance** follows the light or dark theme.
- Language, theme and list density are device-local settings in Settings → Appearance; see `references/settings.md`.

## Tab states

A tab can be connecting (orange dot, "Connecting to …" overlay), connected (green), failed (red, with a card to retry or edit the host), or disconnected (grey, with a banner to reconnect). See `references/hosts-and-connecting.md`. While a conversation of the AI assistant runs or waits for approval, the tab shows a small sparkle or hand icon.
