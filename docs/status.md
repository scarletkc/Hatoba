# Implementation status

This page tracks implementation progress, items awaiting verification, and follow-up work against the requirement IDs in the [architecture and requirements](hatoba-spec.md). That document defines the requirements, and this page records only their status. Its [§12 Testing](hatoba-spec.md#12-testing) describes what each test suite covers.

✅ Implemented, with tests or hands-on verification. 🟡 Implemented, but needs verification on a real Windows, Linux, or macOS machine or has not been tried against the real service yet. ✅ / 🟡 Verified in part; the notes name the part that is not. ⬜ Not implemented.

## Security (§4.3)

| ID | Name | Status | Notes |
|---|---|---|---|
| SEC-01 | Secrets only in Rust memory | ✅ | |
| SEC-02 | Auto-lock | 🟡 | The system-wide idle time on Windows (`GetLastInputInfo`) tried on Windows 11. On macOS (`CGEventSourceSecondsSinceLastEventType`) it needs verification on a real machine; on Linux only input in Hatoba's window counts |
| SEC-03 | Keep sessions while locked | ✅ | |
| SEC-04 | No secrets in logs | ✅ | Confirmed by scanning the logs after the end-to-end test |
| SEC-05 | Tauri hardening | ✅ | |
| SEC-06 | Increasing delay after failed unlocks | ✅ | |
| SEC-07 | Windows Hello | ✅ | Tried on Windows 11 |
| SEC-08 | Clipboard auto-clear | ✅ | |
| SEC-09 | Password strength hint | ✅ | |
| SEC-10 | Touch ID | 🟡 | Tried on one Mac (Apple Silicon, macOS 27, ad-hoc signed): turning it on, unlocking, unlocking after an app update, and the record failing once a fingerprint was added. Not yet tried: macOS 13, a Mac without Touch ID, a vault moved with Migration Assistant, and the switch turning off by itself after a failure like that |
| SEC-11 | PIN unlock | 🟡 | Tried on Windows 11, including keeping the PIN in Windows Credential Manager after a restart. Keeping it after a restart not yet tried with the Secret Service on Linux or the Keychain on macOS |

## SSH, terminal, SFTP, and forwarding (§7)

| ID | Name | Status | Notes |
|---|---|---|---|
| SSH-01…07 | Connection, authentication, fingerprints, timeouts, reconnect | ✅ | |
| SSH-08 | keyboard-interactive | ✅ | |
| SSH-09 | ssh-agent | 🟡 | The Windows named pipe needs verification on a real machine. `IdentityAgent` and the login shell's `SSH_AUTH_SOCK` have unit tests and tests against a real `ssh-agent`, but have not been tried in the app with Bitwarden or 1Password. Pageant (P2) ⬜ |
| SSH-10 | ProxyJump | ✅ | |
| SSH-11 | Import ssh config | ✅ | |
| SSH-12 | Import PuTTY sessions | 🟡 | Unit tests parse a `.reg` export and the session files of macOS and Linux; reading the registry not yet tried in the app on Windows |
| SSH-13 | SOCKS5 and HTTP proxies | 🟡 | Tests run against in-process proxies; needs a try in the app with a real proxy |
| SSH-14 | Per-host environment variables | 🟡 | Integration tests send them to OpenSSH on Linux; needs a try in the app against a real server |
| SSH-15 | Import from Xshell, FinalShell, MobaXterm, SecureCRT, and Tabby | 🟡 | Unit tests parse sample files written to each client's format; not yet tried with sessions saved by the real clients |
| SSH-16 | One connection per host for its tabs and forwards | ✅ | Tried in the app against real servers: tabs and forwards on one connection, the fifth tab's connection, a server that refuses a shell, a tab joining a connection the network dropped, and a lost connection |
| TERM-01…06 | Tabs, colors, wide characters, PTY, paste, scrollback, appearance | ✅ / 🟡 | The candidate window position for Microsoft Pinyin tried on Windows 11; for the Japanese IME it needs verification on a real Windows machine. Dragging a tab to move it (TERM-01) tried in the browser with the mock backend; needs a try in the app on Windows, Linux, and macOS |
| TERM-07, 08 | Terminal search, links | ✅ | |
| TERM-09…11 | Split panes, session logs, Snippets | ⬜ | P2 |
| TERM-12 | Resource usage in the status bar | ✅ | Integration tests run the sampling against OpenSSH on Linux, and it was tried in the app against real servers |
| SFTP-01…04 | File panel, transfers, file operations, attributes | ✅ | |
| SFTP-05 | Edit remote files | ⬜ | P2 |
| FWD-01, 02 | Local forwarding, start with the connection | ✅ | |
| FWD-03, 04 | Remote forwarding, SOCKS | ✅ | Integration tests against OpenSSH (`it_forward.rs`, CI), and tried in the app against real servers |
| FWD-05 | Port Forwarding page, forwards on the host's connection | ✅ | Tried in the app against real servers: the Port Forwarding page, start, stop, the terminal popover, an auto-start failure, the tray menu, and a lost connection's message and notification |
| FWD-06 | Forward names | 🟡 | Tried in the web preview with the mock backend. Not yet tried in the app, where the tray menu shows the names too |
| LOCAL-01, 02 | Local terminal tabs, the shells | 🟡 | Tried in the app on Windows 11 with PowerShell 7, Windows PowerShell, Command Prompt, Git Bash and WSL Ubuntu; unit tests run Command Prompt in ConPTY. Not yet tried on macOS or Linux |
| LOCAL-03 | The assistant in local tabs | 🟡 | Tried in the app on Windows 11 against a mock model: `run_command`, `read_file`, `grep`, `glob`, `send_input`, `read_terminal` and a timeout in PowerShell, `run_command` and `read_file` in WSL Ubuntu, `run_command` in Command Prompt. Used in the app on Windows 11 with real providers, including `edit_file`, `write_file` and their revert. Unit tests cover the native file and search tools on every platform, and PowerShell, Command Prompt, Git Bash and WSL commands and their stops on Windows. The read-only check, allowances and Auto's hard rules read PowerShell and Command Prompt syntax (§13.5): unit tests, and in the app on Windows 11 read-only PowerShell and Command Prompt commands ran without a card in manual approval while `rm` asked. Not yet tried on macOS or Linux |

## Vault, hosts, and keys (§8)

| ID | Name | Status | Notes |
|---|---|---|---|
| VAULT-01…07 | Master password, recovery code, unlock, password change, encrypted backup | ✅ | |
| VAULT-08 | Plaintext export | ⬜ | P2 |
| HOST-01…10 | Host management, search, online probing | ✅ | |
| HOST-11 | Server OS icon | ✅ | |
| HOST-12 | Quick connect | 🟡 | Covered by tests against an in-process SSH server; needs a try in the app against a real server |
| HOST-13 | Open from the command line | 🟡 | Tried with the installer on Windows (the `PATH`), and the deb package and the AppImage on Linux. Not yet tried: the uninstaller removing it from the `PATH`, and the app bundle on macOS |
| HOST-15 | Other programs use the saved hosts through the `hatoba` command | 🟡 | Unit tests cover the protocol, the arguments, finding a host and the host list without secrets; the sshd integration tests cover streamed output and its stop on Linux. Tried on Windows with a test build against an in-process SSH server that names itself OpenSSH for Windows: `hosts`, `exec` with streamed output and exit status, `cp` both ways, the host-key and password questions in the window, `--no-prompt`, one kept connection per host, the stop when `hatoba` is killed, the lock, the pipe's ACL, and `hatoba <host>` opening the host. Not yet tried: a Linux server from Windows, the launchers and socket on Linux and macOS, and the installer's `bin` copy |
| HOST-14 | Dashboard | ✅ | First version. Unit tests cover the filesystem and traffic parsing, holds, versions, the probe key, and the limit of 8 setups, and an integration test reads the filesystems from OpenSSH on Linux; it was tried in the app against real servers. Not yet tried with 30 hosts behind one jump host. The later items of [#172](https://github.com/scarletkc/Hatoba/issues/172) (the assistant's `host_status` tool, billing, traffic per cycle, notifications, more readings, custom readings, this computer, one connection per jump host) ⬜ |
| KEY-01…06 | Import, generation, list, public key deployment | ✅ | |
| KEY-07 | View private key | ⬜ | P2 |

## Sync (§6)

| Item | Status | Notes |
|---|---|---|
| Worker mode (§6.2) | ✅ | |
| D1 direct mode (§6.1) | 🟡 | Tested against a SQLite mock of the REST API, not yet against real Cloudflare D1 |
| Sync engine (§6.3) | ✅ | |
| Conflict resolution (§6.4) | ✅ | |
| Flows A and B (§6.6) | ✅ | |
| Flow C (§6.6) | ✅ | |
| Device management | ✅ | |
| Deploy to Cloudflare button (§6.6) | ✅ | The wizard links to it, and the `deploy` script applies the D1 migrations. Run through the deploy flow with a real Cloudflare account |
| In-app deployment (§6.7, DEPLOY-01…07) | 🟡 | Tested against a stand-in for the Cloudflare API, not yet against a real account |
| Worker upgrades from the app (§6.7, DEPLOY-08) | ✅ | Tested against a stand-in for the Cloudflare API, and tried against a real account |
| Tombstone purge (§6.5) | ⬜ | P2 |
| Rollback detection (§4.4) | ⬜ | P2 |

## Windows adaptation (§9.1)

| ID | Name | Status | Notes |
|---|---|---|---|
| WIN-01 | Custom title bar and Snap Layouts | ✅ | Tried on Windows 11 |
| WIN-02 | Font mapping | ✅ | |
| WIN-03 | High DPI and multiple monitors | 🟡 | Needs verification on a real machine at 100–200% scaling |
| WIN-04, 05 | Shortcuts, terminal copy and paste | ✅ | |
| WIN-06 | WebView2 bootstrapper | ✅ | |
| WIN-07 | Mica backdrop | 🟡 | Needs verification on a real machine |
| WIN-08 | Follow the system light or dark mode | ✅ | |
| WIN-09 | PuTTY `.ppk` | ✅ | |
| WIN-10 | System tray and the close button | 🟡 | Tried on Windows 11; not yet on a Linux desktop |

## macOS (§9.2)

A contributor tried these by hand on an Apple Silicon Mac; the maintainer has no Mac.

| ID | Name | Status | Notes |
|---|---|---|---|
| MAC-01 | Traffic-light buttons in the title bar | ✅ | |
| MAC-02 | Fonts | ✅ | The Menlo default of a new vault is covered by unit tests only |
| MAC-03 | Shortcuts | ✅ | |
| MAC-04 | Secondary click opens the menu | ✅ | |
| MAC-05 | Retina displays and file drops | 🟡 | Dropping on the SFTP panel tried; dropping on the AI panel not yet |
| MAC-06 | Closing the window hides it, the Dock reopens it | ✅ | |
| MAC-07 | Apps opened from Finder or the Dock | 🟡 | The MCP `PATH` tried; the device list with real sync and conflict copies in another language not yet |
| MAC-08 | Lock before sleep | ⬜ | Only the gap detector notices sleep, after wake ([#52](https://github.com/scarletkc/Hatoba/issues/52)) |

## Release (§11)

| Item | Status | Notes |
|---|---|---|
| NSIS installer | ✅ | Built by the `windows-installer` job in [CI](../.github/workflows/ci.yml) on pushes to `main` and manual runs (not on pull requests). Unsigned |
| Linux deb package and AppImage | ✅ | Built by the `linux-packages` job in [CI](../.github/workflows/ci.yml) on pushes to `main` and manual runs (not on pull requests). Both installed and run on a Linux desktop |
| macOS app bundle and DMG | 🟡 | Built by the `macos-bundle` job in [CI](../.github/workflows/ci.yml) on pushes to `main` and manual runs (not on pull requests), for Apple Silicon, and published as an experimental build in each release by the [Release workflow](../.github/workflows/release.yml). The CI build also signs its updater archive with a throwaway key and checks it with the release script. Ad-hoc signed (`signingIdentity` `-`), not notarized. Opening a downloaded DMG through Gatekeeper not yet tried |
| GitHub Releases | ✅ | The [Release workflow](../.github/workflows/release.yml) publishes the installers after approval, and has published Hatoba's releases; [Release Hatoba](releasing.md) has the steps |
| Authenticode code signing | Not planned | A certificate is impractical to obtain ([§11](hatoba-spec.md#11-non-functional-requirements)), so SmartScreen warns on the first install |
| Update check in Settings → About | ✅ | Reads `latest.json` through the Tauri updater: stable versions from the latest release, prereleases from the newest release in the GitHub API. Unit tests run against a mock endpoint and a mock release list. The automatic check runs after the first unlock and again a day later while Hatoba runs, and the tray menu offers the update (unit tests). Tried on Windows against a published release |
| Star prompt and **Report a Problem** (§9) | ✅ | Unit tests cover when the prompt shows and the bug report link. Filling in the bug report form tried on GitHub |
| Signed updates through the Tauri updater | 🟡 | **Download and Install** in Settings → About. Tried end to end on Windows with a local endpoint ([Signed updates](development.md#signed-updates)) and against a published release, not yet on Linux or macOS |
| Updates of package-manager installs (Scoop, AUR) | 🟡 | A `managed-by-package-manager` marker in the resource folder turns the in-app update off, and the About page names the package manager instead. Unit tests cover reading the marker, the skipped automatic check, the missing sidebar dot and tray item, and the About page; tried in the browser mock (`?manager=scoop`). Not yet tried with a Scoop or AUR install |

## AI assistant (§13)

| ID | Name | Status | Notes |
|---|---|---|---|
| AI-01…06 | Providers, models, Test Connection, model selector and thinking level, reasoning | ✅ | Adapter tests replay recorded Chat Completions, Anthropic Messages, and Responses streams from a mock server, and check the thinking level's fields for each protocol, model, and level, and the retry without them. Tried in the desktop app against real providers over all three protocols |
| AI-52 | Sign in with ChatGPT: **Continue with ChatGPT**, this device's session, sign-out, the ChatGPT route's requests and errors | ✅ | Tests run the sign-in, the code exchange and the ID token check, refresh rotation with one refresh at a time, ended and revoked sessions, usage limits and the route's other errors, namespaced tool calls, and the streaming judge, Test Connection and model list against a mock of OpenAI's endpoints. Tried in the desktop app with a real ChatGPT account: the sign-in, conversations, the session across refreshes and restarts, and sign-out |
| AI-07…09 | One conversation per tab, attaching conversations | ✅ | A conversation with no connected tab is offered every tool but the terminal ones |
| AI-07 | System notifications when a conversation finishes a turn or waits for approval | 🟡 | Tried on Windows 11: in the tray, minimized, focused, and with the setting off. The banner and a click on it not yet tried (Do Not Disturb was on), nor Linux or macOS |
| AI-10 | Terminal selection and connection diagnostics as attachments, **Ask AI** | ✅ | |
| AI-35 | Long pastes and text files as attachments | ✅ | Dropping files on the panel tried in the desktop app on Windows 11 |
| AI-46 | The terminal's screen with each message: the `terminal_state` note, **Send Terminal Screen**, the Settings default | ✅ | Unit tests cover the note, how a long screen is cut and when it is sent. Tried in the desktop app with SSH and local terminals, including the Settings default |
| AI-42 | Side questions that leave a running turn alone | ✅ | Tried in the desktop app against real providers |
| AI-11…15 | Tools: `read_terminal`, `run_command`, `send_input`, `web_search`, `fetch_url` | ✅ / 🟡 | Tried in the desktop app against real servers and providers, `web_search` through the providers' native search. `send_input` keys other than Enter, Tab, Esc, Ctrl+C, Ctrl+D, and the arrows not yet tried in the desktop app, beyond a ConPTY test of F1–F12 and Alt+F in PowerShell and Command Prompt. `web_search` not yet tried against Brave, Tavily, or SearXNG |
| AI-38…40 | File tools: `read_file`, `edit_file`, `write_file`, with the diff on the approval card | ✅ | The scripts run against the local `sh` on Linux and macOS and against OpenSSH in the integration tests, and the tools were tried in the desktop app against a real server |
| AI-48, 49 | Search tools: `grep`, `glob` | ✅ | The scripts run against the local `sh`, `find` and `grep` on Linux and macOS and against OpenSSH in the integration tests, and the tools were tried in the desktop app against a real server. Under WSL they were tried with dash, bash in POSIX mode, and BusyBox 1.36's `sh`, `find` and `grep`, each with and without ripgrep 14: the results match, except that BusyBox's `grep` has no `-I`, so a binary file with a match is listed, and no `--null` |
| AI-41 | Revert a file tool's change | ✅ | The delete script runs against the local `sh` on Linux and macOS and against OpenSSH in the integration tests, and reverts were tried in the desktop app against a real server |
| AI-16…18 | Manual approval and bypass modes, read-only commands without asking, approval card, Stop | ✅ | |
| §13.1 | Calls of a response that only read and need no approval run together, their results stored in call order | ✅ | Unit tests cover the batches, the limits on calls, exec channels and searches, approvals, Stop and the lock, and the order of the stored results, and the integration tests run commands at once on a server whose `MaxSessions` refuses some of their channels. Tried in the desktop app against a real server and provider |
| AI-47 | Auto mode: a judge (a System One endpoint or a model that chats) that lets safe or requested calls skip the approval card, its hard rules, the verdict on each call, **Test** in Settings | ✅ | Unit tests cover the System One requests and the chat-model adapter against mock servers, the rules, what the judge reads, and the panel's flow. Tried with Claude Haiku and TypeSafe's Jev preview as the judge, and in everyday use: **Test** gave the expected decision on all 13 samples. Not yet tried with Cloudflare's Clef |
| AI-19 | Allow commands by prefix, pages by domain, or a tool, for a conversation or saved on the device | ✅ | |
| AI-20, 21 | Context meter, compaction | ✅ | |
| AI-22 | Automatic compaction | ✅ | |
| AI-23 | Conversation history | ✅ | |
| AI-24…26 | History search, Markdown export, edit and resend | ✅ | |
| AI-43 | Fork a conversation from a message or from History | ✅ | |
| AI-27, 28, 34 | Skills: management, import and export, `read_skill`, the built-in `hatoba` skill | ✅ | A test checks every label the built-in skill quotes against the i18n tables |
| AI-29…33 | MCP servers: `stdio` and `http`, tools, approvals, lifecycle, `mcpServers` import and export | 🟡 | Tests run mock `stdio` and Streamable HTTP servers. Not yet tried with real servers on Windows, such as ones started with `npx` |
| AI-44 | MCP server templates: GitHub, Linear, Context7, Cloudflare | 🟡 | Unit tests cover the filled-in form and what it saves, and the dialog was tried with the mock backend. None of the four templates has been tried against its service with a real token yet |
| AI-45 | Deferred MCP tools: `search_tools`, `call_mcp_tool`, and the `mcp_tools` note | 🟡 | Turn tests run against mock Chat Completions and Anthropic Messages servers with mock `stdio` and Streamable HTTP MCP servers, and the panel was tried with the mock backend. Not yet tried against real providers and servers |
| AI-36, 37 | Custom instructions, AI notes of a host | ✅ | |
| AI-53 | Memory: the `memory` tool, the `memory` note, global, host and local-terminal memories, **Settings → AI → Memory**, `/memory` | ✅ / 🟡 | Turn tests run the tool, the note and a compaction against a mock provider, including host memories and the note of a move to another host. Tried in the desktop app against a real provider: the tool saving notes that later conversations read, Settings, and `/memory`. Host and local-terminal memories are not yet tried in the desktop app |
| AI-50 | `/` commands: `/btw`, `/clear`, `/compact`, `/mcp`, `/memory`, `/model`, skills, and MCP prompts | ✅ / 🟡 | Unit tests cover the parsing and the menu, and mock `stdio` MCP servers cover `prompts/list` and `prompts/get`. `/btw`, `/clear`, `/compact`, `/memory`, `/model` and skills tried in the desktop app against real providers; `/mcp` and MCP prompts not yet tried with real servers |
| AI-51 | `@` mentions: files and folders on the tab's host, MCP resources, text files from this computer | ✅ / 🟡 | The listing script runs against the local `sh` on Linux and macOS, and mock `stdio` MCP servers cover `resources/list` and `resources/read`. Files and folders on a real server and on a local tab, and text files from this computer, tried in the desktop app; MCP resources not yet tried with real servers |

## MVP scope

Out of scope for the MVP: team sharing and multi-user vaults, mobile apps (the architecture leaves room for them, see [§3.4](hatoba-spec.md#34-mobile-readiness)), and a sync service hosted by Hatoba.

Hosts connect only over SSH. Telnet, Serial, and Mosh are proposed in [#60](https://github.com/scarletkc/Hatoba/issues/60). RDP, VNC, and FTP are not planned.

## Milestones

| Milestone | Scope | Acceptance criteria |
|---|---|---|
| M0 Skeleton | Monorepo, Tauri app shell, tokens extracted from the design, Windows custom title bar (WIN-01), basic layout and routing, IPC binding generation | On Windows 11 and Windows 10, the app starts and shows the design's main pages with fake data, and window dragging, maximizing, and Snap work |
| M1 Terminal | Host data held in memory. Password and key connections, multiple tabs, PTY size sync, fingerprint confirmation | Connects to an OpenSSH test server, vim and htop render correctly, Windows Chinese and Japanese input methods work, 50 MB of output does not stutter, and text is sharp at 150% scaling |
| M2 Local vault | Encryption, SQLite, creating, editing, and deleting hosts, groups, tags, and keys, unlock and auto-lock, recovery code | A search of the local database file finds no plaintext host name, password, or private key |
| M3 Sync | Worker + D1, sync engine, sync wizard and status page, device management | Two devices sync both ways, offline changes merge correctly, conflicts are handled per §6.4, and D1 holds only ciphertext |
| M4 SFTP and polish | SFTP panel, all empty and error states, shortcuts, ssh config import | Every P0 item in §7, §8, and §9 is done |
| M5 Release | Windows installer, auto-update, README, Worker deployment template | A new user can deploy the Worker and enable sync within 10 minutes by following the README |

## Open questions

1. Whether host online status probing (HOST-10) is on by default. The current implementation turns it on by default, and a setting turns it off.
2. Whether D1 direct mode ships in the first public release.
3. Whether one Worker deployment should serve several users (the current design is single-user).
4. Whether the in-app deployment (§6.7) offers a data location for the new D1 database. D1 accepts a jurisdiction (`eu`, `fedramp`, or `us`) or a location hint only when a database is created, and §6.7 sets neither.
5. How a breaking Worker change (a new `api` number) rolls out while devices on older apps still sync, for example whether one Worker serves both `api` numbers for a while.
6. Whether MCP servers that sign in with OAuth (§13.9) are supported, and where each device keeps their tokens. They cannot sync, because many services rotate the refresh token on each use. Windows Credential Manager holds at most 2,560 bytes per credential, which OAuth tokens often exceed, so a device-local table encrypted with vault_key is the likely place.
7. Whether MCP servers can run on the remote host over the tab's SSH connection, speaking `stdio` over an exec channel. It would reach data where it lives without opening ports, but `run_command` already covers much of the same ground.
