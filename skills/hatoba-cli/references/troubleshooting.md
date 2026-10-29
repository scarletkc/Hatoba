# Troubleshooting the hatoba command

When `hatoba` can't be found or fails, find what you see below and tell the user, in their language, what is wrong and how to fix it. Run the command again once they have. Don't switch to `ssh` or `scp` instead: the hosts and their credentials are in Hatoba. Errors are the `hatoba:` lines on stderr. After one, `hatoba hosts` and `hatoba cp` exit with 1; a command line `hatoba` can't read exits with 2 and adds `Run hatoba --help to see how to use it.`

## The shell can't find `hatoba`

`command not found`, or `not recognized` in PowerShell and cmd.exe. Ask the user to:

1. Install Hatoba if they haven't: https://hatoba.fog.moe/download/
2. Open Hatoba and turn on Settings → General → "Use the hatoba command in terminals", then "Let other programs use saved hosts", which appears below it. Both are off by default, and each computer has its own.

The note under the first switch says where the command went:

- **Windows**: a `bin` folder beside Hatoba, added to the user's `PATH`. A program started before that keeps its old `PATH`, so the user has to restart you, or the editor or terminal you run in. Until then, run `hatoba.exe` from that folder by its full path; the folder is in the user's `PATH` as the system keeps it, which your process hasn't picked up yet.
- **Linux**: `~/.local/bin/hatoba`. When the note says `~/.local/bin` isn't on the `PATH`, the user signs out and back in, or adds it to their shell profile. Until then, run `~/.local/bin/hatoba`.
- **macOS**: `/usr/local/bin/hatoba`, added once the user enters an administrator password when macOS asks.

When the user says turning on the first switch showed an error:

- `<path> is missing. Reinstall Hatoba, then turn this on again.` (Windows): the console program the command is copied from isn't beside Hatoba. The user reinstalls Hatoba.
- `<location> already exists and isn't Hatoba's. Move or rename it, then turn this on again.` (Linux and macOS): another program's file is where the command goes. The user moves or renames it and turns the switch on again.

## `can't reach Hatoba`

- `can't reach Hatoba. Open Hatoba, and turn on Settings > General > "Let other programs use saved hosts".` means Hatoba isn't running or that switch is off. Ask the user to open Hatoba and turn it on. If Hatoba is running with the switch on and the message stays, you are running where Hatoba can't be reached: only programs of the same user on the same computer can reach it, not a container, a remote development environment or a cloud agent. Tell the user, so they can run you on their computer.
- `can't reach Hatoba at <socket path>: <reason>` (macOS and Linux), or `can't reach Hatoba: <reason>` or `can't reach Hatoba safely: <reason>` (Windows): when the reason is "Operation not permitted", "Permission denied" or "Access is denied", your sandbox is usually blocking Hatoba's socket or named pipe. If your tool can run a command outside the sandbox with the user's approval, ask to run `hatoba` that way; otherwise tell the user that the sandbox has to let it through. For any other reason, pass it on and ask the user to quit and reopen Hatoba.

## Hatoba quit

`the connection to Hatoba ended before the command did` or `Hatoba closed the connection before it answered`: Hatoba quit or crashed. Ask the user to reopen it. After `hatoba exec`, check what the command got done on the host before you run it again.

## Hatoba's state

- `Hatoba is locked. Unlock it, then run the command again.`: ask the user to unlock Hatoba.
- `Hatoba locked, which stopped the command.` (or `the copy`, or `the connection`): Hatoba locked while it ran. Once the user unlocks it, check what the command got done on the host before you run it again. When the message adds `It may still be running on the host.`, it may not have stopped.
- `"Let other programs use saved hosts" or the hatoba command was turned off in Hatoba, which stopped the command.` (or `the copy`, or `the connection`): the user turned it off. Ask before you use Hatoba again. The same `It may still be running on the host.` can follow.
- `this hatoba command comes from another version of Hatoba. Quit and reopen Hatoba, which updates it, then run it again.`: ask the user to quit and reopen Hatoba.

## Hosts

- `hatoba hosts --json` prints `[]`: no hosts are saved yet. The user adds them in Hatoba, or imports them with "Import" on the hosts page, which reads `~/.ssh/config` and other SSH clients' sessions.
- `no saved host is named <name>. Run hatoba hosts to see their names.`: compare the user's words with `hatoba hosts --json` again. If no host matches, the machine isn't saved in Hatoba, as SKILL.md describes.
- `<n> saved hosts are named <name>. Name one by its id instead: <ids>.`: tell them apart by `address`, `group` and `tags` in `hatoba hosts --json`, and ask the user when that doesn't settle it.

## Signing in

- `waiting for you to check the host key of <address>:<port> in Hatoba` or `waiting for you to sign in to <user>@<address>:<port> in Hatoba`, where the address may be a jump host's: Hatoba asked in its window. If your call ended before the user answered, the sign-in was abandoned. Tell the user what Hatoba asks, and run the command again with a long tool timeout so they can answer.
- With `--no-prompt`, the same cases fail instead: `<host> has no saved password. …`, `the host key of <host>, or of a jump host on the way, isn't trusted yet. …`, `<host> asks for a code or a password that isn't saved. …`. Run the command again without `--no-prompt` while the user is there to answer.
- `the host key of <host>, or of a jump host on the way, has changed. …` comes with `--no-prompt`; without it, Hatoba asks in its window. Tell the user either way: a reinstalled server changes its key, and so does someone intercepting the connection, and the user checks it in Hatoba.
- `can't connect to <host>: jump host <jump> needs a saved password or key`: the jump host's `auth` is `ask`, and Hatoba signs in through a jump host only with what it has saved. Ask the user to save that host's password or key in Hatoba.
- `can't connect to <host>: <reason>` or `the connection to <host> was lost: <reason>`: pass the reason on. `hatoba <name>` opens a terminal on the host in Hatoba, where the user sees the same failure, and https://hatoba.fog.moe/docs/troubleshooting-connections/ covers the common ones.

## Running and copying

- `<host> runs commands with cmd.exe or PowerShell, so --cwd cannot start one in another directory. …` (Windows hosts) or `this host has no sh to start the command in workdir: leave out workdir and change the directory in the command` (hosts without sh, such as routers and some Windows servers): leave out `--cwd` and change the directory in the command.
- `the command ended without an exit status, as when a signal ends it`: the host closed the command without reporting how it ended. The connection dropped or the shell running the command was killed, or, on a Windows host or a host without sh, a signal ended the command. Check what it got done before you run it again.
- `can't copy files with <host>: <reason>`: the host didn't start SFTP, which `hatoba cp` copies over; some routers and minimal servers don't have it. Tell the user.
- `<path> is a folder. hatoba cp copies one file at a time.` (a local folder) or `download <path>: is a directory` (a remote one): copy an archive of the folder instead.
