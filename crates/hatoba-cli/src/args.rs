//! Reading the command line of `hatoba hosts`, `hatoba exec`, `hatoba cp` and `hatoba memory`.

use std::ffi::OsString;

use crate::protocol::{AuthKind, EnvVar, HostChanges};

/// The name of the switch in Settings → General, as the English interface writes it.
pub const SWITCH: &str = "Let other programs use saved hosts";

pub const HELP: &str = "\
hatoba: use the hosts saved in Hatoba from the command line

Usage:
  hatoba hosts [--json]                         List the saved SSH hosts
  hatoba hosts add [options] [user@]<address>[:<port>]
                                                Save a new SSH host
  hatoba hosts edit [options] <host>            Change a saved SSH host
  hatoba hosts delete <host>                    Delete a saved SSH host
  hatoba exec [options] <host> [--] <command>   Run a command on a host
  hatoba cp [options] <source> <target>         Copy a file to or from a host
  hatoba memory --host <host> [--json]          List a host's memories
  hatoba memory save --host <host> <name> [<text>]
                                                Save a memory of a host
  hatoba memory delete --host <host> <name>     Delete a memory of a host
  hatoba <name> | hatoba [user@]<address>       Open a terminal in Hatoba
  hatoba telnet <address> [<port>]              Open a Telnet terminal in Hatoba

<host> is the name or id of a saved SSH host, as hatoba hosts --json lists
them; <name> is the name of a saved SSH or Telnet host. Any other first word
also opens Hatoba, but a word one letter off a command, such as exce, fails.
In cp, one side is <host>:<path> and the other a local path. A remote path is
from the home directory. A target that is a folder or ends with / gets the file
under its own name. A file already at the target is replaced; on the host, the
new file belongs to the login user and keeps the permissions of the file it
replaces, or gets the default ones. A copy that fails leaves the old file as it
was.

Options:
  --cwd <dir>    Start the command in <dir>, absolute or from the home directory
  --sudo         Run the command, or read or write the file on the host, as root
  --mode <mode>  Give the file copied to the host this octal mode, such as 600
  --no-prompt    Fail instead of asking in Hatoba for a host key, password or code
  --json         Print the hosts, or the host saved, as JSON

Options of hosts add and edit:
  --name <name>                 The name; add without it names the host after
                                its address
  --address <address>, --port <port>, --user <user>
                                edit only; add takes them from its target
  --auth password|key|agent|ask How it signs in; ask asks for the password on
                                each connection
  --key <key>                   The key in Hatoba it signs in with (implies
                                --auth key)
  --password-stdin              Read the password from stdin (implies --auth
                                password)
  --jump <host>, --no-jump      Connect through a saved SSH host, or directly
  --proxy <proxy>|direct|default
                                A saved proxy, none, or this device's default
  --group <group>, --no-group   The group by name; a name no group has creates
                                the group
  --tag <tag>, --untag <tag>    Add or remove a tag; both repeat
  --env NAME=value, --unset-env NAME
                                Set or remove an environment variable; both
                                repeat
  --note <text>, --ai-notes <text>
                                Replace the note or the AI Notes; \"\" clears them
  --favorite, --no-favorite     Mark it as a favorite, or not

hosts add saves a host as New Host in Hatoba does, and edit changes only the
fields its options name. Without user@, add saves this computer's user name,
which ssh would sign in as. A <key>, a jump <host> and a <proxy> are a name or
an id, as hosts --json lists them. --auth password without --password-stdin
asks for the password in the Hatoba window. An edit without --auth keeps the
saved password, and no command prints one. A change the host editor would
refuse fails with its message and changes nothing. add and edit print the host
as hosts lists it. delete deletes the host with its port forwarding rules, and
hosts that jumped through it connect directly. A change syncs as one made in
Hatoba does.

exec runs the command with the host's login shell and without a terminal. A
stdin from a pipe or a file is the command's, as with ssh, so a script or a
file's content needs no quoting: hatoba exec db 'sh -s' < deploy.sh. From a
terminal or /dev/null, the command's stdin is empty. exec prints the command's
stdout and stderr as they come and exits with the command's exit status, or
with 255 when Hatoba failed, which may be after the command started. Ending
exec stops the command and what it started; on hosts without sh, such as
Windows, it may keep running. A command that ends by itself leaves what it
started in the background running. hosts and cp exit with 1 when they fail,
and a command line hatoba can't read exits with 2.

