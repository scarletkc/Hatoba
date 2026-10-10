//! Reading and writing a file on the host over an exec channel (spec §13.4, "File tools"; the
//! SFTP panel's editor, SFTP-05, is meant to use it too).
//!
//! Each operation runs a fixed `sh -c` script. The path, and for a write the expected state and
//! the content, travel on stdin, so nothing the caller passes goes through the quoting of the
//! user's login shell, which may be bash, zsh, fish or csh. The scripts are one line without
//! single quotes or backslashes for the same reason. A write pipes into the file
//! (`cat > path`), so an existing file keeps its inode, owner, mode and the links to it.

use std::fmt;

use crate::error::SshError;
use crate::session::{ExecOutput, SshSession};

/// The line the read script prints before the file, so that anything the login shell's startup
/// files print to stdout is told apart from the content.
const MARKER: &[u8] = b"hatoba-file\n";

/// Reads the file named on stdin, `$1` bytes at most. Exit statuses: 2 missing, 3 directory,
/// 4 not readable, 5 larger than `$1` (its size on stderr), 6 not a regular file, 8 a symbolic
/// link to nothing, 9 no path.
const READ_SCRIPT: &str = "IFS= read -r f || exit 9; \
     case $f in \"~\") f=$HOME;; \"~/\"*) f=$HOME/${f#\"~/\"};; esac; \
     [ -d \"$f\" ] && exit 3; [ -L \"$f\" ] && { [ -e \"$f\" ] || exit 8; }; \
     [ -e \"$f\" ] || exit 2; [ -f \"$f\" ] || exit 6; \
     [ -r \"$f\" ] || exit 4; n=$(wc -c < \"$f\") || exit 4; n=$(echo $n); \
     [ \"$n\" -le \"$1\" ] || { echo \"$n\" >&2; exit 5; }; \
     echo hatoba-file; exec cat -- \"$f\"";

/// Writes the rest of stdin to the file named on its first line, after checking the state on
/// its second line: `any`, `missing`, or the file's `cksum` output (`crc size`). Exit statuses:
/// 2 no such directory, 3 directory, 4 not writable, 6 not a regular file, 7 not in the
/// expected state, 8 a symbolic link to nothing (writing would create its target, wherever that
/// is), 9 no path.
///
/// The check and the write run in one command, so a change made since the caller read the file
/// is caught. A write by another process in the moment between the check and `cat` is not: an
/// in-place rewrite has no lock that every writer honors.
const WRITE_SCRIPT: &str = "IFS= read -r f || exit 9; IFS= read -r g || exit 9; \
     case $f in \"~\") f=$HOME;; \"~/\"*) f=$HOME/${f#\"~/\"};; esac; \
     [ -d \"$f\" ] && exit 3; [ -L \"$f\" ] && { [ -e \"$f\" ] || exit 8; }; \
     if [ -e \"$f\" ]; then [ -f \"$f\" ] || exit 6; [ \"$g\" = missing ] && exit 7; \
     case $g in any) ;; *) c=$(cksum < \"$f\") || exit 4; set -- $c; \
     [ \"$1 $2\" = \"$g\" ] || exit 7;; esac; [ -w \"$f\" ] || exit 4; \
     else [ \"$g\" = any ] || [ \"$g\" = missing ] || exit 7; d=$(dirname -- \"$f\"); \
     [ -d \"$d\" ] || exit 2; [ -w \"$d\" ] || exit 4; fi; \
     cat > \"$f\"";

/// What a write checks right before it writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect<'a> {
    /// Nothing: whatever the file holds is replaced.
    Any,
    /// The file must not exist.
    Missing,
    /// The file must hold these bytes, compared by POSIX `cksum` checksum and size.
    Content(&'a [u8]),
}

