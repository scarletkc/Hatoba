---
name: hatoba
description: "How Hatoba works: its windows, menus, settings and features, and where to find them. Read it when the user asks how to do something in Hatoba or about a Hatoba setting or error."
---

# Hatoba

Hatoba is a desktop SSH client (Tauri 2: a Rust backend with a React interface). It keeps hosts, SSH keys and settings in an encrypted vault on the device, and offers terminal tabs for SSH sessions and for the shells of this computer, an SFTP file panel, local port forwarding, optional end-to-end encrypted sync, and an AI assistant panel. Windows is the first platform; macOS and Linux follow. The interface is available in Simplified Chinese (zh-CN), English (en) and Japanese (ja).

- Local first: everything works offline. Hatoba runs no server and collects no telemetry.
- Sync is optional. It goes through a Worker and a D1 database in the user's own Cloudflare account, or straight to D1. The cloud holds only ciphertext.
- The AI assistant calls a model provider that the user adds with their own API key. Nothing is sent to a model until the user adds a provider and sends a message.

## Window layout

- Sidebar (left). The header has **Settings** (gear) and **Show or hide the sidebar**. Below: **Dashboard** (every host's status and resource usage), **All Hosts**, **Favorites**, **Recent**; **Groups** with **New Group** (right-click a group for Rename Group and Delete Group); **Tags** (only when some host has a tag); and a **Vault** heading with **Keys** and **Cloud Sync**. At the bottom: the sync status button (opens Cloud Sync) and **Lock**. A card asking for a GitHub star appears occasionally.
- Tab bar (top, merged with the title bar). The first tab is the home tab. It shows the page chosen in the sidebar: the dashboard, the host list, the host editor, Keys or Cloud Sync. Each open SSH session gets its own tab after it, with a status dot (orange connecting, green connected, red failed, grey disconnected), and the same host can be open in several tabs; a local terminal tab (a shell of this computer) shows a terminal icon. Then **New Tab** (+), which goes to the host list and focuses the search box, **New Local Terminal** (the arrow after it), which lists this computer's shells, and the AI button (sparkle). Windows and Linux show their own minimize, maximize and close buttons here; macOS uses its native window buttons.
- Terminal tab. A status bar sits above the terminal: connection state, `user@host:port`, the jump host ("via"), latency in ms, and the buttons **Show Resource Usage** (or the resource usage readout once it is on), **Find in terminal**, **Port Forwarding**, **SFTP** and **More actions**. The SFTP file panel opens to the right of the terminal.
- AI panel. At the window's right edge, below the tab bar. It shows the active tab's conversation. Open it with the AI button or Ctrl+Shift+A (⌘⇧A on macOS).
- Settings. A window with the tabs General, Appearance, Terminal, Proxies, Security, AI and About. Open it with the gear in the sidebar or with Ctrl+, (⌘, on macOS).
- System tray (Windows and Linux). An icon in the notification area while Hatoba runs, with **Open Hatoba**, **Lock** and **Quit Hatoba**; a left click opens the window. Closing the window asks whether to minimize to the tray or quit (Settings → General → **Close Button**). macOS has no tray: its red button hides the window and the Dock icon brings it back.
- Lock screen and first launch. Both replace the whole window: the lock screen asks for the master password, and first launch offers to create a vault or restore one from the cloud.

Other labels: **This Computer**, **Close Tab**, **Show AI Panel**, **Hide AI Panel**, **Minimize**, **Maximize**, **Restore Down**, **Minimize to Tray**, **Connected**, **Connecting…**, **Connection failed**, **Disconnected**.

## Reference files

Read the file that matches the question with `read_skill`, giving the path shown here.

| File | Read it when the question is about |
|---|---|
| `references/hosts-and-connecting.md` | The host list and editor, quick connect from the search box, authentication methods, jump hosts, proxies, environment variables, importing an SSH config, host key prompts, connecting and reconnecting |
| `references/dashboard.md` | The Dashboard: every host's status and resource usage, cards and table, sorting, thresholds, how hosts are read, hosts that need a password or failed to sign in, leaving hosts out |
| `references/keys-vault-and-lock.md` | The master password, recovery code, locking and auto-lock, Windows Hello and Touch ID, the PIN, the encrypted backup, and the Keys page (import, generate, deploy) |
| `references/terminal.md` | The terminal tab: local terminal tabs and their shells, status bar, copy and paste, search, links, context menus, and the Terminal settings (font, cursor, scrollback, local terminal shell) |
| `references/sftp.md` | The SFTP file panel: browsing, upload, download, rename, delete, transfers |
| `references/port-forwarding.md` | Local port forwarding: creating rules, starting them with the connection, starting and stopping them in a session |
| `references/sync.md` | Cloud Sync: what syncs, the setup wizard, joining a cloud vault from a device that has its own, the status page, devices, conflicts, restoring on a new device, disconnecting |
| `references/sync-worker.md` | The sync Worker: deploying it from Hatoba, connecting a Worker deployed by hand, direct D1 mode, upgrading the Worker |
| `references/ai-panel.md` | Using the AI assistant: the panel, one conversation per tab, moving between hosts, `/` commands (`/btw`, `/clear`, `/compact`, `/mcp`, `/memory`, `/model`, skills, MCP prompts) and `@` mentions, the + menu (Slash Commands, Skills, MCP Tools), the MCP servers of a conversation |
| `references/ai-tools.md` | What the AI assistant can call and which calls wait for the user: the tools, permission modes (Manual approval, Auto, Bypass), approval cards, Allow for This Conversation and Always Allow |
| `references/ai-history.md` | AI conversation history: search, open, rename, pin, export and delete, Edit and Resend, and forking a conversation into a new one |
| `references/ai-models.md` | The model picker, the Thinking Level (Default to Max), the context meter, Compact and automatic compaction, the reasoning display |
| `references/ai-file-changes.md` | Undoing the assistant's file edits: **Revert** on an Edit file or Write file call, where the earlier content is kept, when Revert refuses, what it does not undo, and what **Edit and Resend** asks about the files the deleted calls changed |
| `references/ai-attachments.md` | What goes with an AI message: the terminal's screen, the terminal selection, Ask AI about a failed connection, long pastes, text files, size limits and the context-fit note |
| `references/ai-side-questions.md` | Asking the assistant something while it works or an approval card waits, without stopping it: **Ask a Side Question**, Alt+Enter, **Ask About This** on an approval card, what is not saved, stopping a side question |
| `references/ai-instructions.md` | Custom Instructions in Settings → AI, a host's AI Notes, and the assistant's Memory: what they are for, limits, saving and sync, how the assistant follows them, what it remembers and how to change or turn that off |
| `references/ai-settings.md` | Settings → AI: providers and models (including each model's thinking levels and System One providers), web search, default model and thinking level, the terminal screen switch, Auto mode's judge and its Test, permission mode, saved approvals, skills (including the built-in `hatoba` skill) |
| `references/ai-chatgpt.md` | Sign in with ChatGPT: **Continue with ChatGPT**, a ChatGPT provider's account and models, signing in on each device, Sign Out, usage limits and sign-in errors |
| `references/ai-mcp.md` | MCP servers in Settings → AI: adding a Command or HTTP server, templates, Always ask, import and export, a server's state, tools and Always allow |
| `references/shortcuts.md` | Keyboard shortcuts on Windows/Linux and macOS |
| `references/settings.md` | General, Appearance, Proxies, Security and About settings, language, checking for and installing updates, Report a Problem, and which settings sync |
| `references/troubleshooting-connections.md` | A failed or dropped SSH connection: every error kind (dns, refused, timeout, authentication, host key, proxy, …), jump hosts, proxies, host key prompts, keepalive and the disconnect banner, and how to read the diagnostics that **Ask AI** attaches |
| `references/troubleshooting.md` | Key, master password, recovery, SFTP, update and other error messages |
| `references/troubleshooting-sync.md` | Sync and Cloudflare errors and what to do |
| `references/troubleshooting-ai.md` | AI provider, search and MCP errors and what to do |

These files name labels in English. The label files give them in the other languages, by file and section:

| Files | Simplified Chinese | Japanese |
|---|---|---|
| This file, hosts and connecting, the dashboard, shortcuts | `references/labels/zh-CN-app.md` | `references/labels/ja-app.md` |
| Terminal, SFTP, port forwarding, connection errors | `references/labels/zh-CN-terminal.md` | `references/labels/ja-terminal.md` |
| Keys, vault and lock, settings, other errors | `references/labels/zh-CN-vault.md` | `references/labels/ja-vault.md` |
| Sync, the sync Worker, sync errors | `references/labels/zh-CN-sync.md` | `references/labels/ja-sync.md` |
| The AI panel, tools, history, models, file changes, attachments, side questions | `references/labels/zh-CN-ai.md` | `references/labels/ja-ai.md` |
| AI instructions and memory, AI settings, ChatGPT, MCP, AI errors | `references/labels/zh-CN-ai-settings.md` | `references/labels/ja-ai-settings.md` |

## How to answer

- Name UI elements by their exact label in the language the user writes in (zh-CN, en or ja). For Chinese or Japanese, read the label file that covers the topic and copy the label as it is; do not translate it yourself. If a label is not there, describe the element and where it is instead of guessing its text.
- Give menu paths with an arrow, such as Settings → AI (设置 → AI, 設定 → AI). Settings tabs are listed above; a setting's tab is named in its section.
- Mention platform differences. Shortcuts use Ctrl+Shift+letter on Windows and Linux and ⌘ on macOS. Windows Hello is on Windows only, Touch ID on macOS only. The SSH agent is the OpenSSH agent on Windows and the `SSH_AUTH_SOCK` agent elsewhere. If the user's platform is unknown, give both.
- Say plainly when Hatoba does not support something, and do not invent a feature, menu or setting. Offer a real workaround only when one exists, such as running `ssh` or `scp` in the terminal.
- Treat these files as the description of the current version. If the user describes a screen that differs, say that their version may differ and point to Settings → About for the version and the update check.
- Never ask the user to paste a master password, recovery code, API key, token or private key. For connection problems, ask for the error text, or for the **Copy Diagnostics** output; **Ask AI** on the failed connection's card attaches the same secret-free diagnostics to a message when the user sends it.
- Keep answers short and in steps. Answer from these files first, and use web tools only for things outside Hatoba, such as Cloudflare or a provider's documentation.

## Project, feedback and newer information

Hatoba is open source: https://github.com/scarletkc/Hatoba (README, docs, releases, issues). This skill describes Hatoba {{HATOBA_VERSION}}.

*Feedback.* When the user hits what looks like a bug, or asks for something Hatoba does not do, suggest reporting it. Do not send them there for a question this skill answers.

- A bug: **Report a Problem** in Settings → About (the star card in the sidebar has it too). It opens GitHub's bug report form with the Hatoba version and the operating system filled in.
- A feature request: https://github.com/scarletkc/Hatoba/issues/new?template=feature_request.yml (the "Suggest a feature" form: the problem, where it happens, what happens if nothing changes, a proposed solution).
- A security vulnerability: report it privately at https://github.com/scarletkc/Hatoba/security/advisories/new, not as a public issue.
- Offer to draft the text: what happened, the steps, what was expected, the version (Settings → About) and OS, and for a connection or sync problem the SSH server, the authentication method, ProxyJump, a proxy or ssh-agent, or whether sync uses a Worker or direct D1. Remind the user to leave out host names, addresses, keys, passwords, tokens, recovery codes and other secrets, and to read any logs before pasting them (on Windows they are in `%LOCALAPPDATA%\app.hatoba.desktop\logs`). The user submits the report; never submit anything on their behalf.

*Newer information.* This skill describes the build it ships with. When the user asks about the latest version, recent changes, or whether a feature exists in a newer release, use `fetch_url` (when offered; in manual approval mode it asks first) or `web_search` on these pages:

- https://github.com/scarletkc/Hatoba/releases/latest
- https://hatoba.fog.moe/docs/ (these docs as of the newest release; each page names the version it describes)
- https://api.github.com/repos/scarletkc/Hatoba/releases/latest (JSON)
- https://raw.githubusercontent.com/scarletkc/Hatoba/main/README.md
- https://raw.githubusercontent.com/scarletkc/Hatoba/main/docs/status.md

The `main` branch can be ahead of the latest release and of the user's version, so say where a statement comes from. Compare the latest release with {{HATOBA_VERSION}}. If a newer release exists, point to **Check for Updates** in Settings → About, or to the dot on the sidebar's gear (**Settings (update available)**), which opens About. Once a check finds the newer version, **Download and Install** there downloads it, checks its signature, then closes Hatoba to install it and opens it again; see `references/settings.md`.

## Not in Hatoba (current version)

Split panes in a terminal, session logs, snippets, remote (-R) and SOCKS (-D) port forwarding, editing remote files, uploading or downloading whole folders over SFTP, importing PuTTY sessions, `ProxyCommand`, SOCKS4 and HTTPS proxies, Pageant, viewing or exporting a private key, a plaintext export, importing an encrypted backup file from the app, team sharing, a mobile app, a sync service run by Hatoba, and protocols other than SSH (Telnet, serial, RDP, VNC).
