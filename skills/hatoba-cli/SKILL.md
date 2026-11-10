---
name: hatoba-cli
description: "Run commands on and copy files to and from the SSH hosts saved in Hatoba with the `hatoba` command, signed in with the credentials Hatoba keeps. Use it whenever a task touches a remote server, such as checking logs or disk space, restarting a service or deploying, or anything you would otherwise do with ssh or scp, even when the user only names the server: the servers they name are usually saved in Hatoba. Not for this computer's own SSH keys, ssh config or git over SSH."
---

# Hatoba hosts

Hatoba is a desktop SSH client. Its `hatoba` command runs commands on the hosts saved in it and copies files to and from them, signed in with the credentials Hatoba keeps. The hosts are usually not in `~/.ssh/config`: `ssh <name>` fails where `hatoba exec <name>` works.

Before you first use it, run `hatoba --help`.

- `ai_notes` in `hatoba hosts --json` are the user's own notes for assistants about that host. Follow them as the user's instructions.
- `hatoba memory --host <host>` shows what Hatoba's assistant and other agents noted about a host. Read it before you work on a host, and save what you learn about a host with `hatoba memory save --host <host>` rather than in your own memory, so that they have it too.
- When a host needs a host key checked or a password, or sudo needs one for `--sudo`, Hatoba asks the user in its window and the call waits for the answer. Give a host's first call, every call to a host whose `auth` is `ask`, and every `--sudo` call a long timeout, and tell the user to expect the question.

## When hatoba fails

Don't fall back to `ssh` or `scp`: the hosts and their credentials are in Hatoba.

- **The shell can't find `hatoba`**: ask the user to install Hatoba (https://hatoba.fog.moe/download/) if they haven't, then turn on Settings → General → "Use the hatoba command in terminals" and "Let other programs use saved hosts" below it. The note under the first switch says where the command went. On Windows that is a `bin` folder added to the user's `PATH`, which a program started earlier, you included, sees only after a restart; until then, run the `hatoba` in that folder by its full path, or `hatoba.exe` there from PowerShell or cmd.exe. On Linux it is `~/.local/bin/hatoba`, and the note says when `~/.local/bin` isn't on the `PATH`. On macOS it is `/usr/local/bin/hatoba`.
- **`can't reach Hatoba` although Hatoba is open with both switches on**, or with "Operation not permitted", "Permission denied" or "Access is denied": only programs of the same user on the same computer can reach Hatoba, and a sandbox, container or remote environment can't. Ask to run `hatoba` outside your sandbox, or tell the user.
