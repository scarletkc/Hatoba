# Terminal

Each SSH session, and each local terminal (below), is a tab with a full terminal (256 colors and truecolor, CJK wide characters and input methods, mouse support for programs such as vim and tmux). Inactive tabs keep running. There are no split panes, no session logging and no snippets.

## Local terminal tabs

A local terminal tab runs a shell of this computer beside the SSH tabs: open one from **New Local Terminal** (the arrow right after **New Tab** (+) in the tab bar), which lists the shells, or with Ctrl+Shift+` (⌃⇧` on macOS), which opens the shell chosen in Settings → Terminal → **Local Terminal Shell** (the system default unless changed). The shells:

- Windows: PowerShell (PowerShell 7, when installed), Windows PowerShell, Command Prompt, Git Bash (when Git for Windows is installed) and each installed WSL distribution. The system default is PowerShell 7, else Windows PowerShell.
- macOS and Linux: the login shell and the other shells in `/etc/shells`. The system default is the login shell; on macOS it starts as a login shell, as Terminal does.

A local tab has the same terminal settings, search, links, copy and paste as SSH tabs. Its status bar shows **This Computer** and the shell, and has no resource usage, port forwarding or SFTP. The shell starts in the home directory. Typing `exit` (a shell that exits with status 0) closes the tab; a shell that exits with an error leaves the tab with the exit status and **Restart**, and one that cannot start shows why with **Retry**. **More actions** has **Restart** in place of Reconnect, Disconnect and Edit Host. Locking does not end local shells, unless Settings → Security → **Disconnect all sessions when locked** is on; quitting Hatoba ends them. The AI assistant works in a local tab as in an SSH tab: see `references/ai-tools.md`.

Other labels: **Connect to Host**, **Starting…**, **Running**, **Failed to start**, **Exited**.

## The status bar

The bar above the terminal shows the connection state, the target `user@host:port`, "via" and the jump host's name when one is used, and the latency in ms once connected. The buttons on the right:

- **Show Resource Usage** turns on the resource usage readout described below. It is enabled only when connected.
- **Find in terminal** opens the find bar.
- **Port Forwarding** (a badge shows how many forwards run) opens the forwards popover; see `references/port-forwarding.md`. It shows only for a host with forwarding rules, and is enabled only when connected.
- **SFTP** shows or hides the file panel beside the terminal; see `references/sftp.md`. It is enabled only when connected.
- **More actions** (the … button) opens the tab menu below.

Other labels: **Connecting…**, **Connected**, **Connection failed**, **Disconnected**, **Show or hide the SFTP file panel**.

## Resource usage

For a Linux server, the status bar can show CPU, memory and network rates (↓ received, ↑ sent). It is off until **Show Resource Usage** turns it on, and the choice is remembered for the host on this device; a quick connection keeps it only for its tab. A narrow window drops the rates, then everything but the gauge icon.

Clicking the readout opens a popover: CPU over the last 3 minutes with the number of cores and the load averages, memory and swap, network rates over the last 3 minutes with the interfaces counted (those of the default route), the disk usage of `/`, and the uptime. Pointing at a chart (or the arrow keys once it has focus) shows every value as it was at that moment. A meter turns orange at 80% and red at 95%. **Hide Resource Usage** at the bottom turns it off for the host.

Hatoba reads `/proc` on the server every 2 seconds over the tab's own connection, by running `sh` on a separate channel; nothing is installed, and the process shows on the server (for example in `ps`) while the readout is sampled. Sampling pauses while the tab is in the background, the window is minimized, or the vault is locked, and a pause shows as a gap in the charts. The first reading after a pause has no CPU or network rate yet ("—"); the next one, 2 seconds later, does.

- FreeBSD, NetBSD, macOS, Windows and other systems show "Resource usage is available only for Linux servers." with the system's name when known.
- If the readings stop (for example the server refused to run the command, or `sh` exited), the popover shows the reason and **Retry**.
- In a container, `/proc` describes the host machine, so CPU and memory are the host's, not the container's.

Other labels: **Memory**, **Swap**, **No swap**, **Network**, **Disk**, **Reading…**.

## Menus

The tab menu (**More actions**) and the right-click menu share most entries. Right-click shows the menu when **Right-Click in Terminal** is set to **Show context menu** (see Settings below), the default on macOS; elsewhere right-click by default copies the selection, or pastes when nothing is selected. A program that tracks the mouse (vim, tmux) gets the right click itself unless Shift is held.

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

- Pasting a single line drops its trailing line breaks, so a copied command waits for Enter instead of running. Text with line breaks inside is pasted unchanged, after **Paste multiple lines?** with a preview; **Paste** confirms. The check can be turned off in Settings → General (**Confirm before pasting multiple lines**). Pasting needs a connected tab.
- The find bar searches the visible screen and the scrollback. Enter goes to the next match, Shift+Enter to the previous one, Esc closes the bar. It shows "No results" or the position as i/n.
- HTTP and HTTPS links in the output open in the default browser with Ctrl+click (⌘+click on macOS), including local HTTP services. Other schemes are not opened. A plain click only selects text.

Other labels: **Previous (Shift+Enter)**, **Next (Enter)**, **Close find (Esc)**, **Copy if selected, otherwise paste**.

## Terminal settings

The terminal settings sync with the vault. They are in three places:

- Settings → Terminal: font, size, cursor and scrollback, with a live preview, and **Local Terminal Shell**, the shell new local tabs run, which is kept on this device only (not synced).
- Settings → Appearance: **Terminal Colors**.
- Settings → General: **Right-Click in Terminal** and **Confirm before pasting multiple lines**.

Other labels: **Decrease font size**, **Increase font size**.

- **Font**: the name of an installed font family (default Cascadia Mono, or Menlo on macOS). A font that is not installed falls back to the system monospace font.
- **Font Size**: 10 to 24 (default 13), changed with the minus and plus buttons.
- **Cursor Style**: Block (default), Bar or Underline.
- **Scrollback**: 1,000, 5,000, 10,000 (default), or 50,000 lines.
- **Terminal Colors**: **Always Dark** (default), **Always Light**, or **Match Appearance**, which follows the app's light or dark theme. Below it, the built-in color schemes show as small terminal previews; click one to use it:
  - Dark: Hatoba Dark (default), Dracula, Tokyo Night, Catppuccin Mocha, One Half Dark, Nord, Gruvbox Dark, Solarized Dark, Rosé Pine, Kanagawa, GitHub Dark.
  - Light: Hatoba Light (default), GitHub Light, One Half Light, Catppuccin Latte, Solarized Light, Gruvbox Light, Rosé Pine Dawn.
  - With **Match Appearance** both lists show, under **In Dark Appearance** and **In Light Appearance**, and the terminal switches between the two picks with the theme. Open terminals take a new pick when the Settings window closes.
  - Light schemes darken colors that would be hard to read on a light background, such as white or yellow text from programs written for dark terminals.
  - There are no custom schemes and no import from other terminals.
- Language, theme and list density are device-local settings in Settings → Appearance; see `references/settings.md`.

## Tab states

A tab can be connecting (orange dot, "Connecting to …" overlay), connected (green), failed (red, with a card to retry or edit the host), or disconnected (grey, with a banner to reconnect). See `references/hosts-and-connecting.md`. While a conversation of the AI assistant runs or waits for approval, the tab shows a small sparkle or hand icon.
