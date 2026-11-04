//! What command suggestions take from the server (TERM-16): the commands of the shell's own history
//! (`~/.bash_history`, `~/.zsh_history` in either of zsh's formats, and fish's history) and the
//! names of the commands on `PATH`. One exec channel runs `suggest.sh` as `sh -s`, as resource
//! usage does (TERM-12), and closes once it has answered; nothing is installed or changed on the
//! server.

use std::collections::{BTreeSet, HashSet};
use std::time::Duration;

use crate::error::SshError;
use crate::exec::Exec;

const SCRIPT: &str = include_str!("suggest.sh");

/// The commands kept from the histories, as many as Hatoba's own history keeps (TERM-15).
pub const HISTORY_LIMIT: usize = 1000;
/// Longer commands are left out, as Hatoba's own history leaves them out.
const COMMAND_MAX_CHARS: usize = 4096;
/// The command names kept.
pub const COMMANDS_LIMIT: usize = 10_000;
/// Longer command names are left out (`NAME_MAX` on Linux).
const NAME_MAX_BYTES: usize = 255;
/// How long the server has to answer: a home folder on a network filesystem that stopped answering
/// would otherwise hold the channel open.
const TIMEOUT: Duration = Duration::from_secs(10);

const HEADER: &str = "hatoba-suggest ";
/// zsh's `Meta` byte: the byte after it is stored XOR 32 (`unmetafy` in zsh's `utils.c`).
const ZSH_META: u8 = 0x83;

/// What the server offers command suggestions.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerSuggestions {
    /// The shell's own history, newest first, each command once: the login shell's history first,
    /// then the others'.
    pub history: Vec<String>,
    /// The names of the executables on `PATH`, sorted, each once.
    pub commands: Vec<String>,
}

/// Reads the histories and the command names over one exec channel of `session`.
pub async fn read(session: &impl Exec) -> Result<ServerSuggestions, SshError> {
    let out = tokio::time::timeout(
        TIMEOUT,
        session.exec_output("sh -s", Some(SCRIPT.as_bytes().to_vec())),
    )
    .await
    .map_err(|_| SshError::other("the server did not answer in time"))??;
    Ok(parse(&out.stdout))
}

/// The parts of the script's output, in the order the script prints them.
#[derive(Default)]
struct Parts<'a> {
    shell: String,
    bash: Vec<&'a [u8]>,
    zsh: Vec<&'a [u8]>,
    fish: Vec<&'a [u8]>,
    path: Vec<&'a [u8]>,
}

const SECTIONS: [&str; 6] = ["shell", "bash", "zsh", "fish", "path", "end"];

fn split(stdout: &[u8]) -> Parts<'_> {
    let mut parts = Parts::default();
    // The index in SECTIONS of the part being read; None before the first header.
    let mut at: Option<usize> = None;
    for line in stdout.split(|&b| b == b'\n') {
        let next = at.map_or(0, |i| i + 1);
        let header = line.strip_prefix(HEADER.as_bytes());
        // The first header carries the shell's name, empty when `SHELL` is not set.
        let shell = header
            .filter(|_| next == 0)
            .and_then(|rest| rest.strip_prefix(b"shell "));
        let expected = SECTIONS.get(next).map(|name| name.as_bytes());
        if shell.is_some() || (header.is_some() && header == expected) {
            at = Some(next);
            if let Some(shell) = shell {
                parts.shell = String::from_utf8_lossy(shell).trim().to_owned();
            }
            continue;
        }
        match at.map(|i| SECTIONS[i]) {
            Some("bash") => parts.bash.push(line),
            Some("zsh") => parts.zsh.push(line),
            Some("fish") => parts.fish.push(line),
            Some("path") => parts.path.push(line),
            _ => {}
        }
    }
    parts
}

fn parse(stdout: &[u8]) -> ServerSuggestions {
    let parts = split(stdout);
    let bash = bash_history(&parts.bash);
    let zsh = zsh_history(&parts.zsh);
    let fish = fish_history(&parts.fish);
    let order: [&[String]; 3] = match parts.shell.as_str() {
        "zsh" => [&zsh, &bash, &fish],
        "fish" => [&fish, &bash, &zsh],
        _ => [&bash, &zsh, &fish],
    };
    let mut seen = HashSet::new();
    let mut history = Vec::new();
    'files: for file in order {
        for command in file.iter().rev() {
            if history.len() == HISTORY_LIMIT {
                break 'files;
            }
            if keeps(command) && seen.insert(command.as_str()) {
                history.push(command.clone());
            }
        }
    }
    let commands: BTreeSet<String> = parts
        .path
        .iter()
        .filter(|name| !name.is_empty() && name.len() <= NAME_MAX_BYTES)
        .filter_map(|name| std::str::from_utf8(name).ok())
        .filter(|name| !name.chars().any(|c| c.is_whitespace() || c.is_control()))
        .map(str::to_owned)
        .collect();
    ServerSuggestions {
        history,
        commands: commands.into_iter().take(COMMANDS_LIMIT).collect(),
    }
}

