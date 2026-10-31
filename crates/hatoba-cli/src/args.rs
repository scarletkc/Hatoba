//! Reading the command line of `hatoba hosts`, `hatoba exec` and `hatoba cp`.

use std::ffi::OsString;

/// The name of the switch in Settings → General, as the English interface writes it.
pub const SWITCH: &str = "Let other programs use saved hosts";

pub const HELP: &str = "\
hatoba: use the hosts saved in Hatoba from the command line

Usage:
  hatoba hosts [--json]                         List the saved SSH hosts
  hatoba exec [options] <host> [--] <command>   Run a command on a host
  hatoba cp [options] <source> <target>         Copy a file to or from a host
  hatoba <name> | hatoba [user@]<address>       Open a terminal in Hatoba
  hatoba telnet <address> [<port>]              Open a Telnet terminal in Hatoba

<host> is the name or id of a saved SSH host, as hatoba hosts --json lists
them; <name> is the name of a saved SSH or Telnet host. Any other first word
also opens Hatoba, so a misspelled command prints nothing and exits with 0.
In cp, one side is <host>:<path> and the other a local path. A remote path is
from the home directory. A target that is a folder or ends with / gets the file
under its own name. A file already at the target is replaced; on the host, the
new file belongs to the login user with default permissions. A copy that fails
leaves the old file as it was.

Options:
  --cwd <dir>    Start the command in <dir>, absolute or from the home directory
  --sudo         Run the command, or read or write the file on the host, as root
  --no-prompt    Fail instead of asking in Hatoba for a host key, password or code
  --json         Print the hosts as JSON

exec runs the command with the host's login shell, without a terminal, and with
an empty stdin. It prints the command's stdout and stderr as they come and exits
with the command's exit status, or with 255 when Hatoba failed, which may be
after the command started. Ending exec stops the command and what it started;
on hosts without sh, such as Windows, it may keep running. A command that ends
by itself leaves what it started in the background running. hosts and cp exit
with 1 when they fail, and a command line hatoba can't read exits with 2.

--sudo runs the command as root through sudo, which has to let the login user
run sh as root. The command starts in the same directory with the same shell,
but with the environment sudo sets, so ~ and $HOME in it are root's. cp --sudo
copies the file as root, as sudo cp would: a file already at the target keeps
its owner and permissions, a new one belongs to root, and a copy to the host
that fails while writing can leave the file cut short. When sudo asks for a
password, Hatoba tries the host's saved password, then asks in its window and
remembers the answer until it locks or the host has had no --sudo for 15
minutes.

A host key not trusted yet, or a password or code that isn't saved, is asked in
the Hatoba window, and the command waits up to 10 minutes for the answer. A
jump host can't ask for a password: it needs a saved one, a key or the SSH
agent. Passwords and keys stay in Hatoba.

Hatoba has to be running and unlocked, with Settings > General >
\"Let other programs use saved hosts\" on.
";

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Hosts {
        json: bool,
    },
    Exec {
        host: String,
        command: String,
        cwd: Option<String>,
        prompt: bool,
        sudo: bool,
    },
    Copy {
        source: Place,
        target: Place,
        prompt: bool,
        sudo: bool,
    },
    /// Anything else opens Hatoba with these arguments (HOST-13).
    Open(Vec<OsString>),
}

/// One side of `hatoba cp`.
#[derive(Debug, PartialEq, Eq)]
pub enum Place {
    Local(String),
    Remote { host: String, path: String },
}

/// The words that are this command's own when they come first.
pub const COMMANDS: &[&str] = &["hosts", "exec", "cp", "help", "-h", "--help", "--version"];