/// Why a read or write did not happen.
#[derive(Debug)]
pub enum RemoteFileError {
    /// The path is empty or has a line break or NUL.
    InvalidPath,
    /// Read: the file does not exist.
    NotFound,
    /// Write: the directory the file would be created in does not exist.
    NoDirectory,
    /// The path is a directory.
    IsDirectory,
    /// The path is a device, FIFO, socket or another file that is not a regular file.
    NotRegular,
    /// The path is a symbolic link whose target does not exist.
    DanglingLink,
    /// The user cannot read (or write) the file, or create it in its directory.
    PermissionDenied,
    /// Read: the file has this many bytes, more than the limit.
    TooLarge(u64),
    /// Write: the file is not in the state [`Expect`] named, so nothing was written.
    Changed,
    /// The command failed otherwise: its exit status and the last line of its stderr.
    Failed {
        /// `None` when the command ended without one (a signal).
        status: Option<u32>,
        /// The last non-empty line of stderr, empty when there is none.
        stderr: String,
    },
    /// The channel could not run the command.
    Ssh(SshError),
}

impl fmt::Display for RemoteFileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPath => f.write_str("the path is empty or has a line break"),
            Self::NotFound => f.write_str("no such file"),
            Self::NoDirectory => f.write_str("no such directory"),
            Self::IsDirectory => f.write_str("is a directory"),
            Self::NotRegular => f.write_str("not a regular file"),
            Self::DanglingLink => f.write_str("a symbolic link to a file that does not exist"),
            Self::PermissionDenied => f.write_str("permission denied"),
            Self::TooLarge(size) => write!(f, "the file is too large ({size} bytes)"),
            Self::Changed => f.write_str("the file changed"),
            Self::Failed { status, stderr } => {
                match status {
                    Some(code) => write!(f, "the command exited with status {code}")?,
                    None => f.write_str("the command ended without an exit status")?,
                }
                if stderr.is_empty() {
                    Ok(())
                } else {
                    write!(f, ": {stderr}")
                }
            }
            Self::Ssh(e) => write!(f, "{}", e.message),
        }
    }
}

impl std::error::Error for RemoteFileError {}

/// Reads the file at `path`, refusing one larger than `max` bytes.
pub async fn read(session: &SshSession, path: &str, max: u64) -> Result<Vec<u8>, RemoteFileError> {
    let (command, stdin) = read_request(path, max)?;
    let out = session
        .exec_output(&command, Some(stdin))
        .await
        .map_err(RemoteFileError::Ssh)?;
    parse_read(&out, max)
}

/// Writes `content` to the file at `path` in place, if the file is in the state `expect` names.
pub async fn write(
    session: &SshSession,
    path: &str,
    content: &[u8],
    expect: Expect<'_>,
) -> Result<(), RemoteFileError> {
    let (command, stdin) = write_request(path, content, expect)?;
    let out = session
        .exec_output(&command, Some(stdin))
        .await
        .map_err(RemoteFileError::Ssh)?;
    parse_write(&out)
}

/// `sh -c '<script>' sh [arg]`: a command line every common login shell reads the same way.
fn command(script: &str, arg: Option<&str>) -> String {
    debug_assert!(!script.contains(['\'', '\\', '\n']));
    match arg {
        Some(arg) => format!("sh -c '{script}' sh {arg}"),
        None => format!("sh -c '{script}' sh"),
    }
}

fn check_path(path: &str) -> Result<(), RemoteFileError> {
    if path.is_empty() || path.contains(['\n', '\r', '\0']) {
        Err(RemoteFileError::InvalidPath)
    } else {
        Ok(())
    }
}

/// The command and stdin of a read.
fn read_request(path: &str, max: u64) -> Result<(String, Vec<u8>), RemoteFileError> {
    check_path(path)?;
    Ok((
        command(READ_SCRIPT, Some(&max.to_string())),
        format!("{path}\n").into_bytes(),
    ))
}