/// Whether a command is offered: not blank, not starting with a space (left out of the shell's
/// own history by `HISTCONTROL=ignorespace`, and of Hatoba's), one line without control characters,
/// which → would type as keys, and not too long.
fn keeps(command: &str) -> bool {
    !command.trim().is_empty()
        && !command.starts_with(char::is_whitespace)
        && !command.chars().any(char::is_control)
        && command.chars().count() <= COMMAND_MAX_CHARS
}

/// bash's history, oldest first: one command per line. `HISTTIMEFORMAT` adds a `#<seconds>` line
/// before each entry, the only mark of where one ends, so several lines between two such lines
/// are one entry, left out whole. Before the first and after the last, each line counts as a
/// command: a shell without `HISTTIMEFORMAT` adds its commands with no such line.
fn bash_history(lines: &[&[u8]]) -> Vec<String> {
    let lines: Vec<&[u8]> = lines.iter().copied().filter(|l| !l.is_empty()).collect();
    let groups: Vec<&[&[u8]]> = lines.split(|line| is_timestamp(line)).collect();
    let last = groups.len() - 1;
    groups
        .iter()
        .enumerate()
        .filter(|(i, group)| *i == 0 || *i == last || group.len() == 1)
        .flat_map(|(_, group)| group.iter())
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .collect()
}

fn is_timestamp(line: &[u8]) -> bool {
    matches!(line, [b'#', digits @ ..] if !digits.is_empty() && digits.iter().all(u8::is_ascii_digit))
}

/// zsh's history, oldest first, in its plain format or the extended one
/// (`: <start>:<elapsed>;cmd`). A command of several lines continues each line with a backslash; it
/// is left out whole.
fn zsh_history(lines: &[&[u8]]) -> Vec<String> {
    let mut commands = Vec::new();
    let mut continued = false;
    for line in lines {
        let line = unmetafy(line);
        let in_entry = continued;
        continued = line.last() == Some(&b'\\');
        if in_entry || continued {
            continue;
        }
        let command = extended_command(&line).unwrap_or(&line);
        commands.push(String::from_utf8_lossy(command).into_owned());
    }
    commands
}

/// The command of a line in zsh's extended format, `: 1700000000:0;git status`.
fn extended_command(line: &[u8]) -> Option<&[u8]> {
    let rest = line.strip_prefix(b": ")?;
    let semi = rest.iter().position(|&b| b == b';')?;
    let (start, elapsed) = rest[..semi].split_at(rest[..semi].iter().position(|&b| b == b':')?);
    let digits = |s: &[u8]| !s.is_empty() && s.iter().all(u8::is_ascii_digit);
    (digits(start) && digits(&elapsed[1..])).then_some(&rest[semi + 1..])
}

/// Undoes zsh's metafication of the bytes it stores: `Meta` and the byte after it XOR 32.
fn unmetafy(line: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len());
    let mut bytes = line.iter();
    while let Some(&b) = bytes.next() {
        if b == ZSH_META {
            if let Some(&next) = bytes.next() {
                out.push(next ^ 32);
            }
        } else {
            out.push(b);
        }
    }
    out
}

