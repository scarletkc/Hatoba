//! Running as root with `sudo` (`hatoba exec --sudo` and `hatoba cp --sudo`, HOST-15).
//!
//! [`access`] tells whether the login user is root, whether sudo runs commands without a password,
//! or whether it asks for one, which [`check`] tries. A command then runs as `sudo -S -k`, with the
//! password as the first line of its stdin: sudo reads it one byte at a time up to the line break,
//! and `-k` makes it ask even when it still remembers an earlier password, so it always takes that
//! line and what follows reaches the script it runs. The password is in no command line, file or
//! environment variable, and the empty prompt (`-p ""`) keeps sudo from printing one.
//!
//! What runs as root is a fixed `sh -c` script, one line without single quotes, backslashes or
//! `!`, like those of [`crate::remote_file`]: [`crate::remote_command`]'s, so a stop still ends the
//! command and every process it started, or [`INSTALL_SCRIPT`] and [`FETCH_SCRIPT`] for a copy,
//! which goes through a folder of the login user's that SFTP reads or writes ([`temp_dir`]).

use zeroize::Zeroizing;

use crate::error::SshError;
use crate::exec::Exec;
use crate::remote_file::{RemoteFileError, command, last_line};
use crate::session::ExecOutput;

/// The line the scripts print before what they report, so that anything the login shell's startup
/// files print is told apart from it. The copy scripts also take it as their first line of stdin,
/// which tells them that sudo took the password before it.
const MARKER: &str = "hatoba-sudo";

/// Prints the marker and `root` when the login user is root, `free` when sudo runs a command
/// without asking for a password, `password` when it asks for one, and `none` without sudo.
const ACCESS_SCRIPT: &str = "if [ \"$(id -u)\" = 0 ]; then echo hatoba-sudo root; \
     elif command -v sudo >/dev/null 2>&1; then \
     if sudo -n true >/dev/null 2>&1; then echo hatoba-sudo free; \
     else echo hatoba-sudo password; fi; else echo hatoba-sudo none; fi";

/// Prints the marker, then has sudo run `true` with the password on stdin, in the C locale, so its
/// messages can be read.
const CHECK_SCRIPT: &str = "echo hatoba-sudo; LC_ALL=C sudo -S -k -p \"\" -- true";

/// Makes a folder that only the login user can open, with an empty file `f` in it, and prints the
/// marker and the folder's path.
const TEMP_SCRIPT: &str =
    "d=$(mktemp -d) || exit 1; : > \"$d/f\" || exit 1; echo hatoba-sudo; printf %s \"$d\"; echo";

/// Removes the folder named on stdin.
const REMOVE_SCRIPT: &str = "IFS= read -r d || exit 9; rm -rf -- \"$d\"";

/// As root: copies the file on stdin's second line into the one on its third, or into a folder
/// there, under the name on its fourth line, as `cp` would: in place, so a file already there keeps
/// its owner, mode and links, while a new one belongs to root with root's umask. Stdin's first line
/// is the marker. Exit statuses: 2 no such directory, 3 a directory in the file's place, 6 not a
/// regular file, 8 a symbolic link to nothing, 9 no marker or path; 1 when the copy fails, with the
/// message on stderr.
const INSTALL_SCRIPT: &str = "IFS= read -r x; [ \"$x\" = hatoba-sudo ] || exit 9; \
     echo hatoba-sudo; IFS= read -r t || exit 9; IFS= read -r f || exit 9; \
     IFS= read -r b || exit 9; [ -d \"$f\" ] && f=${f%/}/$b; \
     [ -L \"$f\" ] && { [ -e \"$f\" ] || exit 8; }; \
     if [ -e \"$f\" ]; then [ -d \"$f\" ] && exit 3; [ -f \"$f\" ] || exit 6; \
     else [ -d \"$(dirname -- \"$f\")\" ] || exit 2; fi; cat -- \"$t\" > \"$f\" || exit 1";

