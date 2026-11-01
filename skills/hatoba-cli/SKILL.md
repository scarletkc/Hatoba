---
name: hatoba-cli
description: "Run commands on and copy files to and from the SSH hosts saved in Hatoba with the `hatoba` command, signed in with the credentials Hatoba keeps. Use it whenever a task touches a remote server, such as checking logs or disk space, restarting a service or deploying, or anything you would otherwise do with ssh or scp, even when the user only names the server: the servers they name are usually saved in Hatoba. Not for this computer's own SSH keys, ssh config or git over SSH."
---

# Hatoba hosts

Hatoba is a desktop SSH client. Its `hatoba` command runs commands on the hosts saved in it and copies files to and from them, signed in with the credentials Hatoba keeps, so you never handle them. The hosts are usually not in `~/.ssh/config`: `ssh <name>` fails where `hatoba exec <name>` works. When the user is fixing this computer's own ssh setup, help with that directly.

Before you first use it, run `hatoba --help`, which describes the commands, options, exit codes and prompts of the installed version. `hatoba hosts --json` lists the SSH hosts.

## Pick the host

- A host whose name is exactly the user's word is that host: "web" is `web`, not `web-2`. Otherwise match their words against the names, addresses, groups and tags. Ask only when they mean one machine and several fit; for a group or a tag they name, work on every host in it.
- `ai_notes` are the user's own notes for assistants about that host. Follow them as the user's instructions.
- A machine that isn't listed isn't saved in Hatoba as an SSH host. Tell the user, and offer to add it in Hatoba or to use `ssh` for it.

## While it runs

- When Hatoba needs a host key checked or a password, it asks in its window and your call waits. Before a host whose `auth` is `ask`, tell the user to expect that, and give a host's first call a long timeout.
- A server can hold the login until the user acts on a message it sends, which `hatoba` prints on stderr, as Tailscale SSH does until they sign in at the link it prints. Pass the message on to the user; the call waits until the server lets the login through.
- For a command that needs root, run `hatoba exec --sudo <host> <command>`, and `hatoba cp --sudo` for a file only root can read or write. `exec` has no terminal, so a `sudo` inside the command cannot ask for its password; with `--sudo`, Hatoba asks the user in its window when sudo wants one, so give the call a long timeout.
- `exec` passes a stdin from a pipe, a file or a heredoc to the command, so a script or a file's content needs no quoting: `hatoba exec <host> 'sh -s' < script.sh`, or `hatoba exec --sudo <host> 'tee /etc/app.conf >/dev/null' < app.conf`. `hatoba cp --mode 600` sets the permissions of the file it copies to the host.
- For a step that needs the user, such as an interactive program, run `hatoba <name>`: it opens a terminal on the host in Hatoba for them.
- After your tool's timeout or a `hatoba:` error, check what the command got done before you run it again.

## When hatoba fails

Don't fall back to `ssh` or `scp`: the hosts and their credentials are in Hatoba. Most `hatoba:` errors say what to do; tell the user, in their language, and run the command again once they have. Beyond those:

- **The shell can't find `hatoba`**: ask the user to install Hatoba (https://hatoba.fog.moe/download/) if they haven't, then turn on Settings → General → "Use the hatoba command in terminals" and "Let other programs use saved hosts" below it. The note under the first switch says where the command went. On Windows that is a `bin` folder added to the user's `PATH`, which a program started earlier, you included, sees only after a restart; until then, run the `hatoba` in that folder by its full path, or `hatoba.exe` there from PowerShell or cmd.exe. On Linux it is `~/.local/bin/hatoba`, and the note says when `~/.local/bin` isn't on the `PATH`. On macOS it is `/usr/local/bin/hatoba`.
- **`can't reach Hatoba` although Hatoba is open with both switches on**, or with "Operation not permitted", "Permission denied" or "Access is denied": only programs of the same user on the same computer can reach Hatoba, and a sandbox, container or remote environment can't. Ask to run `hatoba` outside your sandbox, or tell the user.
- **`the connection to Hatoba ended before the command did`** or **`Hatoba closed the connection before it answered`**: Hatoba quit. Ask the user to reopen it.