/// fish's history, oldest first: the `- cmd: ` lines, where fish writes a backslash as `\\` and a
/// line break as `\n`.
fn fish_history(lines: &[&[u8]]) -> Vec<String> {
    lines
        .iter()
        .filter_map(|line| line.strip_prefix(b"- cmd: "))
        .map(|escaped| {
            let mut out = Vec::with_capacity(escaped.len());
            let mut bytes = escaped.iter();
            while let Some(&b) = bytes.next() {
                match (b, bytes.as_slice().first()) {
                    (b'\\', Some(b'\\')) => {
                        out.push(b'\\');
                        bytes.next();
                    }
                    (b'\\', Some(b'n')) => {
                        out.push(b'\n');
                        bytes.next();
                    }
                    _ => out.push(b),
                }
            }
            String::from_utf8_lossy(&out).into_owned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(shell: &str, bash: &str, zsh: &[u8], fish: &str, path: &str) -> Vec<u8> {
        let mut out = b"Welcome to the server\n".to_vec();
        out.extend_from_slice(
            format!("hatoba-suggest shell {shell}\nhatoba-suggest bash\n{bash}\n").as_bytes(),
        );
        out.extend_from_slice(b"hatoba-suggest zsh\n");
        out.extend_from_slice(zsh);
        out.extend_from_slice(
            format!(
                "\nhatoba-suggest fish\n{fish}\nhatoba-suggest path\n{path}hatoba-suggest end\n"
            )
            .as_bytes(),
        );
        out
    }

    #[test]
    fn reads_each_history_newest_first_the_login_shells_first() {
        let out = output(
            "zsh",
            "#1700000000\nls -la\n#1700000001\nmake\n",
            b": 1700000001:0;git status\n: 1700000002:3;cargo test\n",
            "- cmd: fish_config\n  when: 1700000003\n  paths:\n    - ~/x\n",
            "",
        );
        let s = parse(&out);
        assert_eq!(
            s.history,
            ["cargo test", "git status", "make", "ls -la", "fish_config"]
        );
        let s = parse(&output("bash", "make\n", b"git status\n", "", ""));
        assert_eq!(s.history, ["make", "git status"]);
    }

    #[test]
    fn leaves_out_repeats_spaced_multiline_and_long_commands() {
        let long = "x".repeat(COMMAND_MAX_CHARS + 1);
        let bash = format!("make\n ls secret\n{long}\nmake\nprintf 'a\tb'\n\n");
        let zsh = b": 1700000000:0;echo one\\\ntwo\nuptime\n";
        let fish = "- cmd: echo a\\nb\n- cmd: echo back\\\\slash\n";
        let s = parse(&output("bash", &bash, zsh, fish, ""));
        assert_eq!(s.history, ["make", "uptime", "echo back\\slash"]);
    }

    #[test]
    fn leaves_out_a_bash_entry_of_several_lines_where_the_file_marks_entries() {
        let bash = "older\n#1700000000\nls -la\n#1700000001\nprintf \"first\nsecond\"\n\
                    #1700000002\nmake\n";
        let s = parse(&output("bash", bash, b"", "", ""));
        assert_eq!(s.history, ["make", "ls -la", "older"]);
    }

    #[test]
    fn keeps_bash_commands_added_after_the_last_mark() {
        let bash = "#1700000000\nls -la\nmake\ngit status\n";
        let s = parse(&output("bash", bash, b"", "", ""));
        assert_eq!(s.history, ["git status", "make", "ls -la"]);
    }

    #[test]
    fn unmetafies_zsh_history() {
        // "ア" is E3 82 A2; zsh stores A2 as Meta 82.
        let s = parse(&output("zsh", "", b"echo \xe3\x82\x83\x82\n", "", ""));
        assert_eq!(s.history, ["echo ア"]);
    }

    #[test]
    fn lists_command_names_sorted_once() {
        let s = parse(&output("bash", "", b"", "", "ls\ngit\nls\nbad name\n\n"));
        assert_eq!(s.commands, ["git", "ls"]);
    }

    #[test]
    fn needs_the_header_before_anything_counts() {
        assert_eq!(parse(b"ls\nmake\n"), ServerSuggestions::default());
    }

    #[cfg(unix)]
    #[test]
    fn the_script_reads_the_files_and_path() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(".bash_history"), "ls -la\nmake").unwrap();
        std::fs::write(
            home.path().join(".zsh_history"),
            ": 1700000000:0;git status\n",
        )
        .unwrap();
        let bin = home.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        for (name, mode) in [("hello", 0o755), ("notes.txt", 0o644)] {
            std::fs::write(bin.join(name), "").unwrap();
            std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(mode))
                .unwrap();
        }
        let path = format!("{}:relative::/usr/bin:/bin", bin.display());
        let out = crate::test_sh::run_with(
            home.path(),
            "sh -s",
            SCRIPT.as_bytes(),
            &[("PATH", path.as_str()), ("SHELL", "/bin/bash")],
        );
        let s = parse(&out.stdout);
        assert_eq!(s.history, ["make", "ls -la", "git status"]);
        assert!(s.commands.iter().any(|c| c == "hello"));
        assert!(!s.commands.iter().any(|c| c == "notes.txt"));
    }
}
