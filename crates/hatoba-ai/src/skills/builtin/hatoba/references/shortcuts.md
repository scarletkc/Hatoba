# Keyboard shortcuts

On Windows and Linux every app shortcut uses Ctrl+Shift, so that plain Ctrl+letter always reaches the remote shell (Ctrl+L, Ctrl+W, Ctrl+K, Ctrl+C and so on). The few exceptions are Ctrl+Tab, Ctrl+,, the zoom keys (below) and Ctrl+K while focus is outside the terminal. On macOS the shortcuts use ⌘. If the user's platform is unknown, give both columns.

## App shortcuts

| Action | Windows / Linux | macOS |
|---|---|---|
| Search hosts (opens the host list and focuses the search box) | Ctrl+Shift+K (Ctrl+K also works when the terminal does not have focus) | ⌘⇧K (⌘K also works when the terminal does not have focus, and clears the terminal while it has) |
| New tab (also opens the host list and focuses the search box; connecting a host opens its own tab) | Ctrl+Shift+T | ⌘T |
| New local terminal (the shell chosen in Settings → Terminal) | Ctrl+Shift+` | ⌃⇧` |
| Close tab (a terminal tab; the home tab cannot be closed) | Ctrl+Shift+W | ⌘W |
| Next tab / previous tab | Ctrl+Tab / Ctrl+Shift+Tab | ⌃Tab / ⌃⇧Tab |
| Open Settings | Ctrl+, | ⌘, |
| Lock the vault | Ctrl+Shift+L | ⌘L |
| Show or hide the AI panel | Ctrl+Shift+A | ⌘⇧A |

Other labels: **New Tab**, **Close Tab**, **Show AI Panel**, **Hide AI Panel**, **Search hosts or ssh user@host**.

## Terminal

| Action | Windows / Linux | macOS |
|---|---|---|
| Copy the selection | Ctrl+C when text is selected (otherwise it is ^C for the remote), Ctrl+Shift+C, or Ctrl+Insert | ⌘C |
| Paste | Ctrl+V, Ctrl+Shift+V, or Shift+Insert | ⌘V |
| Find in the terminal | Ctrl+Shift+F | ⌘F |
| Open a link in the output | Ctrl+click | ⌘+click |

In the find bar, Enter goes to the next match, Shift+Enter to the previous one, and Esc closes the bar. Right-click either copies or pastes, or opens a menu, depending on **Right-Click in Terminal** (see `references/terminal.md`).

## Zoom

The same keys change the size of what has focus: in a terminal the font size of every terminal tab (Settings → Terminal → **Font Size**), in the AI panel the size of its conversation and input (Settings → AI → **Text Size**), and anywhere else the zoom of the whole window (Settings → Appearance → **Zoom**). Ctrl+wheel over a terminal changes its font size, also while a program there reads the mouse; it changes nothing else. A short label in the middle of what changed shows the new size.

| Action | Windows / Linux | macOS |
|---|---|---|
| Larger | Ctrl+= or Ctrl++ (also on the number pad); over a terminal also Ctrl+wheel up | ⌘= or ⌘+ |
| Smaller | Ctrl+- (also on the number pad); over a terminal also Ctrl+wheel down | ⌘- |
| Default size | Ctrl+0 (also on the number pad) | ⌘0 |

The default sizes are 100% for the window and 13 px for the terminal and the AI text. macOS has no wheel zoom. Ctrl+Shift+- still sends `^_` (readline's undo) to the terminal.

## In pages and panels

| Where | Keys |
|---|---|
| Host list | Up / Down select, Enter connects, typing starts a search |
| Host editor | Ctrl+S (⌘S) saves, Enter in a text field saves, Esc leaves a form that has no changes |
| SFTP panel | Up / Down select, Enter opens a folder or downloads a file, F2 renames, Delete deletes, Backspace goes to the parent folder |
| AI panel | Enter sends, or queues the message while a turn runs; Ctrl+Enter (⌘↩) sends at once, stopping a running turn; Alt+Enter (⌥↩) asks a side question; Shift+Enter adds a line; Up in the empty box edits the queued message; Esc stops a running turn, then sends the queued message |
| Settings | Esc closes the window (a dialog or menu inside it takes Esc first); Left / Right switch tabs when the tab bar has focus |
| Tab bar | Middle-click a terminal tab to close it |

## Not available

There are no user-defined shortcuts and no shortcut to switch to tab number N.