/// As root: copies the file on stdin's second line into the login user's file on its third, in
/// place, so the copy stays the user's. Stdin's first line is the marker. Exit statuses: 2 no such
/// file, 3 a directory, 6 not a regular file, 9 no marker or path; 1 when the copy fails, with the
/// message on stderr.
const FETCH_SCRIPT: &str = "IFS= read -r x; [ \"$x\" = hatoba-sudo ] || exit 9; \
     echo hatoba-sudo; IFS= read -r f || exit 9; IFS= read -r t || exit 9; \
     [ -d \"$f\" ] && exit 3; [ -e \"$f\" ] || exit 2; [ -f \"$f\" ] || exit 6; \
     cat -- \"$f\" > \"$t\" || exit 1";

/// How a host runs a command as root.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// The login user is root.
    Root,
    /// sudo runs it without asking for a password.
    NoPassword,
    /// sudo asks for a password.
    Password,
}

/// Why [`access`] could not tell.
#[derive(Debug)]
pub enum AccessError {
    /// The login user has no `sudo` on its `PATH`.
    NoSudo,
    /// The host did not run the check: the last line of its stderr, empty when there is none.
    Failed(String),
    Ssh(SshError),
}

/// What sudo made of a password.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Checked {
    Accepted,
    /// The password is wrong.
    Wrong,
    /// sudo refused for another reason, such as a user it does not let run commands, or a terminal
    /// it wants: the last line of its stderr.
    Refused(String),
}

/// Who a command runs as.
pub enum RunAs {
    /// The login user, as sshd runs it.
    Login,
    /// Root, through sudo, with the password it asks for, or `None` when it asks for none.
    Sudo(Option<Zeroizing<String>>),
}

impl RunAs {
    pub fn is_sudo(&self) -> bool {
        matches!(self, Self::Sudo(_))
    }

    /// What goes before a command line: `sudo` and its options, or nothing.
    pub(crate) fn prefix(&self) -> &'static str {
        match self {
            Self::Login => "",
            Self::Sudo(None) => "sudo -n -- ",
            Self::Sudo(Some(_)) => "sudo -S -k -p \"\" -- ",
        }
    }

    /// What goes on stdin before anything else: the password and a line break.
    pub(crate) fn password_line(&self) -> Result<Option<Vec<u8>>, SshError> {
        match self {
            Self::Sudo(Some(password)) => password_line(password)
                .map(Some)
                .ok_or_else(|| SshError::other("the sudo password has a line break")),
            _ => Ok(None),
        }
    }
}

/// The password and a line break; `None` for one that sudo would read as more than one line.
fn password_line(password: &str) -> Option<Vec<u8>> {
    if password.contains(['\n', '\r', '\0']) {
        return None;
    }
    Some(format!("{password}\n").into_bytes())
}

/// The word after the last marker line of `stdout`.
fn reported(stdout: &[u8]) -> Option<String> {
    String::from_utf8_lossy(stdout)
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix(MARKER).map(|w| w.trim().to_owned()))
}

/// Whether the login user is root, and otherwise how sudo runs a command.
pub async fn access(session: &impl Exec) -> Result<Access, AccessError> {
    let out = session
        .exec_output(&command(ACCESS_SCRIPT, None), None)
        .await
        .map_err(AccessError::Ssh)?;
    parse_access(&out)
}

fn parse_access(out: &ExecOutput) -> Result<Access, AccessError> {
    match reported(&out.stdout).as_deref() {
        Some("root") => Ok(Access::Root),
        Some("free") => Ok(Access::NoPassword),
        Some("password") => Ok(Access::Password),
        Some("none") => Err(AccessError::NoSudo),
        _ => Err(AccessError::Failed(last_line(&out.stderr))),
    }
}

/// Has sudo run `true` with `password`, without the password it may remember.
pub async fn check(session: &impl Exec, password: &str) -> Result<Checked, SshError> {
    let Some(stdin) = password_line(password) else {
        return Ok(Checked::Wrong);
    };
    let out = session
        .exec_output(&command(CHECK_SCRIPT, None), Some(stdin))
        .await?;
    Ok(parse_check(&out))
}

