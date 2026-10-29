# Troubleshooting the hatoba command

When `hatoba` can't be found or fails, find the case below and tell the user, in their language, what is wrong and how to fix it. Run the command again once they have. Don't switch to `ssh` or `scp` instead: the hosts and their credentials are in Hatoba. Errors are the `hatoba:` lines on stderr; `hatoba exec` then exits with 255, `hatoba hosts` and `hatoba cp` with 1.

Hatoba shows these labels in the language the user picked:

| English | Simplified Chinese | Traditional Chinese | Japanese |
|---|---|---|---|
| Settings → General | 设置 → 通用 | 設定 → 一般 | 設定 → 一般 |
| Use the hatoba command in terminals | 在终端中使用 hatoba 命令 | 在終端機中使用 hatoba 命令 | ターミナルで hatoba コマンドを使う |
| Let other programs use saved hosts | 允许其他程序使用已保存的主机 | 允許其他程式使用已儲存的主機 | 他のプログラムに保存済みのホストを使わせる |

## The shell can't find `hatoba`

`command not found`, or `not recognized` in PowerShell and cmd.exe. Ask the user to:

1. Install Hatoba if they haven't: https://hatoba.fog.moe/download/
2. Open Hatoba, and in **Settings → General** turn on **Use the hatoba command in terminals**, then **Let other programs use saved hosts**, which appears below it. Both are off by default, and each computer has its own.

The note under the first switch says where the command went:

- **Windows**: a `bin` folder beside Hatoba, added to the user's `PATH`. A program started before that keeps its old `PATH`, so the user has to restart you, or the editor or terminal you run in. Until then, run `hatoba.exe` from the folder the note names.
- **Linux**: `~/.local/bin/hatoba`. When the note says `~/.local/bin` isn't on the `PATH`, the user signs out and back in, or adds it to their shell profile. Until then, run `~/.local/bin/hatoba`.
- **macOS**: `/usr/local/bin/hatoba`, added once the user enters an administrator password when macOS asks.

When the note says the file already exists and isn't Hatoba's, the user moves or renames that file and turns the switch on again.

## `can't reach Hatoba`

`can't reach Hatoba. Open Hatoba, and turn on Settings > General > "Let other programs use saved hosts".` means Hatoba isn't running or that switch is off. Ask the user to open Hatoba and turn it on.

If Hatoba is running with the switch on and the message stays, you are running where Hatoba can't be reached. Only programs that run as the same user on the same computer can reach it: a sandbox that blocks local sockets or named pipes, a container, a remote development environment or a cloud agent can't. Tell the user, so they can let the command run outside the sandbox or use an agent on their computer.

`can't reach Hatoba: <reason>` and `can't reach Hatoba safely: <reason>` come from the connection to Hatoba itself. Pass the reason on and ask the user to quit and reopen Hatoba.

## Hatoba's state

- `Hatoba is locked. Unlock it, then run the command again.`: ask the user to unlock Hatoba.
- `Hatoba locked, which stopped the command.`: Hatoba locked while the command ran. Once the user unlocks it, check what the command got done on the host before you run it again.
- `"Let other programs use saved hosts" or the hatoba command was turned off in Hatoba, which stopped …`: the user turned it off. Ask before you use Hatoba again.
- `this hatoba command comes from another version of Hatoba`: ask the user to quit and reopen Hatoba, which updates the command.

## Hosts

- `no hosts are saved in Hatoba yet`: the user adds hosts in Hatoba, or imports them with **Import** on the hosts page, which reads `~/.ssh/config` and other SSH clients' sessions.
- `no saved host is named <name>`: compare the user's words with `hatoba hosts --json` again. If no host matches, ask the user which one they mean, or whether to add the machine to Hatoba.
- `<n> saved hosts are named <name>. Name one by its id instead`: tell them apart by `address`, `group` and `tags` in `hatoba hosts --json`, and ask the user when that doesn't settle it.

## Signing in

- `waiting for you to check the host key of <host> in Hatoba` or `waiting for you to sign in to <host> in Hatoba`: Hatoba asks in its window. Tell the user to answer there; the command goes on by itself.
- With `--no-prompt`, the same cases fail instead: `<host> has no saved password`, `the host key of <host> … isn't trusted yet`, `<host> asks for a code or a password that isn't saved`. Run the command again without `--no-prompt` while the user is there to answer.
- `the host key of <host> … has changed`: tell the user. A reinstalled server changes its key, and so does someone intercepting the connection; the user checks it in Hatoba.
- `can't connect to <host>: <reason>` or `the connection to <host> was lost: <reason>`: pass the reason on. `hatoba <host>` opens a terminal on the host in Hatoba, where the user sees the same failure, and https://hatoba.fog.moe/docs/troubleshooting-connections/ covers the common ones.

## Running and copying

- `<host> runs commands with cmd.exe or PowerShell, so --cwd cannot start one in another directory`: leave out `--cwd` and change the directory in the command.
- `the hatoba command ended, which stopped the command.`: your call ended, as on a tool timeout, and Hatoba stopped the command. When it adds `It may still be running on the host.`, check before you run it again. Start long work with `nohup` as SKILL.md describes.
- `the command ended without an exit status, as when a signal ends it`: something on the host killed it, such as the out-of-memory killer. Check the host's logs before you run it again.
- `<path> is a folder. hatoba cp copies one file at a time.`: archive the folder with `tar`, copy the archive, and unpack it on the other side.
