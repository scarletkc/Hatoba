<div align="center">

<img src="apps/desktop/src-tauri/app-icon.svg" alt="Hatoba logo" width="128" />

# Hatoba

**An open-source desktop SSH client with end-to-end encrypted sync through your own Cloudflare account.**

[![CI](https://img.shields.io/github/actions/workflow/status/scarletkc/Hatoba/ci.yml?branch=main&label=CI&logo=githubactions&logoColor=white)](https://github.com/scarletkc/Hatoba/actions/workflows/ci.yml)
[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://v2.tauri.app/)
[![Sync](https://img.shields.io/badge/sync-Cloudflare%20Workers%20%2B%20D1-F38020?logo=cloudflare&logoColor=white)](workers/sync/README.md)
[![License](https://img.shields.io/badge/license-MIT-blue)](LICENSE)

</div>

Hatoba (波止場, "wharf") keeps your hosts, keys, and terminal sessions in one
place. Its sync backend is a Worker and a D1 database that you deploy to your
own Cloudflare account, and every item is encrypted on your device before it
leaves. There is no Hatoba server, and nobody else holds your data.

- **No official server.** The sync backend runs in your Cloudflare account,
  so Hatoba never sees your data.
- **End-to-end encrypted.** A leaked Worker, D1 database, or even the whole
  Cloudflare account exposes only ciphertext.
- **Local first.** Everything works offline, with or without sync.
- **Windows first.** A custom title bar with Snap Layouts, Mica, Segoe UI and
  Cascadia fonts, and Windows input methods. macOS and Linux follow.

## Features

- **Hosts**: groups, tags, favorites, fuzzy search, recent connections, online
  status, `~/.ssh/config` import, and ProxyJump.
- **Terminal**: tabs, 256 colors and truecolor, CJK wide characters and input
  methods, confirmation before multi-line paste, search, and clickable links.
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

## Build from source

You need Rust stable, Node.js 22 or later, and pnpm. Windows also needs
WebView2, which Windows 11 includes; Linux needs the Tauri system dependencies
such as `libwebkit2gtk-4.1-dev`.

```sh
pnpm install
pnpm tauri dev
```

On Windows, `pnpm tauri build` produces the NSIS installer. The
[development guide](docs/development.md) covers the browser-only frontend,
tests, and generated TypeScript bindings.

## Set up sync

Sync is optional. To deploy the Worker, run these commands from `workers/sync`
with Node.js 22 or later:

```sh
npm install
npx wrangler login
npx wrangler d1 create hatoba                       # put the printed database_id in wrangler.toml
npx wrangler d1 migrations apply hatoba --remote
openssl rand -base64 32                             # generate a setup token and keep it
npx wrangler secret put SETUP_TOKEN                 # paste the setup token
npx wrangler deploy
```

In Hatoba, open **Cloud Sync** in the sidebar, choose **Deploy a Worker**,
enter the Worker URL and the setup token, and select **Test Connection**.
Other devices choose **Restore from Cloud** on first launch and need only the
Worker URL and the master password.
The [deployment guide](workers/sync/README.md) explains each step and covers
upgrades, resets, and backups.

## Security

Keys are derived from your master password with Argon2id, and every item is
encrypted with AES-256-GCM before it is stored or synced. The Worker stores
ciphertext and the hashes it needs to sign you in, never your master password
or any plaintext. If you forget the master
password and lose the recovery code, your data cannot be recovered. The
[security model](docs/hatoba-spec.md#4-安全模型与加密设计) describes the key
hierarchy and the threat model.

## Documentation

The documentation below is written in Chinese, except the end-to-end test
guide.

- [Architecture and requirements](docs/hatoba-spec.md): architecture, security model, data formats, the sync protocol and Worker API, and requirements
- [Implementation status](docs/status.md): progress on each requirement, milestones, and open questions
- [Development](docs/development.md): building, running, and testing
- [Deploying the sync Worker](workers/sync/README.md): deployment, upgrades, resets, and backups
- [End-to-end smoke test](apps/desktop/e2e/README.md): driving the real app against a real OpenSSH server
- [Design](docs/design/README.md): design files, porting conventions, and deviations from the design

## License

Hatoba is licensed under [MIT](LICENSE).