fn parse_check(out: &ExecOutput) -> Checked {
    let stderr = String::from_utf8_lossy(&out.stderr);
    if reported(&out.stdout).is_none() {
        return Checked::Refused(last_line(&out.stderr));
    }
    if out.exit_status == Some(0) {
        Checked::Accepted
    } else if stderr.contains("incorrect password attempt") || stderr.contains("Sorry, try again") {
        Checked::Wrong
    } else {
        Checked::Refused(last_line(&out.stderr))
    }
}

/// Why sudo did not run a script, from what it printed.
pub(crate) fn refused(stderr: &[u8]) -> SshError {
    match last_line(stderr) {
        line if line.is_empty() => SshError::other("sudo did not run the command"),
        line => SshError::other(line),
    }
}

/// Makes a folder on the host that only the login user can open, for a copy that goes through it,
/// with an empty file in it ([`temp_file`]). [`remove_temp_dir`] removes it.
pub async fn temp_dir(session: &impl Exec) -> Result<String, SshError> {
    let out = session
        .exec_output(&command(TEMP_SCRIPT, None), None)
        .await?;
    parse_temp_dir(&out)
}

fn parse_temp_dir(out: &ExecOutput) -> Result<String, SshError> {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let mut lines = stdout.lines();
    let dir = lines
        .by_ref()
        .find(|l| *l == MARKER)
        .and_then(|_| lines.next())
        .filter(|d| d.starts_with('/'));
    match dir {
        Some(dir) if out.exit_status == Some(0) => Ok(dir.to_owned()),
        _ => Err(SshError::other(format!(
            "can't make a folder for the copy with mktemp: {}",
            last_line(&out.stderr)
        ))),
    }
}

/// The file in a folder of [`temp_dir`].
pub fn temp_file(dir: &str) -> String {
    format!("{dir}/f")
}

/// Removes a folder of [`temp_dir`] and what it holds.
pub async fn remove_temp_dir(session: &impl Exec, dir: &str) -> Result<(), SshError> {
    let out = session
        .exec_output(
            &command(REMOVE_SCRIPT, None),
            Some(format!("{dir}\n").into_bytes()),
        )
        .await?;
    match out.exit_status {
        Some(0) => Ok(()),
        _ => Err(SshError::other(last_line(&out.stderr))),
    }
}

/// As `run_as`, copies the file `from` into the file `to`, absolute, or into the folder `to` under
/// `name`, as `cp` would: a file already there keeps its owner, mode and links.
pub async fn install(
    session: &impl Exec,
    run_as: &RunAs,
    from: &str,
    to: &str,
    name: &str,
) -> Result<(), RemoteFileError> {
    let stdin = request(run_as, &[from, to, name])?;
    let out = session
        .exec_output(&root_command(run_as, INSTALL_SCRIPT), Some(stdin))
        .await
        .map_err(RemoteFileError::Ssh)?;
    parse_copy(&out, RemoteFileError::NoDirectory)
}

/// As `run_as`, copies the file `from`, absolute, into the login user's file `to`.
pub async fn fetch(
    session: &impl Exec,
    run_as: &RunAs,
    from: &str,
    to: &str,
) -> Result<(), RemoteFileError> {
    let stdin = request(run_as, &[from, to])?;
    let out = session
        .exec_output(&root_command(run_as, FETCH_SCRIPT), Some(stdin))
        .await
        .map_err(RemoteFileError::Ssh)?;
    parse_copy(&out, RemoteFileError::NotFound)
}

fn root_command(run_as: &RunAs, script: &str) -> String {
    format!("{}{}", run_as.prefix(), command(script, None))
}

/// The password line, the marker, then a line for each of `lines`.
fn request(run_as: &RunAs, lines: &[&str]) -> Result<Vec<u8>, RemoteFileError> {
    if lines
        .iter()
        .any(|l| l.is_empty() || l.contains(['\n', '\r', '\0']))
    {
        return Err(RemoteFileError::InvalidPath);
    }
    let mut stdin = run_as
        .password_line()
        .map_err(RemoteFileError::Ssh)?
        .unwrap_or_default();
    stdin.extend_from_slice(format!("{MARKER}\n{}\n", lines.join("\n")).as_bytes());
    Ok(stdin)
}

