---
name: hatoba-cli
description: "Work on the SSH hosts saved in Hatoba with the `hatoba` command: list them, run commands on them, and copy files to and from them, signed in with the credentials Hatoba keeps. Use it whenever a task needs a remote server, a host, ssh or scp and `hatoba` is on the PATH; the servers the user names are usually saved there."
---

# Hatoba hosts

Hatoba is a desktop SSH client. Its `hatoba` command lets other programs use the hosts saved in it: the running app signs in with each host's saved password or key, jump host and proxy, and returns only the output or the file. You never handle the credentials, and the hosts are usually not in `~/.ssh/config`, so `ssh <name>` fails where `hatoba exec <name>` works.

## Pick the host

Before reaching for `ssh` or `scp`, list the hosts:

```sh
hatoba hosts --json
```

Each entry has `id`, `name`, `address`, `port`, `username`, `auth` (`password`, `key`, `agent`, or `ask` for a password Hatoba asks for), `group`, `tags`, `jump` and, when the user wrote some, `ai_notes`. Match the user's words against the name, address, group and tags, and name the host by its `name` (quoted when it has spaces) or its `id` in the commands below. `ai_notes` is what the user wants assistants to know about the host, such as what runs there or what to leave alone.

Use `ssh` and `scp` only for a machine that isn't listed. If the shell can't find `hatoba`, work as you would without it.

## Run a command

```sh
hatoba exec [--cwd <dir>] [--no-prompt] <host> <command>
```

- The command runs with the host's login shell, as `ssh <host> '<command>'` would. Pass it as one quoted argument when it has pipes, `&&`, redirections or quotes of its own: `hatoba exec web 'cd /srv/app && git log -3 --oneline'` in sh or PowerShell, with double quotes in cmd.exe.
- It exits with the command's exit status. 255 with a `hatoba:` line on stderr means Hatoba couldn't run it.
- Every call is a new shell, so `cd` and `export` don't carry over. Chain the steps in one command, or pass `--cwd` with a directory that is absolute or from the home directory. Windows hosts run commands with cmd.exe or PowerShell and refuse `--cwd`; change the directory in the command there.
- There is no terminal and stdin is empty, so nothing interactive works. Use the batch form: `top -bn1`, `systemctl status nginx --no-pager`, `journalctl -u nginx -n 200 --no-pager`, `git --no-pager log`, `docker exec` without `-t`, `apt-get -y`. Change a file with `sed -i` or a heredoc in the command, or edit a local copy and `hatoba cp` it back.
- `sudo` can't ask for a password. Use `sudo -n`, and when it says a password is required, ask the user to run that step themselves: `hatoba <host>` opens a terminal on the host in Hatoba.
- Ending `hatoba exec`, as a tool timeout does, ends the command and everything it started on the host. Where Hatoba can't be sure of that, as on Windows hosts, the error says the command may still be running: check before you run it again. For work that has to outlive the call, such as a long build or a restart that drops the connection, start it in the background with its output in a file, like `nohup ./deploy.sh > deploy.log 2>&1 &`, and read the log in later calls. What a command leaves in the background keeps running after the command ends.

## Copy a file

```sh
hatoba cp [--no-prompt] <source> <target>
```

One side is `<host>:<path>` and the other a local path: `hatoba cp dist/app.tar.gz web:/tmp/` uploads, and `hatoba cp web:logs/error.log ./` downloads. A remote path is from the home directory unless it starts with `/`, and a target that is a folder or ends with `/` gets the file under its own name. Absolute local paths, Windows drive paths included, are always local; start a relative one that contains `:` with `./`. `hatoba cp` copies one file: for a folder, `tar` it on one side, copy the archive, and unpack it on the other. It exits with 1 when the copy fails.

## When Hatoba needs the user

- A host key Hatoba doesn't trust yet, a password it doesn't keep (`auth` is `ask`), or a code the server asks for is asked in the Hatoba window, and the command prints what it waits for and waits. Tell the user to answer it in Hatoba. `--no-prompt` fails at once instead, which suits a script nobody watches.
- A message about Hatoba itself, such as that it can't be reached, is locked or needs a setting turned on, says what the user has to do. Tell them, and run the command again once they have; `ssh` doesn't have the saved credentials either.