--sudo runs the command as root through sudo, which has to let the login user
run sh as root. The command starts in the same directory with the same shell,
but with the environment sudo sets, so ~ and $HOME in it are root's. cp --sudo
copies the file as root, as sudo cp would: a file already at the target keeps
its owner and permissions, a new one belongs to root, --mode sets the
permissions of either, and a copy to the host that fails while writing can
leave the file cut short. When sudo asks for a password, Hatoba tries the
host's saved password, then asks in its window and remembers the answer until
it locks or the host has had no --sudo for 15 minutes.

A host key not trusted yet, or a password or code that isn't saved, is asked in
the Hatoba window, and the command waits up to 10 minutes for the answer. A
jump host can't ask for a password: it needs a saved one, a key or the SSH
agent. Passwords and keys stay in Hatoba. A message the server sends while
signing in goes to stderr. Without --no-prompt, the command waits while the
server holds the login, as Tailscale SSH does until you sign in at the link it
prints.

memory reads and changes what Hatoba's AI assistant remembers about a host
between conversations: notes that it, the user and other programs save, such
as the host's paths, services and quirks. The assistant starts its
conversations on the host with them, and Settings > AI > Memory lists them.
save keeps <text>, or its stdin when there is none, under <name>, in place of
the host's memory of that name, and delete removes one. Save a fact about the
host, or a correction of the user about working on it, that will still matter
later and can't be found by looking. Leave out what only matters to the task
at hand, what the host's ai_notes already say, and passwords, keys or tokens.
When a memory turns out wrong or out of date, save it again or delete it
instead of adding another. Where a memory differs from the host's ai_notes,
follow those.

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
    /// `hatoba hosts add`: `changes.address` is set.
    HostAdd(HostChange),
    /// `hatoba hosts edit`.
    HostEdit {
        host: String,
        change: HostChange,
    },
    HostDelete {
        host: String,
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
        /// The permission bits of the file a copy to the host writes.
        mode: Option<u32>,
    },
    /// The memories of a host.
    Memories {
        host: String,
        json: bool,
    },
    MemorySave {
        host: String,
        name: String,
        /// `None` takes the text from stdin.
        content: Option<String>,
    },
    MemoryDelete {
        host: String,
        name: String,
    },
    /// Anything else opens Hatoba with these arguments (HOST-13).
    Open(Vec<OsString>),
}

/// What `hatoba hosts add` or `edit` changes, and how.
#[derive(Debug, PartialEq, Eq)]
pub struct HostChange {
    /// Without the password, which `password_stdin` reads.
    pub changes: HostChanges,
    pub password_stdin: bool,
    pub prompt: bool,
    pub json: bool,
}

/// One side of `hatoba cp`.
#[derive(Debug, PartialEq, Eq)]
pub enum Place {
    Local(String),
    Remote { host: String, path: String },
}

/// The words that are this command's own when they come first.
pub const COMMANDS: &[&str] = &[
    "hosts",
    "exec",
    "cp",
    "memory",
    "help",
    "-h",
    "--help",
    "--version",
];

/// Reads the arguments after the program.
pub fn parse(args: Vec<OsString>) -> Result<Command, String> {
    let Some(first) = args.first().and_then(|a| a.to_str()) else {
        return Ok(Command::Open(args));
    };
    if !COMMANDS.contains(&first) {
        if let Some(command) = misspelled(first) {
            return Err(format!("{first} is not a command. Did you mean {command}?"));
        }
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
        "hosts" => hosts(rest),
        "exec" => exec(rest),
        "cp" => copy(&rest),
        "memory" => memory(rest),
        "--version" => Ok(Command::Version),
        _ => Ok(Command::Help),
    }
}

/// The command `word` is one letter off, by a typo or two letters swapped, as `exce` is from
/// `exec`. `cp` is left out: a two-letter host name is as likely as a typo of it.
fn misspelled(word: &str) -> Option<&'static str> {
    const LONG: [&str; 5] = ["hosts", "exec", "memory", "help", "telnet"];
    let word = word.to_ascii_lowercase();
    LONG.into_iter()
        .find(|command| word != *command && one_letter_off(&word, command))
}

/// Whether `a` becomes `b` by adding, removing or changing one character, or swapping two that
/// are next to each other.
fn one_letter_off(a: &str, b: &str) -> bool {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let start = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let (a, b) = (&a[start..], &b[start..]);
    match (a.len(), b.len()) {
        (m, n) if m == n => {
            a[1..] == b[1..] || (m >= 2 && a[0] == b[1] && a[1] == b[0] && a[2..] == b[2..])
        }
        (m, n) if m == n + 1 => a[1..] == *b,
        (m, n) if m + 1 == n => *a == b[1..],
        _ => false,
    }
}

