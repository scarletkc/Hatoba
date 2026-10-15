//! The file tools on Windows itself (AI-38…41, §7.5), where the hosts' `sh` scripts cannot run:
//! the same operations as `hatoba_ssh::remote_file`, with the same results and errors, done with
//! the file system calls. Paths are taken as the scripts take them: `~`, `~/…` and a relative
//! path from the home directory. They are shown with forward slashes, which Windows accepts too,
//! so a path the model copies from a result works in the next call.
//!
//! A write replaces an existing file's content in place, so it keeps its attributes, its owner
//! and the links to it, and creates a missing one, as the scripts do. The checks that the file is
//! what the caller expects come right before it.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};

use hatoba_ssh::remote_file::{
    Expect, RemoteDir, RemoteEntry, RemoteFile, RemoteFileError, RemoteLines, cksum,
};

/// A path as the model gave it, on this computer: `~` and `~/…` (or `~\…`) from `home`, an
/// absolute path as it is, a Git Bash path such as `/c/Users` as `C:/Users` on Windows, and any
/// other path from `home`.
pub fn resolve(path: &str, home: &Path) -> PathBuf {
    if path == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        return home.join(rest);
    }
    #[cfg(windows)]
    if let Some(drive) = git_bash_drive(path) {
        return PathBuf::from(drive);
    }
    let given = Path::new(path);
    if given.is_absolute() || given.has_root() {
        given.to_path_buf()
    } else {
        home.join(given)
    }
}

/// `/c/Users/…` as `C:/Users/…`.
#[cfg(windows)]
fn git_bash_drive(path: &str) -> Option<String> {
    let rest = path.strip_prefix('/')?;
    let mut chars = rest.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    let after = chars.as_str();
    (after.is_empty() || after.starts_with('/')).then(|| {
        format!(
            "{}:/{}",
            letter.to_ascii_uppercase(),
            after.trim_start_matches('/')
        )
    })
}

/// How a result names a path: with forward slashes, and the drive letter in capitals, on Windows.
pub fn shown(path: &Path) -> String {
    let text = path.to_string_lossy();
    if cfg!(windows) {
        let text = text.replace('\\', "/");
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(letter), Some(':')) if letter.is_ascii_alphabetic() => {
                format!("{}{}", letter.to_ascii_uppercase(), &text[1..])
            }
            _ => text,
        }
    } else {
        text.into_owned()
    }
}

fn check_path(path: &str) -> Result<(), RemoteFileError> {
    if path.is_empty() || path.contains(['\n', '\r', '\0']) {
        Err(RemoteFileError::InvalidPath)
    } else {
        Ok(())
    }
}

fn io_error(e: &io::Error) -> RemoteFileError {
    match e.kind() {
        io::ErrorKind::NotFound => RemoteFileError::NotFound,
        io::ErrorKind::PermissionDenied => RemoteFileError::PermissionDenied,
        io::ErrorKind::IsADirectory => RemoteFileError::IsDirectory,
        _ => RemoteFileError::Failed {
            status: None,
            stderr: e.to_string(),
        },
    }
}

/// A file to read: what the read script checks first.
fn readable(file: &Path) -> Result<fs::Metadata, RemoteFileError> {
    let link = fs::symlink_metadata(file).map_err(|e| io_error(&e))?;
    let meta = match fs::metadata(file) {
        Ok(meta) => meta,
        Err(e) if e.kind() == io::ErrorKind::NotFound && link.file_type().is_symlink() => {
            return Err(RemoteFileError::DanglingLink);
        }
        Err(e) => return Err(io_error(&e)),
    };
    if meta.is_dir() {
        return Err(RemoteFileError::IsDirectory);
    }
    if !meta.is_file() {
        return Err(RemoteFileError::NotRegular);
    }
    Ok(meta)
}

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o777)
}

#[cfg(not(unix))]
fn mode_of(_meta: &fs::Metadata) -> Option<u32> {
    None
}

/// Reads the file at `path`, refusing one larger than `max` bytes.
pub fn read(path: &str, max: u64, home: &Path) -> Result<RemoteFile, RemoteFileError> {
    check_path(path)?;
    let file = resolve(path, home);
    let meta = readable(&file)?;
    if meta.len() > max {
        return Err(RemoteFileError::TooLarge(meta.len()));
    }
    let mut bytes = Vec::new();
    File::open(&file)
        .and_then(|f| f.take(max.saturating_add(1)).read_to_end(&mut bytes))
        .map_err(|e| io_error(&e))?;
    if bytes.len() as u64 > max {
        return Err(RemoteFileError::TooLarge(bytes.len() as u64));
    }
    Ok(RemoteFile {
        path: shown(&file),
        bytes,
        mode: mode_of(&meta),
    })
}