/// What a copy script's exit status says; `missing` is its 2.
fn parse_copy(out: &ExecOutput, missing: RemoteFileError) -> Result<(), RemoteFileError> {
    if reported(&out.stdout).is_none() {
        return Err(RemoteFileError::Ssh(refused(&out.stderr)));
    }
    match out.exit_status {
        Some(0) => Ok(()),
        Some(2) => Err(missing),
        Some(3) => Err(RemoteFileError::IsDirectory),
        Some(6) => Err(RemoteFileError::NotRegular),
        Some(8) => Err(RemoteFileError::DanglingLink),
        status => Err(RemoteFileError::Failed {
            status,
            stderr: last_line(&out.stderr),
        }),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn output(status: Option<u32>, stdout: &str, stderr: &str) -> ExecOutput {
        ExecOutput {
            exit_status: status,
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    pub(crate) fn sudo_with(password: &str) -> RunAs {
        RunAs::Sudo(Some(Zeroizing::new(password.to_owned())))
    }

    #[test]
    fn scripts_survive_any_login_shell() {
        for script in [
            ACCESS_SCRIPT,
            CHECK_SCRIPT,
            TEMP_SCRIPT,
            REMOVE_SCRIPT,
            INSTALL_SCRIPT,
            FETCH_SCRIPT,
        ] {
            // `!` too: csh expands it even inside single quotes when its history is on.
            assert!(!script.contains(['\'', '\\', '\n', '!']), "{script}");
        }
        for run_as in [RunAs::Login, RunAs::Sudo(None), sudo_with("x")] {
            assert!(!run_as.prefix().contains(['\'', '\\', '!']));
        }
    }

    #[test]
    fn the_password_goes_first_on_stdin() {
        assert_eq!(
            request(&sudo_with("p w"), &["/tmp/a/f", "/etc/x"]).unwrap(),
            b"p w\nhatoba-sudo\n/tmp/a/f\n/etc/x\n"
        );
        assert_eq!(
            request(&RunAs::Sudo(None), &["/a", "/b"]).unwrap(),
            b"hatoba-sudo\n/a\n/b\n"
        );
        assert!(matches!(
            request(&RunAs::Login, &["/a", "b\nc"]),
            Err(RemoteFileError::InvalidPath)
        ));
        assert!(matches!(
            request(&sudo_with("a\rb"), &["/a", "/b"]),
            Err(RemoteFileError::Ssh(_))
        ));
        assert!(sudo_with("x").prefix().starts_with("sudo -S -k "));
        assert_eq!(RunAs::Sudo(None).prefix(), "sudo -n -- ");
    }

    #[test]
    fn access_and_check_read_the_report() {
        let access = |stdout: &str| parse_access(&output(Some(0), stdout, "")).ok();
        assert_eq!(access("motd\nhatoba-sudo free\n"), Some(Access::NoPassword));
        assert_eq!(access("hatoba-sudo root\n"), Some(Access::Root));
        assert_eq!(access("hatoba-sudo password\n"), Some(Access::Password));
        assert!(matches!(
            parse_access(&output(Some(0), "hatoba-sudo none\n", "")),
            Err(AccessError::NoSudo)
        ));
        assert!(matches!(
            parse_access(&output(Some(127), "", "sh: not found\n")),
            Err(AccessError::Failed(m)) if m == "sh: not found"
        ));

        let check = |status, stderr: &str| parse_check(&output(status, "hatoba-sudo\n", stderr));
        assert_eq!(check(Some(0), ""), Checked::Accepted);
        assert_eq!(
            check(
                Some(1),
                "Sorry, try again.\nsudo: 1 incorrect password attempt\n"
            ),
            Checked::Wrong
        );
        assert_eq!(
            check(Some(1), "kc is not in the sudoers file.\n"),
            Checked::Refused("kc is not in the sudoers file.".into())
        );
        assert_eq!(
            check(Some(1), "sudo: sorry, you must have a tty to run sudo\n"),
            Checked::Refused("sudo: sorry, you must have a tty to run sudo".into())
        );
    }

    #[test]
    fn a_copy_without_the_marker_is_sudos_refusal() {
        let refusal = parse_copy(
            &output(Some(1), "", "sudo: a password is required\n"),
            RemoteFileError::NotFound,
        );
        assert!(
            matches!(refusal, Err(RemoteFileError::Ssh(e)) if e.message == "sudo: a password is required")
        );
        assert!(matches!(
            parse_copy(
                &output(Some(2), "hatoba-sudo\n", ""),
                RemoteFileError::NotFound
            ),
            Err(RemoteFileError::NotFound)
        ));
        assert!(matches!(
            parse_copy(
                &output(Some(2), "hatoba-sudo\n", ""),
                RemoteFileError::NoDirectory
            ),
            Err(RemoteFileError::NoDirectory)
        ));
        assert!(
            parse_copy(
                &output(Some(0), "hatoba-sudo\n", ""),
                RemoteFileError::NotFound
            )
            .is_ok()
        );
        assert_eq!(
            parse_temp_dir(&output(Some(0), "motd\nhatoba-sudo\n/tmp/tmp.Ab12\n", "")).unwrap(),
            "/tmp/tmp.Ab12"
        );
        assert!(parse_temp_dir(&output(Some(1), "", "mktemp: failed\n")).is_err());
    }

    /// A `sudo` of the tests' own, which takes the password `secret` with `-S`, asks for none with
    /// `-n` while `SUDO_FREE` is set, and runs the command with a `HOME` and `SHELL` of its own, as
    /// sudo gives it root's.
    #[cfg(unix)]
    pub(crate) mod fake {
        use std::os::unix::fs::PermissionsExt;

        const SUDO: &str = "#!/bin/sh
p=0
while [ $# -gt 0 ]; do
  case $1 in
    -S) p=1 ;;
    -n|-k) ;;
    -p) shift ;;
    --) shift; break ;;
    *) break ;;
  esac
  shift
done
if [ $p = 1 ]; then
  IFS= read -r w
  if [ \"$w\" != secret ]; then
    echo 'Sorry, try again.' >&2
    echo 'sudo: 1 incorrect password attempt' >&2
    exit 1
  fi
elif [ -z \"$SUDO_FREE\" ]; then
  echo 'sudo: a password is required' >&2
  exit 1
fi
exec env HOME=/nonexistent-root SHELL=/bin/false \"$@\"
";

        /// A directory with the fake `sudo`, and a `PATH` with it first.
        pub(crate) fn sudo_dir() -> (tempfile::TempDir, String) {
            let dir = tempfile::tempdir().unwrap();
            let sudo = dir.path().join("sudo");
            std::fs::write(&sudo, SUDO).unwrap();
            std::fs::set_permissions(&sudo, std::fs::Permissions::from_mode(0o755)).unwrap();
            let path = format!(
                "{}:{}",
                dir.path().display(),
                std::env::var("PATH").unwrap_or_default()
            );
            (dir, path)
        }
    }

    /// The scripts with the local `sh` and the fake `sudo`.
    #[cfg(unix)]
    mod local_sh {
        use super::fake::sudo_dir;
        use super::*;
        use crate::test_sh::{is_root, run_with};

        fn text(path: &std::path::Path) -> &str {
            path.to_str().unwrap()
        }

        #[test]
        fn access_tells_how_sudo_runs_a_command() {
            let home = tempfile::tempdir().unwrap();
            let (_sudo, path) = sudo_dir();
            let access = |vars: &[(&str, &str)]| {
                parse_access(&run_with(
                    home.path(),
                    &command(ACCESS_SCRIPT, None),
                    b"",
                    vars,
                ))
            };
            if is_root() {
                assert!(matches!(access(&[("PATH", &path)]), Ok(Access::Root)));
                return;
            }
            assert!(matches!(access(&[("PATH", &path)]), Ok(Access::Password)));
            assert!(matches!(
                access(&[("PATH", &path), ("SUDO_FREE", "1")]),
                Ok(Access::NoPassword)
            ));
            // A PATH without sudo, with what the script needs.
            let tools = tempfile::tempdir().unwrap();
            for tool in ["id", "sh"] {
                let out = std::process::Command::new("sh")
                    .args(["-c", &format!("command -v {tool}")])
                    .output()
                    .unwrap();
                let target = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                std::os::unix::fs::symlink(target, tools.path().join(tool)).unwrap();
            }
            assert!(matches!(
                access(&[("PATH", text(tools.path()))]),
                Err(AccessError::NoSudo)
            ));
        }

        #[test]
        fn check_tells_a_wrong_password() {
            let home = tempfile::tempdir().unwrap();
            let (_sudo, path) = sudo_dir();
            let check = |password: &str| {
                parse_check(&run_with(
                    home.path(),
                    &command(CHECK_SCRIPT, None),
                    &password_line(password).unwrap(),
                    &[("PATH", &path)],
                ))
            };
            assert_eq!(check("secret"), Checked::Accepted);
            assert_eq!(check("guess"), Checked::Wrong);
        }

        #[test]
        fn a_copy_goes_through_the_users_folder() {
            let home = tempfile::tempdir().unwrap();
            let home_path = home.path().canonicalize().unwrap();
            let (_sudo, path) = sudo_dir();
            let vars = [("PATH", path.as_str()), ("TMPDIR", text(&home_path))];
            let sh = |command: &str, stdin: &[u8]| run_with(&home_path, command, stdin, &vars);

            let dir = parse_temp_dir(&sh(&command(TEMP_SCRIPT, None), b"")).unwrap();
            let file = temp_file(&dir);
            assert_eq!(std::fs::read(&file).unwrap(), b"");
            std::fs::write(&file, b"new").unwrap();

            let target = home_path.join("conf");
            std::fs::write(&target, b"old").unwrap();
            let run_as = sudo_with("secret");
            let to = text(&target);
            let install = |to: &str, name: &str| {
                parse_copy(
                    &sh(
                        &root_command(&run_as, INSTALL_SCRIPT),
                        &request(&run_as, &[&file, to, name]).unwrap(),
                    ),
                    RemoteFileError::NoDirectory,
                )
            };
            install(to, "conf").unwrap();
            assert_eq!(std::fs::read(&target).unwrap(), b"new");
            // Into a folder, under the name.
            install(text(&home_path), "other").unwrap();
            assert_eq!(std::fs::read(home_path.join("other")).unwrap(), b"new");
            let missing = format!("{}/missing/x", text(&home_path));
            assert!(matches!(
                install(&missing, "x"),
                Err(RemoteFileError::NoDirectory)
            ));
            // A wrong password writes nothing.
            let wrong = sudo_with("guess");
            std::fs::write(&target, b"old").unwrap();
            let refused = parse_copy(
                &sh(
                    &root_command(&wrong, INSTALL_SCRIPT),
                    &request(&wrong, &[&file, to, "conf"]).unwrap(),
                ),
                RemoteFileError::NoDirectory,
            );
            assert!(matches!(refused, Err(RemoteFileError::Ssh(_))));
            assert_eq!(std::fs::read(&target).unwrap(), b"old");

            std::fs::write(&target, b"secret config").unwrap();
            let fetch = |from: &str| {
                parse_copy(
                    &sh(
                        &root_command(&run_as, FETCH_SCRIPT),
                        &request(&run_as, &[from, &file]).unwrap(),
                    ),
                    RemoteFileError::NotFound,
                )
            };
            fetch(to).unwrap();
            assert_eq!(std::fs::read(&file).unwrap(), b"secret config");
            assert!(matches!(fetch(&missing), Err(RemoteFileError::NotFound)));
            assert!(matches!(
                fetch(text(&home_path)),
                Err(RemoteFileError::IsDirectory)
            ));

            let out = sh(&command(REMOVE_SCRIPT, None), format!("{dir}\n").as_bytes());
            assert_eq!(out.exit_status, Some(0));
            assert!(!std::path::Path::new(&dir).exists());
        }
    }
}