/// `hosts [--json]` lists the hosts; `add`, `edit` or `delete` first changes one ([`host_change`]).
fn hosts(args: Vec<String>) -> Result<Command, String> {
    if let Some(word) = args.first().map(String::as_str)
        && matches!(word, "add" | "edit" | "delete")
    {
        return host_change(args);
    }
    let mut json = false;
    for arg in &args {
        match arg.as_str() {
            "--json" => json = true,
            "-h" | "--help" => return Ok(Command::Help),
            other => return Err(format!("hosts does not take {other}")),
        }
    }
    Ok(Command::Hosts { json })
}

/// `hosts add`, `edit` or `delete`, then the options and one other argument, the target or the
/// host, in any order. `--` ends the options, and an option's value can follow it after `=`.
fn host_change(args: Vec<String>) -> Result<Command, String> {
    let mut args = args.into_iter();
    let word = args.next().unwrap_or_default();
    let mut changes = HostChanges::default();
    let (mut password_stdin, mut prompt, mut json) = (false, true, false);
    // The options given, without their values.
    let mut given: Vec<String> = Vec::new();
    let mut positionals = Vec::new();
    let mut options = true;
    while let Some(arg) = args.next() {
        if !options || !arg.starts_with('-') || arg == "-" {
            positionals.push(arg);
            continue;
        }
        if arg == "--" {
            options = false;
            continue;
        }
        let (name, inline) = match arg.split_once('=') {
            Some((name, value)) if name.starts_with("--") => {
                (name.to_owned(), Some(value.to_owned()))
            }
            _ => (arg, None),
        };
        let value = if known_option(&name) {
            match inline.or_else(|| args.next()) {
                Some(value) => value,
                None => return Err(format!("{name} needs {}", needs(&name))),
            }
        } else if inline.is_some() && HOST_FLAGS.contains(&name.as_str()) {
            return Err(format!("{name} takes no value"));
        } else {
            String::new()
        };
        match name.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "--name" => changes.name = Some(value),
            "--address" => changes.address = Some(value),
            "--port" => changes.port = Some(port(&value)?),
            "--user" => changes.username = Some(value),
            "--auth" => changes.auth = Some(auth(&value)?),
            "--key" => changes.key = Some(value),
            "--jump" => changes.jump = Some(value),
            "--proxy" => changes.proxy = Some(value),
            "--group" => changes.group = Some(value),
            "--tag" => changes.tags.push(value),
            "--untag" => changes.untags.push(value),
            "--env" => changes.env.push(env(&value)?),
            "--unset-env" => changes.unset_env.push(value),
            "--note" => changes.note = Some(value),
            "--ai-notes" => changes.ai_notes = Some(value),
            "--password-stdin" => password_stdin = true,
            "--no-jump" => changes.no_jump = true,
            "--no-group" => changes.no_group = true,
            "--favorite" => changes.favorite = Some(true),
            "--no-favorite" => changes.favorite = Some(false),
            "--no-prompt" => prompt = false,
            "--json" => json = true,
            other => {
                return Err(format!(
                    "hosts {word} does not take {other}; put -- before an argument that starts \
                     with -"
                ));
            }
        }
        given.push(name);
    }
    let usage = match word.as_str() {
        "add" => "hatoba hosts add [options] [user@]<address>[:<port>]",
        "edit" => "hatoba hosts edit [options] <host>",
        _ => "hatoba hosts delete <host>",
    };
    let one = match (word.as_str(), positionals.len()) {
        (_, 1) => positionals.remove(0),
        ("add", 0) => return Err(format!("add needs a target: {usage}")),
        ("add", _) => return Err(format!("add takes one target: {usage}")),
        (_, 0) => return Err(format!("{word} needs a host: {usage}")),
        (_, _) => {
            return Err(format!(
                "{word} takes one host; put quotes around a name with spaces, such as \"{}\"",
                positionals.join(" ")
            ));
        }
    };
    if word == "delete" {
        if let Some(option) = given.first() {
            return Err(format!("delete does not take {option}: {usage}"));
        }
        return Ok(Command::HostDelete { host: one });
    }
    for (a, b) in [
        ("--jump", "--no-jump"),
        ("--group", "--no-group"),
        ("--favorite", "--no-favorite"),
    ] {
        if given.iter().any(|g| g == a) && given.iter().any(|g| g == b) {
            return Err(format!("{a} and {b} can't go together"));
        }
    }
    let implied = match (changes.key.is_some(), password_stdin) {
        (true, true) => {
            return Err(
                "--key and --password-stdin can't go together: a host signs in with a key or \
                 with a password"
                    .into(),
            );
        }
        (true, false) => Some((AuthKind::Key, "--key", "a key")),
        (false, true) => Some((AuthKind::Password, "--password-stdin", "a password")),
        (false, false) => None,
    };
    match (changes.auth, implied) {
        (Some(auth), Some((kind, option, what))) if auth != kind => {
            return Err(format!(
                "{option} signs in with {what}, so it can't go with --auth {}",
                auth_word(auth)
            ));
        }
        (_, Some((kind, ..))) => changes.auth = Some(kind),
        _ => {}
    }
    if changes.auth == Some(AuthKind::Password) && !password_stdin && !prompt {
        return Err(
            "--auth password with --no-prompt needs --password-stdin, since Hatoba can't ask for \
             the password"
                .into(),
        );
    }
    let change = |changes| HostChange {
        changes,
        password_stdin,
        prompt,
        json,
    };
    if word == "add" {
        if let Some(option) = given
            .iter()
            .find(|g| matches!(g.as_str(), "--address" | "--port" | "--user"))
        {
            return Err(format!(
                "add takes the address, port and user from its target, not {option}: {usage}"
            ));
        }
        let (username, address, port) = target(&one)?;
        changes.username = username;
        changes.address = Some(address);
        changes.port = port;
        return Ok(Command::HostAdd(change(changes)));
    }
    if changes == HostChanges::default() {
        return Err(format!(
            "edit needs an option that changes the host, such as --port 2222: {usage}"
        ));
    }
    Ok(Command::HostEdit {
        host: one,
        change: change(changes),
    })
}