/// The command and stdin of a write.
fn write_request(
    path: &str,
    content: &[u8],
    expect: Expect<'_>,
) -> Result<(String, Vec<u8>), RemoteFileError> {
    check_path(path)?;
    let state = match expect {
        Expect::Any => "any".to_owned(),
        Expect::Missing => "missing".to_owned(),
        Expect::Content(bytes) => {
            let (crc, size) = cksum(bytes);
            format!("{crc} {size}")
        }
    };
    let mut stdin = format!("{path}\n{state}\n").into_bytes();
    stdin.extend_from_slice(content);
    Ok((command(WRITE_SCRIPT, None), stdin))
}

fn last_line(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_owned()
}

fn failed(out: &ExecOutput) -> RemoteFileError {
    RemoteFileError::Failed {
        status: out.exit_status,
        stderr: last_line(&out.stderr),
    }
}

fn parse_read(out: &ExecOutput, max: u64) -> Result<Vec<u8>, RemoteFileError> {
    match out.exit_status {
        Some(0) => {
            let start = if out.stdout.starts_with(MARKER) {
                0
            } else {
                // Lines a startup file printed come before the marker.
                out.stdout
                    .windows(MARKER.len() + 1)
                    .position(|w| w[0] == b'\n' && &w[1..] == MARKER)
                    .map(|at| at + 1)
                    .ok_or_else(|| failed(out))?
            };
            let content = &out.stdout[start + MARKER.len()..];
            // The file grew after the script measured it.
            if content.len() as u64 > max {
                return Err(RemoteFileError::TooLarge(content.len() as u64));
            }
            Ok(content.to_vec())
        }
        Some(2) => Err(RemoteFileError::NotFound),
        Some(3) => Err(RemoteFileError::IsDirectory),
        Some(4) => Err(RemoteFileError::PermissionDenied),
        Some(5) => Err(last_line(&out.stderr)
            .parse()
            .map_or_else(|_| failed(out), RemoteFileError::TooLarge)),
        Some(6) => Err(RemoteFileError::NotRegular),
        Some(8) => Err(RemoteFileError::DanglingLink),
        _ => Err(failed(out)),
    }
}

fn parse_write(out: &ExecOutput) -> Result<(), RemoteFileError> {
    match out.exit_status {
        Some(0) => Ok(()),
        Some(2) => Err(RemoteFileError::NoDirectory),
        Some(3) => Err(RemoteFileError::IsDirectory),
        Some(4) => Err(RemoteFileError::PermissionDenied),
        Some(6) => Err(RemoteFileError::NotRegular),
        Some(7) => Err(RemoteFileError::Changed),
        Some(8) => Err(RemoteFileError::DanglingLink),
        _ => Err(failed(out)),
    }
}