/// Reads at most `max` bytes of the file at `path` from the start of line `from` (1-based), with
/// the file's line count.
pub fn read_lines(
    path: &str,
    from: u64,
    max: u64,
    home: &Path,
) -> Result<RemoteLines, RemoteFileError> {
    check_path(path)?;
    let file = resolve(path, home);
    readable(&file)?;
    let mut reader = BufReader::new(File::open(&file).map_err(|e| io_error(&e))?);
    let max_len = usize::try_from(max).unwrap_or(usize::MAX);
    let (mut lines, mut bytes, mut cut) = (0u64, Vec::new(), false);
    let mut line = Vec::new();
    loop {
        line.clear();
        let n = reader
            .read_until(b'\n', &mut line)
            .map_err(|e| io_error(&e))?;
        if n == 0 {
            break;
        }
        lines += 1;
        if lines >= from.max(1) && !cut {
            let room = max_len - bytes.len();
            if line.len() > room {
                bytes.extend_from_slice(&line[..room]);
                cut = true;
            } else {
                bytes.extend_from_slice(&line);
            }
        }
    }
    Ok(RemoteLines {
        path: shown(&file),
        lines,
        bytes,
        cut,
    })
}

/// Lists the directory at `path`, at most `max` bytes of entries as the list script counts them
/// (a byte for the kind, the name, a line break).
pub fn list_dir(path: &str, max: u64, home: &Path) -> Result<RemoteDir, RemoteFileError> {
    check_path(path)?;
    let dir = resolve(path, home);
    let meta = fs::metadata(&dir).map_err(|e| io_error(&e))?;
    if !meta.is_dir() {
        return Err(RemoteFileError::NotDirectory);
    }
    let mut entries = Vec::new();
    let mut used = 0u64;
    let mut cut = false;
    for entry in fs::read_dir(&dir).map_err(|e| io_error(&e))? {
        let Ok(entry) = entry else { continue };
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.contains(['\n', '\r']) {
            continue;
        }
        let size = name.len() as u64 + 2;
        if used + size > max {
            cut = true;
            break;
        }
        used += size;
        // A symbolic link to a directory counts as one, as `[ -d ]` counts it.
        let is_dir = fs::metadata(entry.path()).is_ok_and(|m| m.is_dir());
        entries.push(RemoteEntry { name, dir: is_dir });
    }
    Ok(RemoteDir {
        path: shown(&dir),
        entries,
        cut,
    })
}

/// Whether the file holds what `expect` says, right before a write.
fn expected(file: &Path, exists: bool, expect: Expect<'_>) -> Result<(), RemoteFileError> {
    match expect {
        Expect::Any => Ok(()),
        Expect::Missing if exists => Err(RemoteFileError::Changed),
        Expect::Missing => Ok(()),
        Expect::Content(_) if !exists => Err(RemoteFileError::Changed),
        Expect::Content(bytes) => {
            let current = fs::read(file).map_err(|e| io_error(&e))?;
            if cksum(&current) == cksum(bytes) {
                Ok(())
            } else {
                Err(RemoteFileError::Changed)
            }
        }
    }
}

/// Writes `content` to the file at `path` in place if it is in the state `expect` names, and
/// returns the path as resolved. A file the write creates gets `mode` where the system has
/// permission bits, owner-only until then.
pub fn write(
    path: &str,
    content: &[u8],
    expect: Expect<'_>,
    mode: Option<u32>,
    home: &Path,
) -> Result<String, RemoteFileError> {
    check_path(path)?;
    let file = resolve(path, home);
    let exists = match fs::symlink_metadata(&file) {
        Ok(link) => match fs::metadata(&file) {
            Ok(meta) if meta.is_dir() => return Err(RemoteFileError::IsDirectory),
            Ok(meta) if !meta.is_file() => return Err(RemoteFileError::NotRegular),
            Ok(_) => true,
            Err(_) if link.file_type().is_symlink() => return Err(RemoteFileError::DanglingLink),
            Err(e) => return Err(io_error(&e)),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => return Err(io_error(&e)),
    };
    if !exists {
        let parent = file.parent().filter(|p| !p.as_os_str().is_empty());
        if parent.is_some_and(|p| !p.is_dir()) {
            return Err(RemoteFileError::NoDirectory);
        }
    }
    expected(&file, exists, expect)?;
    let mut options = OpenOptions::new();
    options.write(true);
    if exists {
        options.truncate(true);
    } else {
        options.create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
    }
    let mut handle = options.open(&file).map_err(|e| match e.kind() {
        io::ErrorKind::AlreadyExists => RemoteFileError::Changed,
        _ => io_error(&e),
    })?;
    // The file is open, and cut short if it existed: a failure from here on may leave it so, as a
    // failed write on a host does, and its snapshot stays for Revert.
    handle
        .write_all(content)
        .and_then(|()| handle.flush())
        .map_err(|e| RemoteFileError::Failed {
            status: None,
            stderr: e.to_string(),
        })?;
    drop(handle);
    #[cfg(unix)]
    if !exists {
        use std::os::unix::fs::PermissionsExt;
        let bits = mode.unwrap_or_else(|| 0o666 & !umask());
        let _ = fs::set_permissions(&file, fs::Permissions::from_mode(bits & 0o777));
    }
    #[cfg(not(unix))]
    let _ = mode;
    Ok(shown(&file))
}

#[cfg(unix)]
#[allow(
    clippy::useless_conversion,
    reason = "mode_t is u16 on macOS and u32 on Linux"
)]
fn umask() -> u32 {
    // SAFETY: umask only swaps the process's mask; it is set back at once.
    unsafe {
        let mask = libc::umask(0o022);
        libc::umask(mask);
        u32::from(mask)
    }
}