/// The options of `hosts add` and `edit` that take no value.
const HOST_FLAGS: &[&str] = &[
    "--password-stdin",
    "--no-jump",
    "--no-group",
    "--favorite",
    "--no-favorite",
    "--no-prompt",
    "--json",
];

/// Whether `option` is one of `hosts add` and `edit`'s that take a value.
fn known_option(option: &str) -> bool {
    !needs(option).is_empty()
}

/// What the value of `option` is, for the message when it has none.
fn needs(option: &str) -> &'static str {
    match option {
        "--name" => "a name",
        "--address" => "an address",
        "--port" => "a port",
        "--user" => "a user name",
        "--auth" => "password, key, agent or ask",
        "--key" => "a key",
        "--jump" => "a host",
        "--proxy" => "a proxy, direct or default",
        "--group" => "a group",
        "--tag" | "--untag" => "a tag",
        "--env" => "NAME=value",
        "--unset-env" => "the name of a variable",
        "--note" | "--ai-notes" => "a text, or \"\" to clear it",
        _ => "",
    }
}

fn port(text: &str) -> Result<u16, String> {
    text.parse()
        .map_err(|_| format!("--port takes a port from 1 to 65535, not {text}"))
}

fn auth(text: &str) -> Result<AuthKind, String> {
    Ok(match text {
        "password" => AuthKind::Password,
        "key" => AuthKind::Key,
        "agent" => AuthKind::Agent,
        "ask" => AuthKind::Ask,
        _ => {
            return Err(format!(
                "--auth takes password, key, agent or ask, not {text}"
            ));
        }
    })
}

fn auth_word(auth: AuthKind) -> &'static str {
    match auth {
        AuthKind::Password => "password",
        AuthKind::Key => "key",
        AuthKind::Agent => "agent",
        AuthKind::Ask => "ask",
    }
}

fn env(text: &str) -> Result<EnvVar, String> {
    let (name, value) = text
        .split_once('=')
        .ok_or_else(|| format!("--env takes NAME=value, not {text}"))?;
    Ok(EnvVar {
        name: name.to_owned(),
        value: value.to_owned(),
    })
}