/// Reads the arguments after the program.
pub fn parse(args: Vec<OsString>) -> Result<Command, String> {
    let Some(first) = args.first().and_then(|a| a.to_str()) else {
        return Ok(Command::Open(args));
    };
    if !COMMANDS.contains(&first) {
        return Ok(Command::Open(args));
    }
    let rest = args[1..]
        .iter()
        .map(|a| {
            a.to_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{} is not valid text", a.to_string_lossy()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    match first {
        "hosts" => hosts(&rest),
        "exec" => exec(rest),
        "cp" => copy(&rest),
        "--version" => Ok(Command::Version),
        _ => Ok(Command::Help),
    }
}

fn hosts(args: &[String]) -> Result<Command, String> {
    let mut json = false;
    for arg in args {
        match arg.as_str() {
            "--json" => json = true,
            "-h" | "--help" => return Ok(Command::Help),
            other => return Err(format!("hosts does not take {other}")),
        }
    }
    Ok(Command::Hosts { json })
}

/// Options until the host, and after it until the command, which starts at the first other
/// argument or after `--` and takes the rest, joined with spaces as `ssh` joins them.
fn exec(args: Vec<String>) -> Result<Command, String> {
    let mut cwd = None;
    let mut prompt = true;
    let mut sudo = false;
    let mut host = None;
    let mut args = args.into_iter();
    let mut command = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--" if host.is_some() => {
                command.extend(args.by_ref());
                break;
            }
            "--cwd" => cwd = Some(args.next().ok_or("--cwd needs a directory")?),
            "--no-prompt" => prompt = false,
            "--sudo" => sudo = true,
            "-h" | "--help" => return Ok(Command::Help),
            a if a.starts_with("--cwd=") => cwd = Some(a["--cwd=".len()..].to_owned()),
            a if a.starts_with('-') && a != "-" => {
                return Err(format!(
                    "exec does not take {a}; put -- before a command that starts with -"
                ));
            }
            _ if host.is_none() => host = Some(arg),
            _ => {
                command.push(arg);
                command.extend(args.by_ref());
                break;
            }
        }
    }
    let host = host.ok_or("exec needs a host and a command: hatoba exec <host> <command>")?;
    let command = command.join(" ");
    if command.trim().is_empty() {
        return Err(format!(
            "exec needs a command: hatoba exec {host} <command>"
        ));
    }
    Ok(Command::Exec {
        host,
        command,
        cwd: cwd.filter(|d| !d.is_empty()),
        prompt,
        sudo,
    })
}

fn copy(args: &[String]) -> Result<Command, String> {
    let mut prompt = true;
    let mut sudo = false;
    let mut paths = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-prompt" => prompt = false,
            "--sudo" => sudo = true,
            "-h" | "--help" => return Ok(Command::Help),
            "--" => {
                paths.extend(args.by_ref().map(String::as_str));
                break;
            }
            a if a.starts_with('-') && a.len() > 1 => {
                return Err(format!(
                    "cp does not take {a}; put -- before a path that starts with -"
                ));
            }
            _ => paths.push(arg.as_str()),
        }
    }
    let [source, target] = paths[..] else {
        return Err("cp needs a source and a target: hatoba cp <source> <target>".into());
    };
    let (source, target) = (place(source), place(target));
    match (&source, &target) {
        (Place::Local(_), Place::Remote { .. }) | (Place::Remote { .. }, Place::Local(_)) => {
            Ok(Command::Copy {
                source,
                target,
                prompt,
                sudo,
            })
        }
        _ => Err(
            "one of the source and the target has to be <host>:<path>, the other a local path"
                .into(),
        ),
    }
}

