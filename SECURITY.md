# Security policy

Report a vulnerability through
[private vulnerability reporting](https://github.com/scarletkc/Hatoba/security/advisories/new).
Do not open a public issue or pull request for it.

## What counts as a vulnerability

A vulnerability is anything that exposes passwords, keys, or other vault data
beyond what the
[security model](docs/hatoba-spec.md#4-security-model-and-encryption) allows,
or that gets around its encryption, the sync Worker's sessions, or release
signing. This covers the vault and sync in the desktop app, the sync Worker,
and the workflows that build and publish releases. For example:

- vault data, keys, or tokens written to disk, logs, or the network
  unencrypted;
- the sync Worker accepting a request without a valid session, or the app
  accepting KDF parameters below its minimums.

The AI assistant's approval modes are product behavior. A command, terminal
input, or URL that runs without the approval you expected, for example one the
read-only command check lets through in manual approval mode, is a bug: report
it as a normal issue.

The [threat model](docs/hatoba-spec.md#44-threat-model) lists the outcomes
Hatoba accepts by design. A report that only shows one of them is not a
vulnerability, for example an attacker who controls the device while the vault
is unlocked, or who holds the recovery code.

## What to include

- the Hatoba version or the commit you built from, and the operating system;
- the steps that reproduce the problem, or a proof of concept;
- what an attacker gains, and what they need first.

Use test data only. Never include a real password, private key, recovery code,
or token, and test only against your own vault and your own Worker.

## Supported versions

Fixes go into the next release, and earlier releases do not get them. A fix to
the sync Worker reaches you when you
[upgrade your Worker](workers/sync/README.md#upgrade).

The advisory is published after the fixed release, and credits the reporter
unless they ask not to be named.
