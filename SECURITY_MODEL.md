# Security model for code scans

This file tells a security review of Hatoba, by a person or a scanning tool,
what the system is, what it protects, and which findings matter. The
[security model](docs/hatoba-spec.md#4-security-model-and-encryption) in the
architecture document defines how Hatoba must behave, and the
[security policy](SECURITY.md) defines what counts as a vulnerability; this
file builds on both and does not override them.

## System and trust boundaries

Hatoba is a Tauri 2 desktop SSH and Telnet client with an AI assistant, SFTP,
and end-to-end encrypted sync through a Worker and D1 database in the user's
own Cloudflare account.

1. **WebView and Rust process.** Secrets exist only in the Rust process; the
   WebView never receives the vault key, private keys, or host passwords
   (§3.2, principle 1). CSP `default-src 'self'`, least-privilege
   capabilities, and no shell, fs, or http plugin (SEC-05,
   `apps/desktop/src-tauri/tauri.conf.json`, `capabilities/default.json`).
2. **App and sync Worker or D1.** Only ciphertext crosses it, and the Worker is
   untrusted storage (§4.4). In direct mode the app calls the D1 REST API.
3. **App and remote SSH servers or Telnet devices.** Terminal output, SFTP
   listings, banners, and host keys from the other end are untrusted input.
4. **AI assistant and its inputs.** Screen text, command output, fetched pages,
   MCP tool descriptions and results, and imported skills can carry prompt
   injection (§4.4). Model, search, and MCP providers are recipients of what
   §13.1, AI-47, and AI-54 list.
5. **Other programs and the `hatoba` command endpoint** (HOST-15): a named pipe
   only the user can open on Windows, a Unix socket in the app's data folder
   elsewhere; off by default.
6. **GitHub Actions.** The `release-signing` environment holds the updater
   signing key; `pr-policy.yml` runs on `pull_request_target` but only runs the
   script from `main`.

## Components

Findings are grouped by these components.

| Component | Code |
| --- | --- |
| Vault & Crypto | `crates/hatoba-core/src/` (crypto, vault, recovery, store, backup, model), Windows Hello, Touch ID, and PIN unlock (`apps/desktop/src-tauri/src/platform`, `lock.rs`) |
| Sync Client | `crates/hatoba-core/src/sync/`, `apps/desktop/src-tauri/src/sync.rs`, `deploy.rs` (in-app Worker deployment and its Cloudflare API token) |
| Sync Worker | `workers/sync/` |
| SSH & Terminals | `crates/hatoba-ssh`, `crates/hatoba-telnet`, `crates/hatoba-local`, and `ssh.rs`, `telnet.rs`, `local.rs`, `ssh_agent.rs`, `remote_edit.rs`, `connection.rs` in `apps/desktop/src-tauri/src` |
| AI Assistant | `crates/hatoba-ai`, `crates/hatoba-assistant`, and `apps/desktop/src/features/ai` (the read-only command check in `rules/commandRules.ts`, tool approval in `tools.ts`) |
| Desktop Shell | The rest of `apps/desktop/src` and `apps/desktop/src-tauri` (commands, `update.rs`, `app_proxy.rs`, `logging.rs`), `crates/hatoba-ipc`, `crates/hatoba-cli`, and `cli_access` (HOST-15) |
| CI & Release | `.github/workflows/` |

## Entry points

- The Worker's HTTP API (`/v1/setup`, `/v1/prelogin`, `/v1/login`,
  `/v1/recover`, `/v1/items`, `/v1/vault/password`, `/v1/devices`): reachable
  from the internet. Setup needs `SETUP_TOKEN`, the rest a bearer session;
  setup, login, and recover are rate limited per IP, failing open.
- SSH and Telnet protocol data, from the server or device and from a network
  attacker in the middle (host key checks in `hatoba-ssh/src/hostkey.rs`).
- Terminal output rendered by xterm.js, which can hold any escape sequence.
- The AI assistant's inputs listed under boundary 4.
- The `hatoba` command endpoint, for programs of the same OS user.
- Imports the user chooses: `~/.ssh/config`, PPK files, other clients.
- The update check: `latest.json` from GitHub Releases, verified with the
  minisign key in `tauri.conf.json` (`requireSignedVersion`).
- GitHub events from outside contributors: pull requests and comments.

## Assets

The maintainer treats all of these as critical:

- local vault data and keys: the master password, `master_key`, `enc_key`,
  `vault_key`, the recovery code, SSH private keys, and host passwords;
- the sync Worker's sessions and the D1 data;
- release signing and CI: `release.yml` and the updater signing key
  (`TAURI_SIGNING_PRIVATE_KEY`);
- AI provider API keys and MCP server configuration (environment and header
  values).

Also sensitive: the Cloudflare API token held in memory during an in-app
deployment, AI conversations, terminal content, and the PIN-, Touch ID-, and
Windows Hello-wrapped `vault_key`. The metadata in §4.5 is visible to the
Worker by design.

## Attackers

All of these matter equally:

- someone who steals or exports the Cloudflare account, Worker, or D1;
- a malicious or modified Worker;
- a network man-in-the-middle;
- an anonymous client calling the Worker's API;
- a malicious SSH server or Telnet device;
- the author of text that reaches the AI assistant: a repository, a web page,
  command output, an MCP server;
- an outside contributor to the repository, against CI and release signing.

## Existing controls

The key hierarchy and envelope in §4.1 and §4.2 (Argon2id, HKDF,
`KdfFloor::PRODUCTION`, AES-256-GCM with the item ID as AAD), zeroize on lock
(SEC-01), auto-lock (SEC-02, SEC-03), log redaction (SEC-04), Tauri hardening
(SEC-05), the unlock delay (SEC-06); in the Worker, hashed `auth_key` and
session tokens, body size limits, `Cache-Control: no-store`, and no CORS; the
AI assistant's permission modes and the hard rules of Auto mode's judge
(AI-47); the signed updater; and in the release workflow,
`persist-credentials: false`, minimal `permissions`, and a signing
environment limited to `main`.

## Accepted risks and out of scope

The [threat model](docs/hatoba-spec.md#44-threat-model) lists the outcomes
Hatoba accepts by design, and a finding that only shows one of them is not a
vulnerability. In short:

- an attacker who controls the device while the vault is unlocked;
- whoever holds the recovery code, or a valid full session token, which can
  change the master password;
- a malicious Worker deleting data, rolling it back, or denying service;
- a leaked deployment API token changing or deleting Workers and D1;
- a malicious `stdio` MCP server or imported skill, which is like installing
  software;
- Touch ID on the ad-hoc signed build, and a PIN that works after a restart,
  protecting the vault about as well as the user's OS account;
- AI providers, the judge, and the command suggestion model receiving what the
  spec lists, and bypass mode doing what it is steered to;
- a forgotten master password with a lost recovery code;
- the absence of a safeguard that comparable tools do not have either
  (§3.2, principle 5).

A command, terminal input, or URL that runs without the approval the AI
assistant's mode requires, such as one the read-only check wrongly lets
through, is reported as a normal issue rather than a security advisory, as the
[security policy](SECURITY.md) says. Such findings keep their severity: they
still need fixing.

## Finding tags

- `Component`: one of the components above.
- `CWE`: the finding's CWE ID, for example `CWE-79`.
- `Platform`: `Windows`, `macOS`, `Linux`, `All desktop`, or `Worker`;
  `GitHub Actions` for CI.

## Open questions

- There is no separate infrastructure repository. The Worker's deployment
  configuration is `workers/sync/wrangler.toml`; each user deploys to their
  own Cloudflare account.