/// `host:path` is remote. A local path may hold `:` after a start that tells it apart: a drive
/// letter on Windows, `/`, `\`, `.` or `~`.
fn place(arg: &str) -> Place {
    let bytes = arg.as_bytes();
    let drive =
        cfg!(windows) && bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    if drive || arg.starts_with(['/', '\\', '.', '~']) {
        return Place::Local(arg.to_owned());
    }
    match arg.split_once(':') {
        Some((host, path)) if !host.is_empty() => Place::Remote {
            host: host.to_owned(),
            path: path.to_owned(),
        },
        _ => Place::Local(arg.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str]) -> Result<Command, String> {
        parse(words.iter().map(OsString::from).collect())
    }

    #[test]
    fn other_command_lines_open_hatoba() {
        for words in [
            &[][..],
            &["root@db"],
            &["My Server"],
            &["-p", "2222", "db"],
            &["ssh", "db"],
        ] {
            assert_eq!(
                parse_words(words),
                Ok(Command::Open(words.iter().map(OsString::from).collect()))
            );
        }
    }

    #[test]
    fn exec_takes_options_then_the_host_then_the_command() {
        let exec = |host: &str, command: &str, cwd: Option<&str>, prompt| {
            Ok(Command::Exec {
                host: host.into(),
                command: command.into(),
                cwd: cwd.map(Into::into),
                prompt,
                sudo: false,
            })
        };
        assert_eq!(
            parse_words(&["exec", "db", "ls", "-la", "/tmp"]),
            exec("db", "ls -la /tmp", None, true)
        );
        assert_eq!(
            parse_words(&[
                "exec",
                "--cwd",
                "/srv",
                "db",
                "--no-prompt",
                "make",
                "--jobs=4"
            ]),
            exec("db", "make --jobs=4", Some("/srv"), false)
        );
        assert_eq!(
            parse_words(&["exec", "My Server", "--cwd=app", "--", "-x", "--cwd"]),
            exec("My Server", "-x --cwd", Some("app"), true)
        );
        assert_eq!(
            parse_words(&["exec", "db", "cd /app && make"]),
            exec("db", "cd /app && make", None, true)
        );
        assert_eq!(
            parse_words(&["exec", "--sudo", "db", "systemctl", "restart", "nginx"]),
            Ok(Command::Exec {
                host: "db".into(),
                command: "systemctl restart nginx".into(),
                cwd: None,
                prompt: true,
                sudo: true,
            })
        );
        // Once the command starts, --sudo is the command's.
        assert_eq!(
            parse_words(&["exec", "db", "ls", "--sudo"]),
            exec("db", "ls --sudo", None, true)
        );
        assert!(parse_words(&["exec", "db"]).is_err());
        assert!(parse_words(&["exec"]).is_err());
        assert!(parse_words(&["exec", "db", "--cwd"]).is_err());
        assert!(parse_words(&["exec", "db", "-x"]).is_err());
        assert_eq!(parse_words(&["exec", "--help"]), Ok(Command::Help));
    }

    #[test]
    fn cp_takes_one_remote_and_one_local_side() {
        assert_eq!(
            parse_words(&["cp", "build/app.tar", "db:/tmp/"]),
            Ok(Command::Copy {
                source: Place::Local("build/app.tar".into()),
                target: Place::Remote {
                    host: "db".into(),
                    path: "/tmp/".into()
                },
                prompt: true,
                sudo: false,
            })
        );
        assert_eq!(
            parse_words(&[
                "cp",
                "--no-prompt",
                "My Server:logs/a.log",
                "--sudo",
                "./a:b.log"
            ]),
            Ok(Command::Copy {
                source: Place::Remote {
                    host: "My Server".into(),
                    path: "logs/a.log".into()
                },
                target: Place::Local("./a:b.log".into()),
                prompt: false,
                sudo: true,
            })
        );
        assert_eq!(
            parse_words(&["cp", "--", "-report.txt", "db:"]),
            Ok(Command::Copy {
                source: Place::Local("-report.txt".into()),
                target: Place::Remote {
                    host: "db".into(),
                    path: String::new()
                },
                prompt: true,
                sudo: false,
            })
        );
        assert!(parse_words(&["cp", "-report.txt", "db:"]).is_err());
        assert!(parse_words(&["cp", "a", "b"]).is_err());
        assert!(parse_words(&["cp", "db:a", "web:b"]).is_err());
        assert!(parse_words(&["cp", "db:a"]).is_err());
        if cfg!(windows) {
            assert_eq!(place(r"C:\x\y"), Place::Local(r"C:\x\y".into()));
        }
    }

    #[test]
    fn hosts_and_help() {
        assert_eq!(parse_words(&["hosts"]), Ok(Command::Hosts { json: false }));
        assert_eq!(
            parse_words(&["hosts", "--json"]),
            Ok(Command::Hosts { json: true })
        );
        assert!(parse_words(&["hosts", "db"]).is_err());
        for word in ["help", "-h", "--help"] {
            assert_eq!(parse_words(&[word]), Ok(Command::Help));
        }
        assert_eq!(parse_words(&["--version"]), Ok(Command::Version));
        assert!(HELP.contains(SWITCH));
    }
}
