//! Reading, writing and deleting a file on the host over an exec channel (spec §13.4, "File
//! tools", and AI-41's revert; the SFTP panel's editor, SFTP-05, is meant to use it too).
//!
//! Each operation runs a fixed `sh -c` script. The path, and for a write the expected state and
//! the content, travel on stdin, so nothing the caller passes goes through the quoting of the
//! user's login shell, which may be bash, zsh, fish or csh. The scripts are one line without
//! single quotes or backslashes for the same reason. A write pipes into the file
//! (`cat > path`), so an existing file keeps its inode, owner, mode and the links to it; a file
//! it creates gets the login's umask, or the mode the caller names.

use std::fmt;

use crate::error::SshError;
use crate::session::{ExecOutput, SshSession};

/// The line both scripts print before the resolved path (and the read script before the file),
/// so that anything the login shell's startup files print to stdout is told apart from them.
const MARKER: &[u8] = b"hatoba-file\n";

/// Reads the file named on stdin, `$1` bytes at most, and prints the marker, the path as resolved,
/// the file's mode as the first ten characters of `ls -l` (empty when `ls` fails) and the file. A
/// relative path or one that starts with `~/` is taken from `$HOME`. Exit statuses: 2 missing,
/// 3 directory, 4 not readable, 5 larger than `$1` (its size on stderr), 6 not a regular file,
/// 8 a symbolic link to nothing, 9 no path.
const READ_SCRIPT: &str = "IFS= read -r f || exit 9; \
     case $f in \"~\") f=$HOME;; \"~/\"*) f=$HOME/${f#\"~/\"};; /*) ;; *) f=$HOME/$f;; esac; \
     [ -d \"$f\" ] && exit 3; [ -L \"$f\" ] && { [ -e \"$f\" ] || exit 8; }; \
     [ -e \"$f\" ] || exit 2; [ -f \"$f\" ] || exit 6; \
     [ -r \"$f\" ] || exit 4; n=$(wc -c < \"$f\") || exit 4; n=$(echo $n); \
     [ \"$n\" -le \"$1\" ] || { echo \"$n\" >&2; exit 5; }; m=$(ls -ldnL -- \"$f\" 2>/dev/null); \
     echo hatoba-file; printf %s \"$f\"; echo; printf %.10s \"$m\"; echo; exec cat -- \"$f\"";

/// Writes the rest of stdin to the file named on its first line, resolved as the read script
/// resolves it, after checking the state on its second line: `any`, `missing`, or the file's
/// `cksum` output (`crc size`). Its third line is the mode, in octal, of a file the write
/// creates, or `none` for the umask's: such a file is created readable by its owner only and then
/// given that mode, so it is never open to others in between. An existing file keeps its mode.
/// Prints the marker and the resolved path. Exit statuses: 2 no such directory, 3 directory,
/// 4 not writable, 6 not a regular file, 7 not in the expected state, 8 a symbolic link to
/// nothing (writing would create its target, wherever that is), 9 no path. A file that passed the
/// checks and still cannot be opened, written or given its mode (a read-only file system, a full
/// disk) is status 1, with the message on stderr: dash would exit with 2 for the failed
/// redirection, which reads as a missing directory.
///
/// The check and the write run in one command, so a change made since the caller read the file
/// is caught. A write by another process in the moment between the check and `cat` is not: an
/// in-place rewrite has no lock that every writer honors.
const WRITE_SCRIPT: &str = "IFS= read -r f || exit 9; IFS= read -r g || exit 9; \
     IFS= read -r m || exit 9; \
     case $f in \"~\") f=$HOME;; \"~/\"*) f=$HOME/${f#\"~/\"};; /*) ;; *) f=$HOME/$f;; esac; \
     [ -d \"$f\" ] && exit 3; [ -L \"$f\" ] && { [ -e \"$f\" ] || exit 8; }; \
     if [ -e \"$f\" ]; then [ -f \"$f\" ] || exit 6; [ \"$g\" = missing ] && exit 7; \
     case $g in any) ;; *) c=$(cksum < \"$f\") || exit 4; set -- $c; \
     [ \"$1 $2\" = \"$g\" ] || exit 7;; esac; [ -w \"$f\" ] || exit 4; m=none; \
     else [ \"$g\" = any ] || [ \"$g\" = missing ] || exit 7; d=$(dirname -- \"$f\"); \
     [ -d \"$d\" ] || exit 2; [ -w \"$d\" ] || exit 4; [ \"$m\" = none ] || umask 077; fi; \
     echo hatoba-file; printf %s \"$f\"; echo; cat > \"$f\" || exit 1; \
     [ \"$m\" = none ] || chmod $m \"$f\" || exit 1";