/// The user, address and port of `[user@]address[:port]`, `[user@][address]:port` for an IPv6
/// address, or either after `ssh://`, as quick connect reads them (HOST-12). The last `@` ends
/// the user, which may hold one itself, as with `ssh`, and an address with several `:` is IPv6.
fn target(text: &str) -> Result<(Option<String>, String, Option<u16>), String> {
    let invalid = || format!("{text} is not [user@]<address>[:<port>]");
    let rest = match text.get(..6) {
        Some(scheme) if scheme.eq_ignore_ascii_case("ssh://") => text[6..].trim_end_matches('/'),
        _ => text,
    };
    let (username, rest) = match rest.rsplit_once('@') {
        Some(("", _)) => return Err(invalid()),
        Some((user, rest)) => (Some(user.to_owned()), rest),
        None => (None, rest),
    };
    let (address, port) = if let Some(inside) = rest.strip_prefix('[') {
        let (address, after) = inside.split_once(']').ok_or_else(invalid)?;
        match after {
            "" => (address, None),
            _ => (address, Some(after.strip_prefix(':').ok_or_else(invalid)?)),
        }
    } else {
        match rest.split_once(':') {
            Some((address, port)) if !port.contains(':') => (address, Some(port)),
            _ => (rest, None),
        }
    };
    let port = port
        .map(|p| {
            p.parse::<u16>()
                .map_err(|_| format!("the port of {text} is not a number from 1 to 65535"))
        })
        .transpose()?;
    Ok((username, address.to_owned(), port))
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
    let mut mode = None;
    let mut paths = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--no-prompt" => prompt = false,
            "--sudo" => sudo = true,
            "--mode" => {
                mode = Some(octal(
                    args.next().ok_or("--mode needs a mode, such as 600")?,
                )?)
            }
            a if a.starts_with("--mode=") => mode = Some(octal(&a["--mode=".len()..])?),
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
        (Place::Remote { .. }, Place::Local(_)) if mode.is_some() => {
            Err("--mode is for a copy to a host".into())
        }
        (Place::Local(_), Place::Remote { .. }) | (Place::Remote { .. }, Place::Local(_)) => {
            Ok(Command::Copy {
                source,
                target,
                prompt,
                sudo,
                mode,
            })
        }
        _ => Err(
            "one of the source and the target has to be <host>:<path>, the other a local path"
                .into(),
        ),
    }
}

/// What `hatoba memory` does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MemoryAction {
    List,
    Save,
    Delete,
}

/// `list` (or nothing), `save` or `delete`, then the name, then for a save its text, which starts
/// at the first other argument after the name and takes the rest, joined with spaces. Options come
/// anywhere before the text. `--host` is needed: other programs get only a host's memories.
fn memory(args: Vec<String>) -> Result<Command, String> {
    use MemoryAction::{Delete, List, Save};
    let mut host = None;
    let mut json = false;
    let mut action = None;
    let mut name: Option<String> = None;
    let mut text = Vec::new();
    let mut options = true;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if options {
            match arg.as_str() {
                "--" => {
                    options = false;
                    continue;
                }
                "--host" => {
                    host = Some(args.next().ok_or("--host needs a host")?);
                    continue;
                }
                "--json" => {
                    json = true;
                    continue;
                }
                "-h" | "--help" => return Ok(Command::Help),
                a if a.starts_with("--host=") => {
                    host = Some(a["--host=".len()..].to_owned());
                    continue;
                }
                a if a.starts_with('-') && a != "-" => {
                    return Err(format!(
                        "memory does not take {a}; put -- before a name or a text that starts \
                         with -"
                    ));
                }
                _ => {}
            }
        }
        match action {
            None => {
                action = Some(match arg.as_str() {
                    "list" => List,
                    "save" => Save,
                    "delete" => Delete,
                    other => {
                        return Err(format!("memory takes list, save or delete, not {other}"));
                    }
                });
            }
            Some(List) => return Err(format!("memory list does not take {arg}")),
            Some(_) if name.is_none() => name = Some(arg),
            Some(Save) => {
                text.push(arg);
                text.extend(args.by_ref());
            }
            Some(Delete) => {
                return Err(format!(
                    "delete takes one name; put quotes around a name with spaces, such as \
                     \"{} {arg}\"",
                    name.unwrap_or_default()
                ));
            }
        }
    }
    if host.as_deref().is_some_and(|h: &str| h.trim().is_empty()) {
        return Err("--host needs a host".into());
    }
    let action = action.unwrap_or(List);
    if json && action != List {
        return Err("--json is for listing memories".into());
    }
    let (word, usage) = match action {
        List => ("memory", "hatoba memory --host <host> [--json]"),
        Save => ("save", "hatoba memory save --host <host> <name> [<text>]"),
        Delete => ("delete", "hatoba memory delete --host <host> <name>"),
    };
    let Some(host) = host else {
        return Err(format!("{word} needs --host <host>: {usage}"));
    };
    match (action, name) {
        (List, _) => Ok(Command::Memories { host, json }),
        (Save, Some(name)) => Ok(Command::MemorySave {
            host,
            name,
            content: (!text.is_empty()).then(|| text.join(" ")),
        }),
        (Delete, Some(name)) => Ok(Command::MemoryDelete { host, name }),
        (_, None) => Err(format!("{word} needs a name: {usage}")),
    }
}

