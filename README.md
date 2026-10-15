<div align="center">

<img src="apps/desktop/src-tauri/app-icon.svg" alt="Hatoba logo" width="128" />

# Hatoba

**Open-source desktop SSH client with an AI assistant and end-to-end encrypted sync through your own Cloudflare account**

[Website](https://hatoba.fog.moe) | [Download](https://hatoba.fog.moe/download/) | [Documentation](https://hatoba.fog.moe/docs/)

[![Release](https://img.shields.io/github/v/release/scarletkc/Hatoba?include_prereleases&sort=semver&label=release&logo=github)](https://github.com/scarletkc/Hatoba/releases)
[![CI](https://img.shields.io/github/actions/workflow/status/scarletkc/Hatoba/ci.yml?branch=main&label=CI&logo=githubactions&logoColor=white)](https://github.com/scarletkc/Hatoba/actions/workflows/ci.yml)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://v2.tauri.app/)
[![Sync](https://img.shields.io/badge/sync-Cloudflare%20Workers%20%2B%20D1-F38020?logo=cloudflare&logoColor=white)](workers/sync/README.md)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

<img src="docs/media/hatoba-demo.webp" alt="The AI assistant finds and fixes an nginx 502 on a server, then Hatoba deploys its sync Worker to Cloudflare" width="880" />

</div>

Hatoba keeps your hosts, keys, and terminal sessions in one place, syncs them
between your devices, and puts an AI assistant beside each terminal. The sync
backend is a Worker and a D1 database that you deploy to your own Cloudflare
account.

- **No Hatoba server.** Sync runs entirely in your Cloudflare account, so
  nobody else holds your data.
- **End-to-end encrypted.** Everything is encrypted on your device before it
  leaves, so a leaked Worker, D1 database, or even the whole Cloudflare account
  exposes only ciphertext.
- **Local first.** Everything works offline, and sync is optional.
- **Windows first.** A custom title bar with Snap Layouts, Mica, Segoe UI and
  Cascadia fonts, and Windows input methods. Each release also has Linux
  packages and an experimental macOS build.

## Features

- **Hosts**: groups, tags, favorites, fuzzy search, recent connections, online
  status, `~/.ssh/config` import, ProxyJump, and SOCKS5 and HTTP proxies. Type
  `ssh user@host` in the search box to connect without saving a host, or run
  `hatoba user@host` in a terminal once the `hatoba` command is on in Settings.
- **Dashboard**: the online status, CPU, memory, disks, network, load, and
  uptime of every saved host on one page, as cards or a table, read over SSH
  without installing anything on the servers.
- **Terminal**: tabs, 256 colors and truecolor, CJK wide characters and input
  methods, confirmation before multi-line paste, search, clickable links, and
  the CPU, memory, and network usage of Linux servers in the status bar.
- **Local terminal**: tabs for the shells of your own computer, such as
  PowerShell, Command Prompt, Git Bash, and WSL distributions on Windows, or
  your login shell on macOS and Linux, beside the SSH tabs.
- **Authentication**: passwords, Ed25519, ECDSA, and RSA keys with or without
  a passphrase, ask on every connection, ssh-agent, and keyboard-interactive
  (2FA).
- **Security**: host key confirmation on first connect, a blocked connection
  when the key changes, a master password with a recovery code, and auto-lock
  on idle, on sleep, or on demand.
- **SFTP**: a file panel beside the terminal with drag-and-drop upload,
  download with progress and cancel, rename, delete, and new folder.
- **Key vault**: import OpenSSH, PEM, and PuTTY `.ppk` keys, generate Ed25519
  or RSA 4096 keys, and copy or deploy public keys.
- **Port forwarding**: local forwards that can start with the connection.
- **Cloud sync**: through your Worker or directly to D1, with incremental
  sync, automatic conflict resolution you can review item by item, and a
  device list with revocation.
- **AI assistant**: a panel beside each terminal that reads the screen, runs
  commands, types into the shell, reads, edits, and searches files on the host,
  or on your own computer in a local terminal, and can search the web and read
  pages. You approve its actions, a model you
  choose approves the safe ones in Auto mode, or bypass mode skips approval. It
  uses your own OpenAI-compatible or Anthropic API key, supports skills and MCP
  servers, and takes terminal selections, connection errors, long pastes, and
  text files as attachments. Type `/` for commands, skills, and MCP prompts, and
  `@` to attach files and folders on the host or MCP resources. Conversations
  sync with the rest of the vault.

## Download

[![Download for Windows](https://img.shields.io/badge/Download-Windows%20x64-0078D4?style=for-the-badge&logo=data:image/svg%2bxml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCIgZmlsbD0id2hpdGUiPjxwYXRoIGQ9Ik0wIDBoMTF2MTFIMHpNMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgxM3oiLz48L3N2Zz4=)](https://hatoba.fog.moe/download/#windows)
[![Download for Linux](https://img.shields.io/badge/Download-Linux%20x64-FCC624?style=for-the-badge&logo=linux&logoColor=black)](https://hatoba.fog.moe/download/#linux)
[![Download for macOS](https://img.shields.io/badge/Download-macOS%20Apple%20Silicon%20%28experimental%29-000000?style=for-the-badge&logo=apple&logoColor=white)](https://hatoba.fog.moe/download/#macos)

The [download page](https://hatoba.fog.moe/download/) offers the newest
release for your system, with its checksums and install steps, and
[Releases](https://github.com/scarletkc/Hatoba/releases) has every version. The
maintainer uses Windows, so the Linux and macOS builds get little testing on
real machines. If something doesn't work on them, please
[open an issue](https://github.com/scarletkc/Hatoba/issues).

## Security

Keys are derived from your master password with Argon2id, and every item is
encrypted with AES-256-GCM before it is stored or synced. The Worker stores
ciphertext and the hashes it needs to sign you in, never your master password
or any plaintext. If you forget the master password and lose the recovery code,
your data cannot be recovered. The
[security model](docs/hatoba-spec.md#4-security-model-and-encryption) describes
the key hierarchy and the threat model.

The AI assistant sends the model provider your messages, their attachments,
the terminal's screen as each message is sent (unless you turn that off), and
what its tools read. Host addresses, passwords, and keys reach the provider
only when they appear on the screen, in a tool result, or in an attachment you
send.

## Build from source

You need Rust stable, Node.js 22 or later, and pnpm. Windows also needs
WebView2, which Windows 11 includes, macOS needs the Xcode Command Line Tools,
and Linux needs the Tauri system dependencies such as `libwebkit2gtk-4.1-dev`.

```sh
pnpm install
pnpm tauri dev
```

`pnpm tauri build --no-sign` produces the NSIS installer on Windows, and the deb
package and the AppImage on Linux, without the update signatures that only
release builds carry. On macOS, don't pass `--no-sign`: it also drops the
ad-hoc code signature, and Apple Silicon Macs report an app without one as
damaged once it is downloaded or copied from another Mac. Turn off the update
artifacts instead, and the build produces the app bundle and the DMG:

```sh
pnpm tauri build --config '{"bundle":{"createUpdaterArtifacts":false}}'
```

The [development guide](docs/development.md) covers the browser-only frontend,
tests, and the generated TypeScript bindings.

## Set up sync

Sync is optional. It runs on a Worker and a D1 database that you deploy once to
your Cloudflare account, and the free plan usually covers personal use. Hatoba
can deploy them for you with a Cloudflare API token; otherwise deploy them with
one click from the browser or with wrangler, then connect the first device with
the Worker URL and a setup token you generate during deployment. Other devices
need only the Worker URL and the master password.

[Deploy the sync Worker](workers/sync/README.md) walks through each way and
connecting Hatoba, and covers upgrades, resets, and backups.

## Documentation

- [User documentation](https://hatoba.fog.moe/docs/): hosts, the terminal, SFTP, keys and the vault, sync, the AI assistant, and troubleshooting
- [Architecture and requirements](docs/hatoba-spec.md): architecture, security model, data formats, the sync protocol and Worker API, and requirements
- [Implementation status](docs/status.md): progress on each requirement, milestones, and open questions
- [Development guide](docs/development.md): building, running, and testing
- [Release Hatoba](docs/releasing.md): versions, release notes, and publishing
- [Deploy the sync Worker](workers/sync/README.md): deployment, upgrades, resets, and backups
- [End-to-end smoke test](apps/desktop/e2e/README.md): driving the real app against a real OpenSSH server
- [Design](docs/design/README.md): design files, porting conventions, and deviations from the design
- [Contributing](CONTRIBUTING.md): issues, branches, commits, and pull requests
- [Security policy](SECURITY.md): reporting a vulnerability privately

## License

Hatoba is licensed under [MIT](LICENSE).