/// Deletes the file named on stdin's first line, resolved as the read script resolves it, if it
/// is a regular file (not a symbolic link) whose `cksum` output is stdin's second line. Prints
/// the marker and the resolved path. Exit statuses: 2 missing, 3 directory, 4 not deletable,
/// 6 not a regular file, 7 not in the expected state, 8 a symbolic link in its place (neither the
/// link nor its target is deleted), 9 no path; 1 when `rm` fails after the checks, with its
/// message on stderr. Like a write, a change in the moment between the check and `rm` is not
/// caught.
const DELETE_SCRIPT: &str = "IFS= read -r f || exit 9; IFS= read -r g || exit 9; \
     case $f in \"~\") f=$HOME;; \"~/\"*) f=$HOME/${f#\"~/\"};; /*) ;; *) f=$HOME/$f;; esac; \
     [ -L \"$f\" ] && exit 8; [ -d \"$f\" ] && exit 3; [ -e \"$f\" ] || exit 2; \
     [ -f \"$f\" ] || exit 6; c=$(cksum < \"$f\") || exit 4; set -- $c; \
     [ \"$1 $2\" = \"$g\" ] || exit 7; d=$(dirname -- \"$f\"); [ -w \"$d\" ] || exit 4; \
     echo hatoba-file; printf %s \"$f\"; echo; rm -f -- \"$f\" || exit 1";

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
    /// Delete: a symbolic link is in the file's place, so neither it nor its target was deleted.
    SymbolicLink,
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
            Self::SymbolicLink => f.write_str("a symbolic link is in its place"),
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

/// A file read from the host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFile {
    /// The path as the script resolved it: absolute, with a relative path or one that starts
    /// with `~/` taken from the home directory. Symbolic links and `..` are left as they are.
    pub path: String,
    /// The file's bytes.
    pub bytes: Vec<u8>,
    /// Its permission bits (`0o777` at most), as `ls` showed them; `None` when it showed none
    /// that parse.
    pub mode: Option<u32>,
}

/// Reads the file at `path`, refusing one larger than `max` bytes.
pub async fn read(
    session: &SshSession,
    path: &str,
    max: u64,
) -> Result<RemoteFile, RemoteFileError> {
    let (command, stdin) = read_request(path, max)?;
    let out = session
        .exec_output(&command, Some(stdin))
        .await
        .map_err(RemoteFileError::Ssh)?;
    parse_read(&out, max)
}

/// Writes `content` to the file at `path` in place, if the file is in the state `expect` names,
/// and returns the path as resolved. `mode`: the permission bits a file the write creates gets,
/// readable by its owner only until then; `None` leaves them to the login's umask. An existing
/// file keeps its own.
pub async fn write(
    session: &SshSession,
    path: &str,
    content: &[u8],
    expect: Expect<'_>,
    mode: Option<u32>,
) -> Result<String, RemoteFileError> {
    let (command, stdin) = write_request(path, content, expect, mode)?;
    let out = session
        .exec_output(&command, Some(stdin))
        .await
        .map_err(RemoteFileError::Ssh)?;
    parse_write(&out)
}

/// Deletes the file at `path` if it still holds `expect`, compared by POSIX `cksum` checksum and
/// size, and returns the path as resolved. A symbolic link in the file's place is not deleted
/// ([`RemoteFileError::SymbolicLink`]): the link is not what was checked.
pub async fn delete(
    session: &SshSession,
    path: &str,
    expect: &[u8],
) -> Result<String, RemoteFileError> {
    let (command, stdin) = delete_request(path, expect)?;
    let out = session
        .exec_output(&command, Some(stdin))
        .await
        .map_err(RemoteFileError::Ssh)?;
    parse_delete(&out)
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
    mode: Option<u32>,
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
    let mode = mode.map_or_else(|| "none".to_owned(), |m| format!("{:o}", m & 0o777));
    let mut stdin = format!("{path}\n{state}\n{mode}\n").into_bytes();
    stdin.extend_from_slice(content);
    Ok((command(WRITE_SCRIPT, None), stdin))
}

