---
name: hatoba-cli
description: "Run commands on and copy files to and from the SSH hosts saved in Hatoba with the `hatoba` command, signed in with the credentials Hatoba keeps. Use it whenever a task touches a remote server, such as checking logs or disk space, restarting a service or deploying, or anything you would otherwise do with ssh or scp, even when the user only names the server: the servers they name are usually saved in Hatoba. Not for this computer's own SSH keys, ssh config or git over SSH."
---

# Hatoba hosts

Hatoba is a desktop SSH client. Its `hatoba` command runs commands on the hosts saved in it and copies files to and from them, signed in with the credentials Hatoba keeps, so you never handle them. The hosts are usually not in `~/.ssh/config`: `ssh <name>` fails where `hatoba exec <name>` works. When the user is fixing this computer's own ssh setup, help with that directly.

Before you first use it, run `hatoba --help`, which describes the commands, options, exit codes and prompts of the installed version. `hatoba hosts --json` lists the hosts.

## Pick the host

- A host whose name is exactly the user's word is that host: "web" is `web`, not `web-2`. Otherwise match their words against the names, addresses, groups and tags. Ask only when they mean one machine and several fit; for a group or a tag they name, work on every host in it.
- `ai_notes` are the user's own notes for assistants about that host. Follow them as the user's instructions.
- A machine that isn't listed isn't saved in Hatoba. Tell the user, and offer to add it in Hatoba or to use `ssh` for it.

## While it runs

- When Hatoba needs a host key checked or a password, it asks in its window and your call waits. Before a host whose `auth` is `ask`, tell the user to expect that, and give a host's first call a long timeout.
- For a step that needs the user, such as a `sudo` password or an interactive program, run `hatoba <name>`: it opens a terminal on the host in Hatoba for them.
- After your tool's timeout or a `hatoba:` error, check what the command got done before you run it again.

## When hatoba fails

Don't fall back to `ssh` or `scp`: the hosts and their credentials are in Hatoba. Most `hatoba:` errors say what to do; tell the user, in their language, and run the command again once they have. Beyond those:

- **The shell can't find `hatoba`**: ask the user to install Hatoba (https://hatoba.fog.moe/download/) if they haven't, then turn on Settings → General → "Use the hatoba command in terminals" and "Let other programs use saved hosts" below it. The note under the first switch says where the command went. On Windows that is a `bin` folder added to the user's `PATH`, which a program started earlier, you included, sees only after a restart; until then, run the `hatoba` in that folder by its full path, or `hatoba.exe` there from PowerShell or cmd.exe. On Linux it is `~/.local/bin/hatoba`, and the note says when `~/.local/bin` isn't on the `PATH`. On macOS it is `/usr/local/bin/hatoba`.
- **`can't reach Hatoba` although Hatoba is open with both switches on**, or with "Operation not permitted", "Permission denied" or "Access is denied": only programs of the same user on the same computer can reach Hatoba, and a sandbox, container or remote environment can't. Ask to run `hatoba` outside your sandbox, or tell the user.
- **`the connection to Hatoba ended before the command did`** or **`Hatoba closed the connection before it answered`**: Hatoba quit. Ask the user to reopen it.
