---
name: hatoba-cli
description: "Work on the SSH hosts saved in Hatoba with the `hatoba` command: list them, run commands on them, and copy files to and from them, signed in with the credentials Hatoba keeps. Use it whenever a task needs a remote server, a host, ssh or scp: the servers the user names are usually saved in Hatoba. Not for creating SSH keys or editing ssh config on this computer."
---

# Hatoba hosts

Hatoba is a desktop SSH client. Its `hatoba` command lets other programs use the hosts saved in it: the running app signs in with each host's saved password or key, jump host and proxy, and returns only the output or the file. You never handle the credentials, and the hosts are usually not in `~/.ssh/config`, so `ssh <name>` fails where `hatoba exec <name>` works. When the user is fixing their own ssh setup on this computer, such as their keys or `~/.ssh/config`, help with that directly.

```sh
hatoba hosts --json                                        # list the saved hosts
hatoba exec [--cwd <dir>] [--no-prompt] <host> <command>   # run a command on one
hatoba cp [--no-prompt] <source> <target>                  # copy a file to or from one
hatoba <name>                                              # open a terminal on it in Hatoba, for the user
```

When the shell can't find `hatoba`, or `hatoba` fails before it reaches the host, don't fall back to `ssh`: find the message in [references/troubleshooting.md](references/troubleshooting.md) and tell the user how to fix it.

## Pick the host

Each entry of `hatoba hosts --json` has `id`, `name`, `address`, `port`, `username`, `auth` (`password`, `key`, `agent`, or `ask` for a password that isn't saved, which Hatoba asks for), `group`, `tags`, `jump` (the name of the saved host Hatoba signs in through) and, when the user wrote some, `ai_notes`.

- `<host>` is a saved host's `id` or its exact `name`, in any case (quote a name with spaces); an address is not.
- A host whose name is exactly the user's word is that host: "web" is `web`, not `web-2`. Otherwise match their words against the names, addresses, groups and tags, and ask only when they mean one machine and several hosts fit. For a group or a tag, such as "all the prod servers", work on every host in it.
- `ai_notes` are the user's own notes for assistants about that host, such as what runs there or what to leave alone. Follow them as the user's instructions.
- A machine that isn't listed isn't saved in Hatoba. Tell the user, and offer to add it in Hatoba or to use `ssh` for it.

## Run a command

- The command runs with the host's login shell, as `ssh <host> '<command>'` would, so quote it the same way.
- It exits with the command's exit status. 255 with a `hatoba:` line on stderr is Hatoba's own error, which may come after the command started, as when Hatoba locks or the connection drops: find the line in [references/troubleshooting.md](references/troubleshooting.md) before you run the command again.
- `--cwd` starts the command in a directory that is absolute or from the home directory. Windows hosts and hosts without sh refuse it: change the directory in the command there.
- There is no terminal and stdin is empty, so interactive programs and a `sudo` that asks for a password don't work; hand such a step to the user (below).
- When your tool's timeout ends `hatoba exec`, or you stop it, Hatoba prints nothing more. On hosts with sh it stops the command and everything the command started; on Windows hosts and hosts without sh the command may keep running. Either way, check what it got done before you run it again. On hosts with sh, a command that ends by itself leaves what it put in the background running, so start work that may outlast your timeout in the background with its output in a log, and read the log in later calls.

## Copy a file

One side is `<host>:<path>` and the other a local path: `hatoba cp dist/app.tar.gz web:/tmp/` uploads, and `hatoba cp web:logs/error.log ./` downloads.

- A remote path is from the home directory unless it starts with `/`; `~` and `~/…` work too. A target that is a folder gets the file under its own name.
- Absolute local paths, Windows drive paths included, are always local; start a relative one that contains `:` with `./`.
- A copy replaces a file that is already at the target, on either side. An uploaded file belongs to the login user with default permissions. A copy that fails or is stopped, as by a timeout, leaves the old file as it was.
- `hatoba cp` copies one file, so archive a folder first.

## When Hatoba needs the user

- Hatoba asks in its window, which comes to the front, for a host key it doesn't trust yet (the host's or a jump host's), for the password of a host whose `auth` is `ask`, and for a code or password a server wants that isn't saved. It waits up to 10 minutes, and your call blocks meanwhile: you see its `hatoba: waiting for you to … in Hatoba` line only when the call returns, and ending the call abandons that sign-in.
- So before the first command to a host whose `auth` is `ask`, tell the user that Hatoba will ask for the password in its window. Give the first call to any host a long tool timeout, since a host key Hatoba doesn't trust yet can't be known in advance.
- A jump host whose `auth` is `ask` is never asked for: calls through it fail until the user saves its password or key in Hatoba, so ask for that first.
- Once signed in, Hatoba keeps the connection until it has gone 10 minutes without a command or copy, or Hatoba locks, so later calls to that host don't ask again. An accepted host key stays trusted; an asked password is never saved.
- `--no-prompt` fails at once instead of asking, which suits a script nobody watches.
- `hatoba <name>` opens a terminal tab on that host in Hatoba for the user, for a step that needs them, such as a `sudo` password or an interactive program. It takes the host's name, not its id, which it would read as an address. It prints nothing and exits 0, and so does any first word other than `hosts`, `exec`, `cp`, `help`, `-h`, `--help` or `--version`: a misspelled `hatoba host --json` only opens Hatoba.