/// Deletes the file at `path` if it still holds `expect`, compared by POSIX `cksum` checksum and
/// size, and returns the path as resolved. A symbolic link in its place is not deleted.
pub fn delete(path: &str, expect: &[u8], home: &Path) -> Result<String, RemoteFileError> {
    check_path(path)?;
    let file = resolve(path, home);
    let link = fs::symlink_metadata(&file).map_err(|e| io_error(&e))?;
    if link.file_type().is_symlink() {
        return Err(RemoteFileError::SymbolicLink);
    }
    if link.is_dir() {
        return Err(RemoteFileError::IsDirectory);
    }
    if !link.is_file() {
        return Err(RemoteFileError::NotRegular);
    }
    let current = fs::read(&file).map_err(|e| io_error(&e))?;
    if cksum(&current) != cksum(expect) {
        return Err(RemoteFileError::Changed);
    }
    fs::remove_file(&file).map_err(|e| io_error(&e))?;
    Ok(shown(&file))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn paths_are_taken_from_the_home_directory() {
        let home = Path::new(if cfg!(windows) {
            r"C:\Users\demo"
        } else {
            "/home/demo"
        });
        assert_eq!(resolve("~", home), home);
        assert_eq!(resolve("~/a.txt", home), home.join("a.txt"));
        assert_eq!(resolve("notes/a.txt", home), home.join("notes/a.txt"));
        if cfg!(windows) {
            assert_eq!(shown(&resolve("/c/Users/x", home)), "C:/Users/x");
            assert_eq!(shown(&resolve(r"d:\work\x.txt", home)), "D:/work/x.txt");
            assert_eq!(shown(&resolve(r"~\a.txt", home)), "C:/Users/demo/a.txt");
        } else {
            assert_eq!(resolve("/etc/hosts", home), Path::new("/etc/hosts"));
        }
    }

    #[test]
    fn reads_writes_and_deletes_as_the_scripts_do() {
        let dir = home();
        let home = dir.path();
        assert!(matches!(
            read("a.txt", 100, home),
            Err(RemoteFileError::NotFound)
        ));
        assert!(matches!(
            write("none/a.txt", b"x", Expect::Any, None, home),
            Err(RemoteFileError::NoDirectory)
        ));
        let path = write("a.txt", b"one\ntwo\n", Expect::Missing, None, home).unwrap();
        assert_eq!(path, shown(&home.join("a.txt")));
        assert!(matches!(
            write("a.txt", b"x", Expect::Missing, None, home),
            Err(RemoteFileError::Changed)
        ));
        assert!(matches!(
            write("a.txt", b"x", Expect::Content(b"other"), None, home),
            Err(RemoteFileError::Changed)
        ));
        write(
            "a.txt",
            b"one\r\ntwo\r\nthree",
            Expect::Content(b"one\ntwo\n"),
            None,
            home,
        )
        .unwrap();
        let file = read("a.txt", 100, home).unwrap();
        assert_eq!(file.bytes, b"one\r\ntwo\r\nthree");
        assert!(matches!(
            read("a.txt", 5, home),
            Err(RemoteFileError::TooLarge(15))
        ));
        let lines = read_lines("a.txt", 2, 6, home).unwrap();
        assert_eq!(
            (lines.lines, lines.bytes.as_slice(), lines.cut),
            (3, b"two\r\nt".as_slice(), true)
        );
        assert!(matches!(
            read("~", 100, home),
            Err(RemoteFileError::IsDirectory)
        ));
        assert!(matches!(
            delete("a.txt", b"stale", home),
            Err(RemoteFileError::Changed)
        ));
        delete("a.txt", b"one\r\ntwo\r\nthree", home).unwrap();
        assert!(!home.join("a.txt").exists());
    }

    #[test]
    fn lists_a_directory_with_its_subdirectories_marked() {
        let dir = home();
        let home = dir.path();
        fs::create_dir(home.join("sub")).unwrap();
        fs::write(home.join("f.txt"), "x").unwrap();
        let mut listing = list_dir("~", 1000, home).unwrap();
        listing.entries.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(
            listing.entries,
            [
                RemoteEntry {
                    name: "f.txt".into(),
                    dir: false
                },
                RemoteEntry {
                    name: "sub".into(),
                    dir: true
                }
            ]
        );
        assert!(!listing.cut);
        assert!(list_dir("~", 4, home).unwrap().cut);
        assert!(matches!(
            list_dir("f.txt", 100, home),
            Err(RemoteFileError::NotDirectory)
        ));
    }
}
