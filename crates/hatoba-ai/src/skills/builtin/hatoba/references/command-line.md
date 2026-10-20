# The hatoba command

## Opening hosts

Settings → General → **Use the hatoba command in terminals** (off by default, kept per device) adds a `hatoba` command: on Windows in a `bin` folder beside Hatoba that goes on your `PATH`, on Linux at `~/.local/bin/hatoba`, and on macOS at `/usr/local/bin/hatoba` after macOS asks for an administrator's password. `hatoba <host name>` connects that saved host in Hatoba, and `hatoba user@host` or other `ssh` arguments connect as quick connect does. Hatoba comes to the front, and a launch while it is locked waits for the unlock.

## Other programs

**Let other programs use saved hosts**, below it, lets coding agents such as Claude Code and Codex, and scripts, work on the saved hosts without a copy of their passwords or keys:

- `hatoba hosts` lists the saved hosts with their `user@address:port` and group. `hatoba hosts --json` also gives each one's id, how it signs in (`password`, `key`, `agent`, or `ask` for a password Hatoba asks for), its tags, jump host and AI Notes. Neither shows a password or a key.
- `hatoba exec <host> <command>` runs the command with the host's login shell, without a terminal and with an empty stdin, and prints its output as it comes. It exits with the command's exit status, or with 255 when Hatoba could not run it. `--cwd <dir>` starts it in another directory, absolute or from the home directory; a Windows host runs commands with cmd.exe or PowerShell and refuses it, so change the directory in the command there.
- `hatoba cp <file> <host>:<path>` uploads one file and `hatoba cp <host>:<path> <file>` downloads one, over SFTP. A remote path is from the home directory, and a target that is a folder or ends with `/` gets the file under its own name.
- `<host>` is a saved host's name or id. Hatoba has to be running and unlocked. It connects with the host's saved sign-in, jump host and proxy, uses an open terminal's connection to the host when there is one, and otherwise keeps its own for later commands until it has been unused for 10 minutes.
- When a connection needs you, for a host key that is not trusted yet, a password that is not saved or a code, Hatoba comes to the front and asks as it does for a terminal, and the command says what it waits for. With `--no-prompt` it fails instead.
- Ending the command (Ctrl+C, or a timeout of the program that ran it) stops what it started on the host. Locking Hatoba stops every command and copy and closes their connections.
- **Recent use**, below the switch, counts what other programs ran until Hatoba locks, and **Details** beside it lists each one, on which host and how it ended.
- Only programs that run as you on this computer can reach Hatoba this way. `hatoba --help` lists the commands and options.
