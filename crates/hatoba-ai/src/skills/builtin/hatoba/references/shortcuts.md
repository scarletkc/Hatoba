# Keyboard shortcuts

On Windows and Linux every app shortcut uses Ctrl+Shift, so that plain Ctrl+letter always reaches the remote shell (Ctrl+L, Ctrl+W, Ctrl+K, Ctrl+C and so on). The few exceptions are Ctrl+Tab, Ctrl+, and Ctrl+K while focus is outside the terminal. On macOS the shortcuts use ⌘. If the user's platform is unknown, give both columns.

## App shortcuts

| Action | Windows / Linux | macOS |
|---|---|---|
| Search hosts (opens the host list and focuses the search box) | Ctrl+Shift+K (Ctrl+K also works when the terminal does not have focus) | ⌘K |
| New tab (also opens the host list and focuses the search box; connecting a host opens its own tab) | Ctrl+Shift+T | ⌘T |
| Close tab (a terminal tab; the home tab cannot be closed) | Ctrl+Shift+W | ⌘W |
| Next tab / previous tab | Ctrl+Tab / Ctrl+Shift+Tab | ⌃Tab / ⌃⇧Tab |
| Open Settings | Ctrl+, | ⌘, |
| Lock the vault | Ctrl+Shift+L | ⌘L |
| Show or hide the AI panel | Ctrl+Shift+A | ⌘⇧A |

| en | zh-CN | ja |
|---|---|---|
| New Tab | 新建标签 | 新しいタブ |
| Close Tab | 关闭标签 | タブを閉じる |
| Lock | 锁定 | ロック |
| Settings | 设置 | 設定 |
| Show AI Panel | 显示 AI 面板 | AI パネルを表示 |
| Hide AI Panel | 隐藏 AI 面板 | AI パネルを隠す |
| Search name, IP or tag | 搜索名称、IP 或标签 | 名前、IP、タグを検索 |
| Right-Click in Terminal | 终端中的右键 | ターミナルでの右クリック |

## Terminal

| Action | Windows / Linux | macOS |
|---|---|---|
| Copy the selection | Ctrl+C when text is selected (otherwise it is ^C for the remote), Ctrl+Shift+C, or Ctrl+Insert | ⌘C |
| Paste | Ctrl+V, Ctrl+Shift+V, or Shift+Insert | ⌘V |
| Find in the terminal | Ctrl+Shift+F | ⌘F |
| Open a link in the output | Ctrl+click | ⌘+click |

In the find bar, Enter goes to the next match, Shift+Enter to the previous one, and Esc closes the bar. Right-click either copies or pastes, or opens a menu, depending on **Right-Click in Terminal** (see `references/terminal.md`).

## In pages and panels

| Where | Keys |
|---|---|
| Host list | Up / Down select, Enter connects, typing starts a search |
| Host editor | Ctrl+S (⌘S) saves, Enter in a text field saves, Esc leaves a form that has no changes |
| SFTP panel | Up / Down select, Enter opens a folder or downloads a file, F2 renames, Delete deletes, Backspace goes to the parent folder |
| AI panel | Enter sends, Shift+Enter adds a line, Esc stops a running turn |
| Settings | Esc closes the window (a dialog or menu inside it takes Esc first); Left / Right switch tabs when the tab bar has focus |
| Tab bar | Middle-click a terminal tab to close it |

## Not available

There are no user-defined shortcuts and no shortcut to switch to tab number N.