/// A mode in octal, as `chmod` takes it: three or four digits, such as 600 or 0755.
fn octal(text: &str) -> Result<u32, String> {
    let valid = (3..=4).contains(&text.len()) && text.bytes().all(|b| (b'0'..=b'7').contains(&b));
    match u32::from_str_radix(text, 8) {
        Ok(mode) if valid => Ok(mode),
        _ => Err(format!(
            "--mode takes three or four octal digits, such as 600 or 0755, not {text}"
        )),
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
    fn a_misspelled_command_fails_instead_of_opening_hatoba() {
        for (word, command) in [
            ("exce", "exec"),
            ("exe", "exec"),
            ("exex", "exec"),
            ("eexec", "exec"),
            ("host", "hosts"),
            ("hsots", "hosts"),
            ("hlep", "help"),
            ("telent", "telnet"),
            ("memroy", "memory"),
            ("memor", "memory"),
        ] {
            assert_eq!(
                parse_words(&[word, "db", "ls"]),
                Err(format!("{word} is not a command. Did you mean {command}?"))
            );
        }
        // Host names that only look a little like a command.
        for word in ["db", "cpp", "ghost", "web1", "exec.example.com", "Help"] {
            assert!(
                matches!(parse_words(&[word]), Ok(Command::Open(_))),
                "{word}"
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
                mode: None,
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
                mode: None,
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
                mode: None,
            })
        );
        for words in [
            &["cp", "--mode", "600", "wg0.conf", "db:/etc/wireguard/"][..],
            &["cp", "wg0.conf", "--mode=0600", "db:/etc/wireguard/"],
        ] {
            assert!(
                matches!(
                    parse_words(words),
                    Ok(Command::Copy {
                        mode: Some(0o600),
                        ..
                    })
                ),
                "{words:?}"
            );
        }
        for words in [
            &["cp", "--mode", "rw", "a", "db:"][..],
            &["cp", "--mode", "800", "a", "db:"],
            &["cp", "--mode=60", "a", "db:"],
            &["cp", "--mode", "600", "db:a", "a"],
            &["cp", "a", "db:", "--mode"],
        ] {
            assert!(parse_words(words).is_err(), "{words:?}");
        }
        assert!(parse_words(&["cp", "-report.txt", "db:"]).is_err());
        assert!(parse_words(&["cp", "a", "b"]).is_err());
        assert!(parse_words(&["cp", "db:a", "web:b"]).is_err());
        assert!(parse_words(&["cp", "db:a"]).is_err());
        if cfg!(windows) {
            assert_eq!(place(r"C:\x\y"), Place::Local(r"C:\x\y".into()));
        }
    }

    #[test]
    fn memory_lists_saves_and_deletes_the_memories_of_a_host() {
        let list = |host: &str, json| {
            Ok(Command::Memories {
                host: host.into(),
                json,
            })
        };
        assert_eq!(
            parse_words(&["memory", "--host", "My Server"]),
            list("My Server", false)
        );
        assert_eq!(
            parse_words(&["memory", "--json", "--host=db", "list"]),
            list("db", true)
        );

        let save = |host: &str, name: &str, content: Option<&str>| {
            Ok(Command::MemorySave {
                host: host.into(),
                name: name.into(),
                content: content.map(Into::into),
            })
        };
        // Without a text, it comes from stdin.
        assert_eq!(
            parse_words(&["memory", "save", "--host", "db", "sudo"]),
            save("db", "sudo", None)
        );
        assert_eq!(
            parse_words(&["memory", "save", "--host", "db", "sudo use", "Uses doas."]),
            save("db", "sudo use", Some("Uses doas."))
        );
        // Options come anywhere before the text, which takes the rest.
        assert_eq!(
            parse_words(&[
                "memory",
                "--host",
                "db",
                "save",
                "logs",
                "--host=web",
                "in",
                "--json"
            ]),
            save("web", "logs", Some("in --json"))
        );
        assert_eq!(
            parse_words(&["memory", "save", "--host", "db", "--", "-x", "--host"]),
            save("db", "-x", Some("--host"))
        );

        assert_eq!(
            parse_words(&["memory", "delete", "sudo use", "--host", "db"]),
            Ok(Command::MemoryDelete {
                host: "db".into(),
                name: "sudo use".into(),
            })
        );

        // Without --host, nothing: the global memories are the assistant's own.
        assert_eq!(
            parse_words(&["memory"]),
            Err("memory needs --host <host>: hatoba memory --host <host> [--json]".into())
        );
        assert_eq!(
            parse_words(&["memory", "save", "sudo", "Uses doas."]),
            Err(
                "save needs --host <host>: hatoba memory save --host <host> <name> [<text>]".into()
            )
        );
        for words in [
            &["memory", "list", "--json"][..],
            &["memory", "save", "sudo"],
            &["memory", "save", "--", "-x", "--host"],
            &["memory", "delete", "sudo"],
            &["memory", "sudo"],
            &["memory", "list", "sudo"],
            &["memory", "save"],
            &["memory", "save", "--host", "db"],
            &["memory", "save", "--json", "--host", "db", "sudo", "x"],
            &["memory", "save", "-x", "y"],
            &["memory", "delete", "--host", "db"],
            &["memory", "delete", "--host", "db", "sudo", "use"],
            &["memory", "delete", "--json", "--host", "db", "sudo"],
            &["memory", "--host"],
            &["memory", "--host", " "],
            &["memory", "--host="],
        ] {
            assert!(parse_words(words).is_err(), "{words:?}");
        }
        assert_eq!(
            parse_words(&["memory", "delete", "--host", "db"]),
            Err("delete needs a name: hatoba memory delete --host <host> <name>".into())
        );
        assert_eq!(
            parse_words(&["memory", "save", "--help"]),
            Ok(Command::Help)
        );
    }

    fn change(changes: HostChanges) -> HostChange {
        HostChange {
            changes,
            password_stdin: false,
            prompt: true,
            json: false,
        }
    }

    #[test]
    fn hosts_add_takes_a_target_and_the_host_editors_fields() {
        assert_eq!(
            parse_words(&[
                "hosts",
                "add",
                "deploy@10.0.0.7",
                "--name",
                "web-3",
                "--key",
                "ci",
                "--jump",
                "bastion",
                "--group",
                "prod",
                "--tag",
                "tokyo",
                "--tag=web",
            ]),
            Ok(Command::HostAdd(change(HostChanges {
                name: Some("web-3".into()),
                address: Some("10.0.0.7".into()),
                username: Some("deploy".into()),
                auth: Some(AuthKind::Key),
                key: Some("ci".into()),
                jump: Some("bastion".into()),
                group: Some("prod".into()),
                tags: vec!["tokyo".into(), "web".into()],
                ..HostChanges::default()
            })))
        );
        assert_eq!(
            parse_words(&[
                "hosts",
                "add",
                "--password-stdin",
                "root@nas.local",
                "--json"
            ]),
            Ok(Command::HostAdd(HostChange {
                changes: HostChanges {
                    address: Some("nas.local".into()),
                    username: Some("root".into()),
                    auth: Some(AuthKind::Password),
                    ..HostChanges::default()
                },
                password_stdin: true,
                prompt: true,
                json: true,
            }))
        );
        // As quick connect reads a target: the last @ ends the user, and an IPv6 address with a
        // port goes in brackets.
        for (word, user, address, port) in [
            ("nas.local", None, "nas.local", None),
            ("db:2222", None, "db", Some(2222)),
            ("a@corp@db", Some("a@corp"), "db", None),
            ("ops@[fe80::1]:2200", Some("ops"), "fe80::1", Some(2200)),
            ("fe80::1", None, "fe80::1", None),
            ("SSH://root@db:22/", Some("root"), "db", Some(22)),
            // The host editor refuses port 0, with its own message.
            ("db:0", None, "db", Some(0)),
        ] {
            let Ok(Command::HostAdd(HostChange { changes, .. })) =
                parse_words(&["hosts", "add", word])
            else {
                panic!("{word}");
            };
            assert_eq!(
                (
                    changes.username.as_deref(),
                    changes.address.as_deref(),
                    changes.port
                ),
                (user, Some(address), port),
                "{word}"
            );
        }
        for words in [
            &["hosts", "add"][..],
            &["hosts", "add", "db", "web"],
            &["hosts", "add", "@db"],
            &["hosts", "add", "db:http"],
            &["hosts", "add", "db:70000"],
            &["hosts", "add", "[fe80::1"],
            &["hosts", "add", "[fe80::1]2200"],
            &["hosts", "add", "db", "--port", "2222"],
            &["hosts", "add", "db", "--user", "root"],
            &["hosts", "add", "db", "--name"],
            &["hosts", "add", "db", "--json=yes"],
            &["hosts", "add", "db", "--colour", "red"],
            &["hosts", "add", "db", "--auth", "token"],
            &["hosts", "add", "db", "--env", "TZ"],
            &["hosts", "add", "db", "--jump", "b", "--no-jump"],
            &["hosts", "add", "db", "--favorite", "--no-favorite"],
            &["hosts", "add", "db", "--key", "ci", "--password-stdin"],
            &["hosts", "add", "db", "--key", "ci", "--auth", "agent"],
            &["hosts", "add", "db", "--password-stdin", "--auth", "ask"],
            // Hatoba can't ask for the password.
            &["hosts", "add", "db", "--auth", "password", "--no-prompt"],
        ] {
            assert!(parse_words(words).is_err(), "{words:?}");
        }
        assert!(matches!(
            parse_words(&[
                "hosts",
                "add",
                "db",
                "--auth",
                "password",
                "--no-prompt",
                "--password-stdin"
            ]),
            Ok(Command::HostAdd(HostChange {
                prompt: false,
                password_stdin: true,
                ..
            }))
        ));
        assert_eq!(parse_words(&["hosts", "add", "--help"]), Ok(Command::Help));
    }

    #[test]
    fn hosts_edit_changes_what_its_options_name() {
        assert_eq!(
            parse_words(&[
                "hosts",
                "edit",
                "web-3",
                "--port",
                "2222",
                "--env",
                "TZ=Asia/Tokyo",
                "--env=EMPTY=",
                "--unset-env",
                "LANG",
                "--ai-notes",
                "Staging; restart with systemctl",
                "--note=",
                "--untag",
                "tokyo",
                "--no-group",
                "--no-jump",
                "--proxy",
                "direct",
                "--no-favorite",
            ]),
            Ok(Command::HostEdit {
                host: "web-3".into(),
                change: change(HostChanges {
                    port: Some(2222),
                    env: vec![
                        EnvVar {
                            name: "TZ".into(),
                            value: "Asia/Tokyo".into()
                        },
                        EnvVar {
                            name: "EMPTY".into(),
                            value: String::new()
                        },
                    ],
                    unset_env: vec!["LANG".into()],
                    ai_notes: Some("Staging; restart with systemctl".into()),
                    note: Some(String::new()),
                    untags: vec!["tokyo".into()],
                    no_group: true,
                    no_jump: true,
                    proxy: Some("direct".into()),
                    favorite: Some(false),
                    ..HostChanges::default()
                }),
            })
        );
        assert_eq!(
            parse_words(&[
                "hosts",
                "edit",
                "--address",
                "10.0.0.8",
                "--user",
                "ops",
                "--",
                "-web"
            ]),
            Ok(Command::HostEdit {
                host: "-web".into(),
                change: change(HostChanges {
                    address: Some("10.0.0.8".into()),
                    username: Some("ops".into()),
                    ..HostChanges::default()
                }),
            })
        );
        // An edit with --auth password asks for the password in the window.
        assert!(matches!(
            parse_words(&["hosts", "edit", "nas", "--auth", "password"]),
            Ok(Command::HostEdit {
                change: HostChange {
                    password_stdin: false,
                    prompt: true,
                    ..
                },
                ..
            })
        ));
        for words in [
            &["hosts", "edit", "web-3"][..],
            &["hosts", "edit", "web-3", "--json"],
            &["hosts", "edit", "--port", "22"],
            &["hosts", "edit", "My", "Server", "--port", "22"],
            &["hosts", "edit", "web-3", "--port", "-1"],
            &["hosts", "edit", "web-3", "--group", "a", "--no-group"],
        ] {
            assert!(parse_words(words).is_err(), "{words:?}");
        }
    }

    #[test]
    fn hosts_delete_takes_only_the_host() {
        assert_eq!(
            parse_words(&["hosts", "delete", "web-3"]),
            Ok(Command::HostDelete {
                host: "web-3".into()
            })
        );
        for words in [
            &["hosts", "delete"][..],
            &["hosts", "delete", "web-3", "--json"],
            &["hosts", "delete", "web-3", "--no-prompt"],
            &["hosts", "delete", "web-3", "db"],
        ] {
            assert!(parse_words(words).is_err(), "{words:?}");
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
