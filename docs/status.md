# Implementation status

This page tracks implementation progress, items awaiting verification, and follow-up work against the requirement IDs in the [architecture and requirements](hatoba-spec.md). That document defines the requirements, and this page records only their status. Its [§12 Testing](hatoba-spec.md#12-testing) describes what each test suite covers.

✅ Implemented, with tests or hands-on verification. 🟡 Implemented, but needs verification on a real Windows machine or has not been tried against the real service yet. ⬜ Not implemented.

## Security (§4.3)

| ID | Name | Status | Notes |
|---|---|---|---|
| SEC-01 | Secrets only in Rust memory | ✅ | |
| SEC-02 | Auto-lock | ✅ | |
| SEC-03 | Keep sessions while locked | ✅ | |
| SEC-04 | No secrets in logs | ✅ | Confirmed by scanning the logs after the end-to-end test |
| SEC-05 | Tauri hardening | ✅ | |
| SEC-06 | Increasing delay after failed unlocks | ✅ | |
| SEC-07 | Windows Hello | 🟡 | Needs verification on a real Windows machine |
| SEC-08 | Clipboard auto-clear | ✅ | |
| SEC-09 | Password strength hint | ✅ | |
| SEC-10 | Touch ID | ⬜ | P2, with the macOS version |

## SSH, terminal, SFTP, and forwarding (§7)

| ID | Name | Status | Notes |
|---|---|---|---|
| SSH-01…07 | Connection, authentication, fingerprints, timeouts, reconnect | ✅ | |
| SSH-08 | keyboard-interactive | ✅ | |
| SSH-09 | ssh-agent | 🟡 | The Windows named pipe needs verification on a real machine. Pageant (P2) ⬜ |
| SSH-10 | ProxyJump | ✅ | |
| SSH-11 | Import ssh config | ✅ | |
| SSH-12 | Import PuTTY sessions | ⬜ | P2 |
| TERM-01…06 | Tabs, colors, wide characters, PTY, paste, scrollback, appearance | ✅ / 🟡 | The candidate window position for Microsoft Pinyin and the Japanese IME needs verification on a real Windows machine |
| TERM-07, 08 | Terminal search, links | ✅ | |
| TERM-09…11 | Split panes, session logs, Snippets | ⬜ | P2 |
| SFTP-01…04 | File panel, transfers, file operations, attributes | ✅ | |
| SFTP-05 | Edit remote files | ⬜ | P2 |
| FWD-01, 02 | Local forwarding, start with the connection | ✅ | |
| FWD-03, 04 | Remote forwarding, SOCKS | ⬜ | P2 |

## Vault, hosts, and keys (§8)

| ID | Name | Status | Notes |
|---|---|---|---|
| VAULT-01…07 | Master password, recovery code, unlock, password change, encrypted backup | ✅ | |
| VAULT-08 | Plaintext export | ⬜ | P2 |
| HOST-01…10 | Host management, search, online probing | ✅ | |
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
| Flow C (§6.6) | ⬜ | P1 |
| Device management | ✅ | |
| Deploy to Cloudflare button (§6.6) | 🟡 | The wizard links to it, and the `deploy` script applies the D1 migrations. Needs a full run through the deploy flow |
| In-app deployment (§6.7, DEPLOY-01…07) | 🟡 | Tested against a stand-in for the Cloudflare API, not yet against a real account |
| Worker upgrades from the app (§6.7, DEPLOY-08) | ⬜ | P1. Also needs the Worker version separated from the app version, which the release scripts still bump together, and the CI check of the Worker version rule |
| Tombstone purge (§6.5) | ⬜ | P2 |
| Rollback detection (§4.4) | ⬜ | P2 |

## Windows adaptation (§9.1)

| ID | Name | Status | Notes |
|---|---|---|---|
| WIN-01 | Custom title bar and Snap Layouts | 🟡 | Needs verification on a real machine |
| WIN-02 | Font mapping | ✅ | |
| WIN-03 | High DPI and multiple monitors | 🟡 | Needs verification on a real machine at 100–200% scaling |
| WIN-04, 05 | Shortcuts, terminal copy and paste | ✅ | |
| WIN-06 | WebView2 bootstrapper | ✅ | |
| WIN-07 | Mica backdrop | 🟡 | Needs verification on a real machine |
| WIN-08 | Follow the system light or dark mode | ✅ | |
| WIN-09 | PuTTY `.ppk` | ✅ | |

## Release (§11)

| Item | Status | Notes |
|---|---|---|
| NSIS installer | ✅ | Built by the `rust-windows` job in [CI](../.github/workflows/ci.yml) on pushes to `main` and manual runs (not on pull requests). Unsigned |
| GitHub Releases | 🟡 | The [Release workflow](../.github/workflows/release.yml) publishes the installer after approval; [Release Hatoba](releasing.md) has the steps |
| Authenticode code signing | ⬜ | P1. Azure Trusted Signing could keep the cost down |
| Update check in Settings → About | 🟡 | Unit tests cover the handling of GitHub's answers. Not yet tried against a published release |
| Star prompt and **Report a Problem** (§9) | 🟡 | Unit tests cover when the prompt shows and the bug report link. Filling in the bug report form not yet tried on GitHub |
| Signed updates through the Tauri updater | ⬜ | P1 |

## AI assistant (§13)

| ID | Name | Status | Notes |
|---|---|---|---|
| AI-01…06 | Providers, models, Test Connection, model selector, reasoning | ⬜ | P1 |
| AI-07…09 | One conversation per tab, attaching conversations | ⬜ | P1 |
| AI-10 | Ask AI about the terminal selection | ⬜ | P2 |
| AI-11…15 | Tools: `read_terminal`, `run_command`, `send_input`, `web_search`, `fetch_url` | ⬜ | P1 |
| AI-16…18 | Manual approval and bypass modes, approval card, tool call limit | ⬜ | P1 |
| AI-19 | Allow a tool for the rest of a conversation | ⬜ | P2 |
| AI-20, 21 | Context meter, compaction | ⬜ | P1 |
| AI-22 | Automatic compaction | ⬜ | P2 |
| AI-23 | Conversation history | ⬜ | P1 |
| AI-24…26 | History search, Markdown export, edit and resend | ⬜ | P2 |
| AI-27, 28 | Skills: management, import and export, `read_skill` | ⬜ | P2 |
| AI-29…33 | MCP servers: `stdio` and `http`, tools, approvals, lifecycle, `mcpServers` import and export | ⬜ | P2 |

## MVP scope

Out of scope for the MVP: team sharing and multi-user vaults, mobile apps (the architecture leaves room for them, see [§3.4](hatoba-spec.md#34-mobile-readiness)), Telnet, Serial, RDP, VNC, and a sync service hosted by Hatoba.

## Milestones

| Milestone | Scope | Acceptance criteria |
|---|---|---|
| M0 Skeleton | Monorepo, Tauri app shell, tokens extracted from the design, Windows custom title bar (WIN-01), basic layout and routing, IPC binding generation | On Windows 11 and Windows 10, the app starts and shows the design's main pages with fake data, and window dragging, maximizing, and Snap work |
| M1 Terminal | Host data held in memory. Password and key connections, multiple tabs, PTY size sync, fingerprint confirmation | Connects to an OpenSSH test server, vim and htop render correctly, Windows Chinese and Japanese input methods work, 50 MB of output does not stutter, and text is sharp at 150% scaling |
| M2 Local vault | Encryption, SQLite, creating, editing, and deleting hosts, groups, tags, and keys, unlock and auto-lock, recovery code | A search of the local database file finds no plaintext host name, password, or private key |
| M3 Sync | Worker + D1, sync engine, sync wizard and status page, device management | Two devices sync both ways, offline changes merge correctly, conflicts are handled per §6.4, and D1 holds only ciphertext |
| M4 SFTP and polish | SFTP panel, all empty and error states, shortcuts, ssh config import | Every P0 item in §7, §8, and §9 is done |
| M5 Release | Windows installer and code signing, auto-update, README, Worker deployment template | A new user can deploy the Worker and enable sync within 10 minutes by following the README |

## Open questions

1. Whether host online status probing (HOST-10) is on by default. The current implementation turns it on by default, and a setting turns it off.
2. Whether D1 direct mode ships in the first public release.
3. Whether one Worker deployment should serve several users (the current design is single-user).
4. The merge details and UI for Flow C (connecting an existing local vault to an initialized cloud vault).
5. Whether the in-app deployment (§6.7) offers a data location for the new D1 database. D1 accepts a jurisdiction (`eu`, `fedramp`, or `us`) or a location hint only when a database is created, and §6.7 sets neither.
6. How a breaking Worker change (a new `api` number) rolls out while devices on older apps still sync, for example whether one Worker serves both `api` numbers for a while.
7. Whether MCP servers that sign in with OAuth (§13.9) are supported, and where each device keeps their tokens. They cannot sync, because many services rotate the refresh token on each use. Windows Credential Manager holds at most 2,560 bytes per credential, which OAuth tokens often exceed, so a device-local table encrypted with vault_key is the likely place.
8. Whether MCP servers can run on the remote host over the tab's SSH connection, speaking `stdio` over an exec channel. It would reach data where it lives without opening ports, but `run_command` already covers much of the same ground.