/// The command and stdin of a deletion.
fn delete_request(path: &str, expect: &[u8]) -> Result<(String, Vec<u8>), RemoteFileError> {
    check_path(path)?;
    let (crc, size) = cksum(expect);
    Ok((
        command(DELETE_SCRIPT, None),
        format!("{path}\n{crc} {size}\n").into_bytes(),
    ))
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

/// What follows the marker line in a script's stdout: the resolved path and the rest. `None`
/// without a marker and a path line after it. Lines a startup file printed come before the
/// marker.
fn after_marker(stdout: &[u8]) -> Option<(String, &[u8])> {
    let start = if stdout.starts_with(MARKER) {
        0
    } else {
        stdout
            .windows(MARKER.len() + 1)
            .position(|w| w[0] == b'\n' && &w[1..] == MARKER)?
            + 1
    };
    let rest = &stdout[start + MARKER.len()..];
    let end = rest.iter().position(|&b| b == b'\n')?;
    let path = String::from_utf8_lossy(&rest[..end]).into_owned();
    Some((path, &rest[end + 1..]))
}

/// The permission bits in the first ten characters of `ls -l` for a regular file, such as
/// `-rw-r-----`: set-id and sticky bits are left out. `None` for anything else.
fn parse_mode(line: &[u8]) -> Option<u32> {
    let [b'-', bits @ ..] = line else {
        return None;
    };
    if bits.len() != 9 {
        return None;
    }
    bits.iter().enumerate().try_fold(0, |mode, (i, &c)| {
        let set = match (i % 3, c) {
            (_, b'-') | (2, b'S' | b'T') => false,
            (0, b'r') | (1, b'w') | (2, b'x' | b's' | b't') => true,
            _ => return None,
        };
        Some(if set { mode | (1 << (8 - i)) } else { mode })
    })
}

fn parse_read(out: &ExecOutput, max: u64) -> Result<RemoteFile, RemoteFileError> {
    match out.exit_status {
        Some(0) => {
            let (path, rest) = after_marker(&out.stdout).ok_or_else(|| failed(out))?;
            let end = rest
                .iter()
                .position(|&b| b == b'\n')
                .ok_or_else(|| failed(out))?;
            let content = &rest[end + 1..];
            // The file grew after the script measured it.
            if content.len() as u64 > max {
                return Err(RemoteFileError::TooLarge(content.len() as u64));
            }
            Ok(RemoteFile {
                path,
                bytes: content.to_vec(),
                mode: parse_mode(&rest[..end]),
            })
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

/// The resolved path of a write that succeeded. Status 0 without the marker is a failure: a login
/// shell whose startup files exit with 0 never ran the script.
fn parse_write(out: &ExecOutput) -> Result<String, RemoteFileError> {
    match out.exit_status {
        Some(0) => after_marker(&out.stdout)
            .map(|(path, _)| path)
            .ok_or_else(|| failed(out)),
        Some(2) => Err(RemoteFileError::NoDirectory),
        Some(3) => Err(RemoteFileError::IsDirectory),
        Some(4) => Err(RemoteFileError::PermissionDenied),
        Some(6) => Err(RemoteFileError::NotRegular),
        Some(7) => Err(RemoteFileError::Changed),
        Some(8) => Err(RemoteFileError::DanglingLink),
        _ => Err(failed(out)),
    }
}

/// The resolved path of a deletion that succeeded, as for [`parse_write`].
fn parse_delete(out: &ExecOutput) -> Result<String, RemoteFileError> {
    match out.exit_status {
        Some(0) => after_marker(&out.stdout)
            .map(|(path, _)| path)
            .ok_or_else(|| failed(out)),
        Some(2) => Err(RemoteFileError::NotFound),
        Some(3) => Err(RemoteFileError::IsDirectory),
        Some(4) => Err(RemoteFileError::PermissionDenied),
        Some(6) => Err(RemoteFileError::NotRegular),
        Some(7) => Err(RemoteFileError::Changed),
        Some(8) => Err(RemoteFileError::SymbolicLink),
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
        for script in [READ_SCRIPT, WRITE_SCRIPT, DELETE_SCRIPT] {
            // `!` too: csh expands it even inside single quotes when its history is on.
            assert!(!script.contains(['\'', '\\', '\n', '!']), "{script}");
        }
        assert!(matches!(
            read_request("a\nb", 10),
            Err(RemoteFileError::InvalidPath)
        ));
        assert!(matches!(
            write_request("", b"", Expect::Any, None),
            Err(RemoteFileError::InvalidPath)
        ));
        let (_, stdin) =
            write_request("/tmp/x", b"body", Expect::Content(b"123456789"), None).unwrap();
        assert_eq!(stdin, b"/tmp/x\n930766865 9\nnone\nbody");
        // Permission bits only, in octal.
        let (_, stdin) = write_request("/tmp/x", b"", Expect::Missing, Some(0o4640)).unwrap();
        assert_eq!(stdin, b"/tmp/x\nmissing\n640\n");
        assert!(matches!(
            delete_request("a\rb", b""),
            Err(RemoteFileError::InvalidPath)
        ));
        let (_, stdin) = delete_request("~/new.txt", b"123456789").unwrap();
        assert_eq!(stdin, b"~/new.txt\n930766865 9\n");
    }

    #[test]
    fn output_before_the_marker_is_dropped() {
        let out = |stdout: &[u8], status| ExecOutput {
            exit_status: Some(status),
            stdout: stdout.to_vec(),
            stderr: b"line 1\nwc: 99\n".to_vec(),
        };
        assert_eq!(
            parse_read(&out(b"hatoba-file\n/home/a/b c\n-rw-r-----\nbody", 0), 10).unwrap(),
            RemoteFile {
                path: "/home/a/b c".into(),
                bytes: b"body".to_vec(),
                mode: Some(0o640),
            }
        );
        // `ls` failed: no mode, the file all the same.
        let noisy = parse_read(&out(b"motd\nhatoba-file\n/x\n\nhatoba-file\n", 0), 20).unwrap();
        assert_eq!(
            (noisy.path.as_str(), &noisy.bytes[..], noisy.mode),
            ("/x", &b"hatoba-file\n"[..], None)
        );
        for broken in [
            &b"no marker"[..],
            b"hatoba-file\n/no/line/end",
            b"hatoba-file\n/x\n-rw-r--r--",
        ] {
            assert!(matches!(
                parse_read(&out(broken, 0), 20),
                Err(RemoteFileError::Failed { .. })
            ));
        }
        assert!(matches!(
            parse_read(&out(b"hatoba-file\n/x\n-rw-------\n0123456789x", 0), 10),
            Err(RemoteFileError::TooLarge(11))
        ));
        assert_eq!(
            parse_write(&out(b"motd\nhatoba-file\n/srv/a\n", 0)).unwrap(),
            "/srv/a"
        );
        assert!(matches!(
            parse_write(&out(b"", 0)),
            Err(RemoteFileError::Failed { .. })
        ));
        assert_eq!(
            parse_delete(&out(b"motd\nhatoba-file\n/srv/a\n", 0)).unwrap(),
            "/srv/a"
        );
        assert!(matches!(
            parse_delete(&out(b"", 0)),
            Err(RemoteFileError::Failed { .. })
        ));
        assert!(matches!(
            parse_delete(&out(b"", 7)),
            Err(RemoteFileError::Changed)
        ));
        assert!(matches!(
            parse_delete(&out(b"", 2)),
            Err(RemoteFileError::NotFound)
        ));
        assert!(matches!(
            parse_delete(&out(b"", 8)),
            Err(RemoteFileError::SymbolicLink)
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

    #[test]
    fn modes_read_as_ls_shows_them() {
        assert_eq!(parse_mode(b"-rw-r--r--"), Some(0o644));
        assert_eq!(parse_mode(b"-rw-------"), Some(0o600));
        assert_eq!(parse_mode(b"-rwxr-x--x"), Some(0o751));
        // Set-id and sticky bits are left out; whether the execute bit is set is kept.
        assert_eq!(parse_mode(b"-rwsr-Sr-t"), Some(0o745));
        assert_eq!(parse_mode(b"----------"), Some(0));
        for other in [
            &b""[..],
            b"drwxr-xr-x",
            b"lrwxrwxrwx",
            b"-rw-r--r",
            b"-rw-r--r-+",
            b"-rwxrwxrwz",
        ] {
            assert_eq!(
                parse_mode(other),
                None,
                "{}",
                String::from_utf8_lossy(other)
            );
        }
    }

    /// The scripts against the local `sh`, `wc`, `cksum` and `dirname` (GNU on Linux, BSD on
    /// macOS), started the way a login shell would start them. They run in `/`, so relative paths
    /// show that they are taken from `$HOME`.
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
                .current_dir("/")
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

        fn read_file(home: &Path, path: &str, max: u64) -> Result<RemoteFile, RemoteFileError> {
            let (command, stdin) = read_request(path, max)?;
            parse_read(&run(home, &command, &stdin), max)
        }

        fn read(home: &Path, path: &str, max: u64) -> Result<Vec<u8>, RemoteFileError> {
            read_file(home, path, max).map(|f| f.bytes)
        }

        fn write_file(
            home: &Path,
            path: &str,
            content: &[u8],
            expect: Expect<'_>,
            mode: Option<u32>,
        ) -> Result<String, RemoteFileError> {
            let (command, stdin) = write_request(path, content, expect, mode)?;
            parse_write(&run(home, &command, &stdin))
        }

        fn write(
            home: &Path,
            path: &str,
            content: &[u8],
            expect: Expect<'_>,
        ) -> Result<(), RemoteFileError> {
            write_file(home, path, content, expect, None).map(|_| ())
        }

        fn mode_of(path: &Path) -> u32 {
            std::fs::metadata(path).unwrap().mode() & 0o7777
        }

        fn delete_file(home: &Path, path: &str, expect: &[u8]) -> Result<String, RemoteFileError> {
            let (command, stdin) = delete_request(path, expect)?;
            parse_delete(&run(home, &command, &stdin))
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
            // Its permission bits, through a link too.
            std::fs::set_permissions(home.join("notes"), std::fs::Permissions::from_mode(0o640))
                .unwrap();
            std::os::unix::fs::symlink(home.join("notes"), home.join("notes.link")).unwrap();
            for path in ["notes", "notes.link"] {
                assert_eq!(
                    read_file(home, path, 100).unwrap().mode,
                    Some(0o640),
                    "{path}"
                );
            }
            // `~/`, relative and absolute paths resolve to the same path.
            let resolved = home.join("notes").to_str().unwrap().to_owned();
            for path in ["~/notes", "notes", resolved.as_str()] {
                assert_eq!(read_file(home, path, 100).unwrap().path, resolved, "{path}");
            }
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
            // A mode is for a file the write creates: this one keeps its own.
            write_file(home, "app.conf", b"x", Expect::Any, Some(0o600)).unwrap();
            assert_eq!(mode_of(&path), 0o640);

            // New files: only where the card saw none, and only in a directory that exists.
            assert!(matches!(
                write(home, "new.txt", b"x", Expect::Content(b"")),
                Err(RemoteFileError::Changed)
            ));
            assert_eq!(
                write_file(home, "new.txt", b"first line\n", Expect::Missing, None).unwrap(),
                home.join("new.txt").to_str().unwrap()
            );
            assert_eq!(
                std::fs::read(home.join("new.txt")).unwrap(),
                b"first line\n"
            );
            // Created with the mode named, whatever the umask: Restore anyway puts back a file
            // that was deleted with the mode it had.
            write_file(home, "secret.conf", b"s\n", Expect::Missing, Some(0o600)).unwrap();
            assert_eq!(mode_of(&home.join("secret.conf")), 0o600);
            write_file(home, "shared.conf", b"s\n", Expect::Missing, Some(0o664)).unwrap();
            assert_eq!(mode_of(&home.join("shared.conf")), 0o664);
            assert_eq!(std::fs::read(home.join("shared.conf")).unwrap(), b"s\n");
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

        #[test]
        fn deletes_only_the_file_as_expected() {
            let dir = tempfile::tempdir().unwrap();
            let home = dir.path();
            let path = home.join("new.conf");
            std::fs::write(&path, b"made\n").unwrap();

            // Changed since: kept.
            assert!(matches!(
                delete_file(home, "new.conf", b"other\n"),
                Err(RemoteFileError::Changed)
            ));
            assert!(path.exists());
            assert_eq!(
                delete_file(home, "~/new.conf", b"made\n").unwrap(),
                path.to_str().unwrap()
            );
            assert!(!path.exists());
            assert!(matches!(
                delete_file(home, "new.conf", b"made\n"),
                Err(RemoteFileError::NotFound)
            ));
            assert!(matches!(
                delete_file(home, ".", b""),
                Err(RemoteFileError::IsDirectory)
            ));
            assert!(matches!(
                delete_file(home, "/dev/null", b""),
                Err(RemoteFileError::NotRegular)
            ));

            // A link put in the file's place is not the file: neither it nor its target goes,
            // even when the target holds what was expected.
            let target = home.join("target");
            std::fs::write(&target, b"made\n").unwrap();
            std::os::unix::fs::symlink(&target, &path).unwrap();
            assert!(matches!(
                delete_file(home, "new.conf", b"made\n"),
                Err(RemoteFileError::SymbolicLink)
            ));
            assert!(path.exists() && target.exists());
            std::os::unix::fs::symlink(home.join("gone"), home.join("dangling")).unwrap();
            assert!(matches!(
                delete_file(home, "dangling", b""),
                Err(RemoteFileError::SymbolicLink)
            ));

            if !is_root() {
                let locked = home.join("locked");
                std::fs::create_dir(&locked).unwrap();
                std::fs::write(locked.join("x"), b"x").unwrap();
                std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o500)).unwrap();
                assert!(matches!(
                    delete_file(home, "locked/x", b"x"),
                    Err(RemoteFileError::PermissionDenied)
                ));
                std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o700)).unwrap();
                assert!(locked.join("x").exists());
            }
        }
    }
}
