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
[![Mentioned in Awesome Rust](https://img.shields.io/badge/mentioned%20in-Awesome%20Rust-FC60A8?logo=awesomelists&logoColor=white)](https://github.com/rust-unofficial/awesome-rust#applications)

https://github.com/user-attachments/assets/cd33e989-36e3-4e17-bc7f-74d472c96078

</div>

Hatoba keeps your hosts, keys, and terminal sessions in one place, syncs them
between your devices, and puts an AI assistant beside each terminal.

- **AI in every terminal.** The assistant reads the screen, runs commands, and
  edits files on the host, and you choose what it may do without asking.
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

- **AI assistant**: reads the screen, runs commands, edits files on the host or
  your own computer, and searches the web. You approve its actions, or let Auto
  mode approve the safe ones. It uses your own OpenAI-compatible or Anthropic
  API key and supports skills and MCP servers.
- **Hosts**: groups, tags, fuzzy search, `~/.ssh/config` import, ProxyJump, and
  proxies. Type `ssh user@host` in the search box to connect without saving a
  host.
- **Coding agents**: Claude Code, Codex, and scripts run commands and copy
  files on your saved hosts with the `hatoba` command, while passwords and keys
  stay in Hatoba. The [hatoba-cli skill](skills/hatoba-cli/SKILL.md) shows
  agents how.
- **Resource usage**: the CPU, memory, disks, and network of every saved host,
  read over SSH with nothing to install on the servers.
- **Terminal**: tabs, truecolor, CJK input methods, search, and clickable
  links, with local shells such as PowerShell, WSL, or your login shell beside
  the SSH tabs.
- **SFTP**: a file panel beside the terminal with drag-and-drop upload and
  download, and remote files edited in your own editor, uploaded on every save.
- **Port forwarding**: local, remote, and SOCKS forwards that can start with
  the connection or run without a terminal.
- **Authentication**: passwords, Ed25519, ECDSA, and RSA keys, ssh-agent, and
  2FA, with OpenSSH, PEM, and PuTTY key import and key generation.
- **Security**: host key verification, a master password with a recovery code,
  and auto-lock.
- **Sync**: incremental sync between your devices, with conflicts you can
  review item by item and a device list with revocation.
- **Languages**: English, Simplified Chinese, Traditional Chinese, and
  Japanese, following the system language unless you pick one in Settings.

The [documentation](https://hatoba.fog.moe/docs/) covers each one in detail.

## Download

[![Download for Windows](https://img.shields.io/badge/Download-Windows%20x64-0078D4?style=for-the-badge&logo=data:image/svg%2bxml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHZpZXdCb3g9IjAgMCAyNCAyNCIgZmlsbD0id2hpdGUiPjxwYXRoIGQ9Ik0wIDBoMTF2MTFIMHpNMTMgMGgxMXYxMUgxM3pNMCAxM2gxMXYxMUgxM3oiLz48L3N2Zz4=)](https://hatoba.fog.moe/download/#windows)
[![Download for Linux](https://img.shields.io/badge/Download-Linux%20x64-FCC624?style=for-the-badge&logo=linux&logoColor=black)](https://hatoba.fog.moe/download/#linux)
[![Download for macOS](https://img.shields.io/badge/Download-macOS%20Apple%20Silicon%20%28experimental%29-000000?style=for-the-badge&logo=apple&logoColor=white)](https://hatoba.fog.moe/download/#macos)

The [download page](https://hatoba.fog.moe/download/) offers the newest
release for your system, with its checksums and install steps, and
[Releases](https://github.com/scarletkc/Hatoba/releases) has every version.

Package managers can install and update it too. On Windows, from the Extras
bucket of [Scoop](https://scoop.sh):

```powershell
scoop bucket add extras
scoop install extras/hatoba
```

On Arch Linux and distributions based on it, install
[`hatoba-bin`](https://aur.archlinux.org/packages/hatoba-bin) from the AUR with
an AUR helper such as yay:

```sh
yay -S hatoba-bin
```

The maintainer uses Windows, so the Linux and macOS builds get little testing on
real machines. If something doesn't work on them, please
[open an issue](https://github.com/scarletkc/Hatoba/issues). To build Hatoba
yourself, follow the [development guide](docs/development.md#build).

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
- [Deploy the sync Worker](workers/sync/README.md): deployment, upgrades, resets, and backups
- [Architecture and requirements](docs/hatoba-spec.md): architecture, security model, data formats, the sync protocol and Worker API, and requirements
- [Development guide](docs/development.md): building, running, and testing
- [Contributing](CONTRIBUTING.md): issues, branches, commits, and pull requests
- [Security policy](SECURITY.md): reporting a vulnerability privately

## Sponsors

Thanks to everyone who has sponsored Hatoba's author.

<a href="https://github.com/kleedaisuki"><img src="https://github.com/kleedaisuki.png?size=96" width="48" height="48" alt="kleedaisuki" /></a>
<a href="https://github.com/kamiiroawase"><img src="https://github.com/kamiiroawase.png?size=96" width="48" height="48" alt="kamiiroawase" /></a>

[Sponsor Hatoba on GitHub](https://github.com/sponsors/scarletkc)

## License

Hatoba is licensed under [MIT](LICENSE).