/// The checksum and size POSIX `cksum` prints for `data`: CRC-32 with the polynomial
/// `0x04C11DB7`, most significant bit first, over the data and then its length in as few bytes as
/// it takes, least significant first, inverted.
#[must_use]
pub fn cksum(data: &[u8]) -> (u32, u64) {
    fn step(crc: u32, byte: u8) -> u32 {
        let mut crc = crc ^ (u32::from(byte) << 24);
        for _ in 0..8 {
            crc = if crc & 0x8000_0000 == 0 {
                crc << 1
            } else {
                (crc << 1) ^ 0x04C1_1DB7
            };
        }
        crc
    }
    let mut crc = data.iter().fold(0, |crc, &b| step(crc, b));
    let size = data.len() as u64;
    let mut n = size;
    while n != 0 {
        crc = step(crc, (n & 0xff) as u8);
        n >>= 8;
    }
    (!crc, size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cksum_matches_posix() {
        // Values from `cksum` (GNU coreutils and BSD agree).
        assert_eq!(cksum(b""), (4_294_967_295, 0));
        assert_eq!(cksum(b"123456789"), (930_766_865, 9));
        assert_eq!(cksum(b"hello\nworld\n"), (3_795_442_390, 12));
    }

    #[test]
    fn scripts_survive_any_login_shell() {
        for script in [READ_SCRIPT, WRITE_SCRIPT] {
            // `!` too: csh expands it even inside single quotes when its history is on.
            assert!(!script.contains(['\'', '\\', '\n', '!']), "{script}");
        }
        assert!(matches!(
            read_request("a\nb", 10),
            Err(RemoteFileError::InvalidPath)
        ));
        assert!(matches!(
            write_request("", b"", Expect::Any),
            Err(RemoteFileError::InvalidPath)
        ));
        let (_, stdin) = write_request("/tmp/x", b"body", Expect::Content(b"123456789")).unwrap();
        assert_eq!(stdin, b"/tmp/x\n930766865 9\nbody");
    }

    #[test]
    fn output_before_the_marker_is_dropped() {
        let out = |stdout: &[u8], status| ExecOutput {
            exit_status: Some(status),
            stdout: stdout.to_vec(),
            stderr: b"line 1\nwc: 99\n".to_vec(),
        };
        assert_eq!(
            parse_read(&out(b"hatoba-file\nbody", 0), 10).unwrap(),
            b"body"
        );
        assert_eq!(
            parse_read(&out(b"motd\nhatoba-file\nhatoba-file\n", 0), 20).unwrap(),
            b"hatoba-file\n"
        );
        assert!(matches!(
            parse_read(&out(b"no marker", 0), 20),
            Err(RemoteFileError::Failed { .. })
        ));
        assert!(matches!(
            parse_read(&out(b"hatoba-file\n0123456789x", 0), 10),
            Err(RemoteFileError::TooLarge(11))
        ));
        let mut big = out(b"", 5);
        big.stderr = b"1048577\n".to_vec();
        assert!(matches!(
            parse_read(&big, 10),
            Err(RemoteFileError::TooLarge(1_048_577))
        ));
        match parse_read(&out(b"", 1), 10) {
            Err(RemoteFileError::Failed { status, stderr }) => {
                assert_eq!((status, stderr.as_str()), (Some(1), "wc: 99"));
            }
            other => panic!("{other:?}"),
        }
    }

    /// The scripts against the local `sh`, `wc`, `cksum` and `dirname` (GNU on Linux, BSD on
    /// macOS), started the way a login shell would start them.
    #[cfg(unix)]
    mod local_sh {
        use std::io::Write;
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        use std::path::Path;
        use std::process::{Command, Stdio};

        use super::super::*;

        fn run(home: &Path, command: &str, stdin: &[u8]) -> ExecOutput {
            let mut child = Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(home)
                .env("HOME", home)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(stdin).unwrap();
            let out = child.wait_with_output().unwrap();
            ExecOutput {
                exit_status: out.status.code().map(|c| c as u32),
                stdout: out.stdout,
                stderr: out.stderr,
            }
        }

        fn read(home: &Path, path: &str, max: u64) -> Result<Vec<u8>, RemoteFileError> {
            let (command, stdin) = read_request(path, max)?;
            parse_read(&run(home, &command, &stdin), max)
        }

        fn write(
            home: &Path,
            path: &str,
            content: &[u8],
            expect: Expect<'_>,
        ) -> Result<(), RemoteFileError> {
            let (command, stdin) = write_request(path, content, expect)?;
            parse_write(&run(home, &command, &stdin))
        }

        fn is_root() -> bool {
            Command::new("id")
                .arg("-u")
                .output()
                .is_ok_and(|o| o.stdout.starts_with(b"0\n"))
        }

        #[test]
        fn reads_files_and_classifies_failures() {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            // A name the login shell would mangle if it were quoted into the command.
            let odd = "-x $(id) 'q' \"d\" \\b *.conf";
            std::fs::write(home.join(odd), b"a\r\nb\x00\xff").unwrap();
            assert_eq!(read(home, odd, 100).unwrap(), b"a\r\nb\x00\xff");
            let full = home.join(odd);
            assert_eq!(
                read(home, full.to_str().unwrap(), 100).unwrap(),
                b"a\r\nb\x00\xff"
            );
            std::fs::write(home.join("notes"), b"in home\n").unwrap();
            assert_eq!(read(home, "~/notes", 100).unwrap(), b"in home\n");
            std::fs::write(home.join("empty"), b"").unwrap();
            assert_eq!(read(home, "empty", 0).unwrap(), b"");

            assert!(matches!(
                read(home, "missing", 100),
                Err(RemoteFileError::NotFound)
            ));
            assert!(matches!(
                read(home, "~", 100),
                Err(RemoteFileError::IsDirectory)
            ));
            assert!(matches!(
                read(home, "/dev/null", 100),
                Err(RemoteFileError::NotRegular)
            ));
            assert!(matches!(
                read(home, "notes", 7),
                Err(RemoteFileError::TooLarge(8))
            ));
            assert!(read(home, "notes", 8).is_ok());
            std::os::unix::fs::symlink(home.join("gone"), home.join("dangling")).unwrap();
            assert!(matches!(
                read(home, "dangling", 100),
                Err(RemoteFileError::DanglingLink)
            ));

            if !is_root() {
                let secret = home.join("secret");
                std::fs::write(&secret, b"x").unwrap();
                std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o200)).unwrap();
                assert!(matches!(
                    read(home, "secret", 100),
                    Err(RemoteFileError::PermissionDenied)
                ));
            }
        }

        #[test]
        fn writes_in_place_after_checking_the_file() {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            let path = home.join("app.conf");
            std::fs::write(&path, b"old\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
            let link = home.join("link.conf");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            let before = std::fs::metadata(&path).unwrap();

            // The file changed: nothing is written.
            assert!(matches!(
                write(home, "app.conf", b"new\n", Expect::Content(b"older\n")),
                Err(RemoteFileError::Changed)
            ));
            assert!(matches!(
                write(home, "app.conf", b"new\n", Expect::Missing),
                Err(RemoteFileError::Changed)
            ));
            assert_eq!(std::fs::read(&path).unwrap(), b"old\n");

            // Through the link, in place: same inode and mode, the link stays a link.
            write(home, "link.conf", b"new\r\n\x00", Expect::Content(b"old\n")).unwrap();
            let after = std::fs::metadata(&path).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"new\r\n\x00");
            assert_eq!(after.ino(), before.ino());
            assert_eq!(after.mode() & 0o777, 0o640);
            assert!(
                std::fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            write(home, "~/app.conf", b"", Expect::Any).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"");

            // New files: only where the card saw none, and only in a directory that exists.
            assert!(matches!(
                write(home, "new.txt", b"x", Expect::Content(b"")),
                Err(RemoteFileError::Changed)
            ));
            write(home, "new.txt", b"first line\n", Expect::Missing).unwrap();
            assert_eq!(
                std::fs::read(home.join("new.txt")).unwrap(),
                b"first line\n"
            );
            assert!(matches!(
                write(home, "no/such/dir/x", b"x", Expect::Any),
                Err(RemoteFileError::NoDirectory)
            ));
            assert!(matches!(
                write(home, ".", b"x", Expect::Any),
                Err(RemoteFileError::IsDirectory)
            ));
            // A link to nothing would create its target somewhere else: refused.
            std::os::unix::fs::symlink(home.join("elsewhere"), home.join("dangling")).unwrap();
            for expect in [Expect::Missing, Expect::Any] {
                assert!(matches!(
                    write(home, "dangling", b"x", expect),
                    Err(RemoteFileError::DanglingLink)
                ));
            }
            assert!(!home.join("elsewhere").exists());

            if !is_root() {
                let locked = home.join("locked");
                std::fs::create_dir(&locked).unwrap();
                std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
                assert!(matches!(
                    write(home, "locked/x", b"x", Expect::Missing),
                    Err(RemoteFileError::PermissionDenied)
                ));
                std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
                std::fs::write(home.join("ro"), b"x").unwrap();
                std::fs::set_permissions(home.join("ro"), std::fs::Permissions::from_mode(0o400))
                    .unwrap();
                assert!(matches!(
                    write(home, "ro", b"y", Expect::Content(b"x")),
                    Err(RemoteFileError::PermissionDenied)
                ));
            }
        }
    }
}
