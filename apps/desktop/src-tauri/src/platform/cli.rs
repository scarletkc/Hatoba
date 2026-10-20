//! The `hatoba` command (HOST-13): a launcher on the user's `PATH` that opens Hatoba with what
//! follows it on the command line, or runs one of the command's own commands, such as
//! `hatoba exec` (HOST-15), in the terminal. Settings → General adds and removes it.
//!
//! - Windows: `hatoba.exe`, a copy of the console program installed beside the executable
//!   (`src/bin/hatoba-cli.rs`), and `hatoba` for Git Bash and WSL, which runs it, in a `bin`
//!   folder beside the executable. The folder goes on the user's `PATH` in `HKCU\Environment`,
//!   which is read and written whole, so a long `PATH` is never cut short. The uninstaller runs
//!   Hatoba with `--uninstall-command` to take it off again.
//! - Linux: `~/.local/bin/hatoba`, which starts the executable, or the AppImage.
//! - macOS: `/usr/local/bin/hatoba`, which opens the app bundle. Writing there usually takes an
//!   administrator, so macOS asks for one.
//!
//! On Linux and macOS the command's own commands run the executable in the foreground, which runs
//! them before the app would start (`main.rs`).

use std::path::Path;

pub use imp::{install, location, on_path, uninstall};

/// Begins the comment in every launcher, which tells Hatoba's launcher from a file someone else
/// put at the same path.
#[cfg_attr(windows, allow(dead_code))]
const MARKER: &str = "Hatoba command line launcher";
/// The comment itself. No `>`, which a batch file could read as a redirection.
const COMMENT: &str =
    "Hatoba command line launcher. Turn it off in Hatoba's settings, under General.";

/// Writes `contents` unless the file holds them already.
fn write_if_changed(path: &Path, contents: &str) -> std::io::Result<()> {
    if std::fs::read(path).is_ok_and(|old| old == contents.as_bytes()) {
        return Ok(());
    }
    std::fs::write(path, contents)
}

/// Whether `dir` is one of the folders in a `PATH` value. Windows compares without case and
/// ignores a trailing separator and quotes, as it does when it looks a command up.
#[cfg_attr(not(windows), allow(dead_code))]
fn path_has(path: &str, dir: &str) -> bool {
    path.split(';').any(|entry| same_dir(entry, dir))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn same_dir(a: &str, b: &str) -> bool {
    let norm = |s: &str| {
        s.trim()
            .trim_matches('"')
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    !a.trim().is_empty() && norm(a) == norm(b)
}

/// `path` with `dir` added at the end, or `None` when it has it already.
#[cfg_attr(not(windows), allow(dead_code))]
fn path_with(path: &str, dir: &str) -> Option<String> {
    if path_has(path, dir) {
        None
    } else if path.is_empty() || path.ends_with(';') {
        Some(format!("{path}{dir}"))
    } else {
        Some(format!("{path};{dir}"))
    }
}

/// `path` without `dir`, or `None` when it does not have it. The other entries stay as written.
#[cfg_attr(not(windows), allow(dead_code))]
fn path_without(path: &str, dir: &str) -> Option<String> {
    if !path_has(path, dir) {
        return None;
    }
    Some(
        path.split(';')
            .filter(|entry| !same_dir(entry, dir))
            .collect::<Vec<_>>()
            .join(";"),
    )
}

/// `'…'` for sh, which takes everything between single quotes literally.
#[cfg_attr(windows, allow(dead_code))]
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A launcher that starts `exe` in the background with its arguments, so the terminal neither
/// waits for Hatoba nor takes it down when it closes, unless they are one of the command's own
/// commands (HOST-15).
#[cfg_attr(windows, allow(dead_code))]
fn detached_launcher(exe: &Path) -> String {
    let exe = sh_quote(&exe.to_string_lossy());
    format!(
        "#!/bin/sh\n# {COMMENT}\n{}nohup {exe} \"$@\" >/dev/null 2>&1 &\n",
        own_commands(&exe)
    )
}

/// The lines of a launcher that run `exe`, quoted for sh, in the foreground for the command's own
/// commands (HOST-15), which print what they get back.
#[cfg_attr(windows, allow(dead_code))]
fn own_commands(exe: &str) -> String {
    format!(
        "case \"$1\" in\n  {}) exec {exe} \"$@\" ;;\nesac\n",
        hatoba_cli::COMMANDS.join("|")
    )
}

#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};

    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_EXPAND_SZ, REG_SZ};
    use winreg::{RegKey, RegValue};

    use super::{COMMENT, path_with, path_without, write_if_changed};
    use crate::error::{AppError, AppResult};

    /// The console program installed beside the executable, and its copy in `bin`.
    const CONSOLE: &str = "hatoba-cli.exe";
    const COMMAND: &str = "hatoba.exe";

    /// The folder that goes on the `PATH`. Only the launchers are in it: the executable's own
    /// folder also holds `uninstall.exe`, which must not become a command.
    pub fn location() -> AppResult<PathBuf> {
        let exe = std::env::current_exe()?;
        let dir = exe
            .parent()
            .ok_or_else(|| AppError::internal("the executable has no folder"))?;
        Ok(dir.join("bin"))
    }

    /// `hatoba.exe` opens Hatoba without waiting for it, as the `hatoba.cmd` of earlier versions
    /// did, which it replaces: cmd.exe and PowerShell would run `hatoba.exe` first anyway.
    pub fn install() -> AppResult<()> {
        let bin = location()?;
        let console = std::env::current_exe()?.with_file_name(CONSOLE);
        if !console.is_file() {
            return Err(AppError::internal(format!(
                "{} is missing",
                console.display()
            )));
        }
        std::fs::create_dir_all(&bin)?;
        replace_if_changed(&console, &bin.join(COMMAND))?;
        write_if_changed(
            &bin.join("hatoba"),
            &format!("#!/bin/sh\n# {COMMENT}\nexec \"$(dirname \"$0\")/{COMMAND}\" \"$@\"\n"),
        )?;
        remove(&bin.join("hatoba.cmd"))?;
        edit_path(|path| path_with(path, &bin.to_string_lossy()))
    }

    pub fn uninstall() -> AppResult<()> {
        let bin = location()?;
        edit_path(|path| path_without(path, &bin.to_string_lossy()))?;
        for name in ["hatoba.cmd", "hatoba"] {
            remove(&bin.join(name))?;
        }
        // A copy that runs now cannot be removed; off the `PATH`, it is never found again.
        let exe = bin.join(COMMAND);
        if remove(&exe).is_err() {
            let _ = std::fs::rename(&exe, exe.with_extension("exe.old"));
        }
        // Only when empty: anything else in it is not Hatoba's.
        let _ = std::fs::remove_dir(&bin);
        Ok(())
    }

    fn remove(path: &Path) -> AppResult<()> {
        match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Copies `from` over `to` unless they are the same. A `to` that runs now, as `hatoba exec`
    /// may while Hatoba updates, cannot be written but can be renamed out of the way.
    fn replace_if_changed(from: &Path, to: &Path) -> std::io::Result<()> {
        let new = std::fs::read(from)?;
        if std::fs::read(to).is_ok_and(|old| old == new) {
            return Ok(());
        }
        let old = to.with_extension("exe.old");
        let _ = std::fs::remove_file(&old);
        if to.exists() {
            std::fs::rename(to, &old)?;
        }
        std::fs::write(to, &new)?;
        let _ = std::fs::remove_file(&old);
        Ok(())
    }

    pub fn on_path(_location: &Path) -> bool {
        true
    }

    /// Applies `edit` to the user's `PATH` and tells running programs when it changed.
    fn edit_path(edit: impl FnOnce(&str) -> Option<String>) -> AppResult<()> {
        if edit_value("Environment", "Path", edit)? {
            announce();
        }
        Ok(())
    }

    /// Applies `edit` to a string value of a key under `HKEY_CURRENT_USER`, keeping its type
    /// (`REG_EXPAND_SZ` unless it was a plain `REG_SZ`) and deleting it when `edit` empties it.
    /// `edit` returns `None` to leave it as it is. Returns whether the value changed.
    fn edit_value(
        key: &str,
        name: &str,
        edit: impl FnOnce(&str) -> Option<String>,
    ) -> std::io::Result<bool> {
        let key =
            RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(key, KEY_READ | KEY_WRITE)?;
        let (value, plain) = match key.get_raw_value(name) {
            Ok(value) => (decode(&value.bytes), value.vtype == REG_SZ),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (String::new(), false),
            Err(e) => return Err(e),
        };
        let Some(next) = edit(&value) else {
            return Ok(false);
        };
        if next.is_empty() {
            key.delete_value(name)?;
        } else {
            let bytes = next
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<u8>>();
            key.set_raw_value(
                name,
                &RegValue {
                    bytes: bytes.into(),
                    vtype: if plain { REG_SZ } else { REG_EXPAND_SZ },
                },
            )?;
        }
        Ok(true)
    }

    /// A registry string: UTF-16LE, up to its first NUL.
    fn decode(bytes: &[u8]) -> String {
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| u16::from_le_bytes(c))
            .take_while(|&u| u != 0)
            .collect();
        String::from_utf16_lossy(&units)
    }

    /// Explorer, and the terminals it starts from then on, read the new `PATH` after this.
    fn announce() {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{
            HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
        };
        use windows::core::w;
        // SAFETY: the string is a static NUL-terminated literal that outlives the call.
        unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                WPARAM(0),
                LPARAM(w!("Environment").as_ptr() as isize),
                SMTO_ABORTIFHUNG,
                5000,
                None,
            );
        }
    }

    #[cfg(test)]
    mod tests {
        use winreg::RegKey;
        use winreg::enums::{HKEY_CURRENT_USER, REG_EXPAND_SZ, REG_SZ};

        use super::{decode, edit_value, replace_if_changed};
        use crate::platform::cli::{path_with, path_without};

        /// HOST-15: the `hatoba.exe` in `bin` follows the console program an update installed.
        #[test]
        fn the_command_is_copied_again_only_when_it_changed() {
            let dir = std::env::temp_dir().join(format!("hatoba-cli-copy-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let (from, to) = (dir.join("hatoba-cli.exe"), dir.join("hatoba.exe"));
            std::fs::write(&from, b"v1").unwrap();
            replace_if_changed(&from, &to).unwrap();
            assert_eq!(std::fs::read(&to).unwrap(), b"v1");
            let copied = std::fs::metadata(&to).unwrap().modified().unwrap();
            replace_if_changed(&from, &to).unwrap();
            assert_eq!(std::fs::metadata(&to).unwrap().modified().unwrap(), copied);

            std::fs::write(&from, b"v2").unwrap();
            replace_if_changed(&from, &to).unwrap();
            assert_eq!(std::fs::read(&to).unwrap(), b"v2");
            assert!(!to.with_extension("exe.old").exists());
            std::fs::remove_dir_all(&dir).unwrap();
        }

        #[test]
        fn decodes_registry_strings_up_to_the_nul() {
            let bytes: Vec<u8> = "C:\\a;D:\\b\0junk"
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect();
            assert_eq!(decode(&bytes), r"C:\a;D:\b");
            assert_eq!(decode(&[]), "");
        }

        /// Runs against a throwaway key, never the user's `PATH`.
        #[test]
        fn edits_a_registry_value_in_place() {
            let name = format!(r"Software\Hatoba-test-{}", std::process::id());
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let (key, _) = hkcu.create_subkey(&name).unwrap();
            let dir = r"C:\Hatoba\bin";
            let edit = |f: &dyn Fn(&str) -> Option<String>| edit_value(&name, "Path", f).unwrap();

            // A missing value is created as REG_EXPAND_SZ, so `%VAR%` entries keep working.
            assert!(edit(&|p| path_with(p, dir)));
            let value = key.get_raw_value("Path").unwrap();
            assert_eq!(value.vtype, REG_EXPAND_SZ);
            assert_eq!(decode(&value.bytes), dir);
            assert!(!edit(&|p| path_with(p, dir)));

            // Removing the only entry removes the value.
            assert!(edit(&|p| path_without(p, dir)));
            assert!(key.get_raw_value("Path").is_err());

            // A plain REG_SZ stays one.
            key.set_value("Path", &r"C:\tools").unwrap();
            assert!(edit(&|p| path_with(p, dir)));
            let value = key.get_raw_value("Path").unwrap();
            assert_eq!(value.vtype, REG_SZ);
            assert_eq!(decode(&value.bytes), r"C:\tools;C:\Hatoba\bin");

            hkcu.delete_subkey_all(&name).unwrap();
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use super::{MARKER, write_if_changed};
    use crate::error::{AppError, AppResult};

    /// Refuses a file at `path` that is not Hatoba's launcher.
    pub fn ensure_free(path: &Path) -> AppResult<()> {
        match std::fs::read(path) {
            Ok(old) if !String::from_utf8_lossy(&old).contains(MARKER) => Err(AppError::invalid(
                "command",
                format!("{} already exists", path.display()),
            )),
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }

    /// Whether the file at `path` is Hatoba's launcher.
    pub fn is_ours(path: &Path) -> bool {
        std::fs::read(path).is_ok_and(|c| String::from_utf8_lossy(&c).contains(MARKER))
    }

    pub fn write_executable(path: &Path, contents: &str) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_if_changed(path, contents)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use std::path::{Path, PathBuf};

    use super::detached_launcher;
    use super::unix::{ensure_free, is_ours, write_executable};
    use crate::error::{AppError, AppResult};

    pub fn location() -> AppResult<PathBuf> {
        let home = dirs::home_dir().ok_or_else(|| AppError::internal("no home folder"))?;
        Ok(home.join(".local/bin/hatoba"))
    }

    pub fn install() -> AppResult<()> {
        let path = location()?;
        ensure_free(&path)?;
        // An AppImage runs from a mount that goes away when it quits; `APPIMAGE` names the file.
        let exe = match std::env::var_os("APPIMAGE") {
            Some(appimage) if !appimage.is_empty() => PathBuf::from(appimage),
            _ => std::env::current_exe()?,
        };
        Ok(write_executable(&path, &detached_launcher(&exe))?)
    }

    pub fn uninstall() -> AppResult<()> {
        let path = location()?;
        if is_ours(&path) {
            std::fs::remove_file(&path)?;
        }
        Ok(())
    }

    pub fn on_path(location: &Path) -> bool {
        let (Some(dir), Some(path)) = (location.parent(), std::env::var_os("PATH")) else {
            return false;
        };
        std::env::split_paths(&path).any(|p| p == dir)
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::unix::{ensure_free, is_ours, write_executable};
    use super::{COMMENT, detached_launcher, own_commands, sh_quote};
    use crate::error::{AppError, AppResult, ErrorCode};

    const LAUNCHER: &str = "/usr/local/bin/hatoba";

    pub fn location() -> AppResult<PathBuf> {
        Ok(PathBuf::from(LAUNCHER))
    }

    pub fn install() -> AppResult<()> {
        let path = location()?;
        ensure_free(&path)?;
        let contents = launcher()?;
        match write_executable(&path, &contents) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                let staged = std::env::temp_dir().join(format!("hatoba-{}", std::process::id()));
                std::fs::write(&staged, &contents)?;
                let done = as_admin(&format!(
                    "mkdir -p /usr/local/bin && install -m 755 {} {LAUNCHER}",
                    sh_quote(&staged.to_string_lossy())
                ));
                let _ = std::fs::remove_file(&staged);
                done
            }
            done => Ok(done?),
        }
    }

    pub fn uninstall() -> AppResult<()> {
        let path = location()?;
        if !is_ours(&path) {
            return Ok(());
        }
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                as_admin(&format!("rm -f {LAUNCHER}"))
            }
            done => Ok(done?),
        }
    }

    pub fn on_path(_location: &Path) -> bool {
        // `/etc/paths` lists it. An app opened from Finder runs without it, so the app's own
        // `PATH` cannot tell.
        true
    }

    /// Opens the app bundle with the arguments. `open -n` starts another copy, which hands them
    /// to the one already running. The command's own commands run the bundle's executable
    /// instead (HOST-15). A development build has no bundle and runs its executable.
    fn launcher() -> AppResult<String> {
        let exe = std::env::current_exe()?;
        // …/Hatoba.app/Contents/MacOS/Hatoba
        let bundle = exe
            .ancestors()
            .nth(3)
            .filter(|p| p.extension().is_some_and(|e| e == "app"));
        Ok(match bundle {
            Some(app) => format!(
                "#!/bin/sh\n# {COMMENT}\n{}exec open -n -a {} --args \"$@\"\n",
                own_commands(&sh_quote(&exe.to_string_lossy())),
                sh_quote(&app.to_string_lossy())
            ),
            None => detached_launcher(&exe),
        })
    }

    /// Runs a shell command as an administrator, after macOS asks for one.
    fn as_admin(command: &str) -> AppResult<()> {
        let quoted = command.replace('\\', "\\\\").replace('"', "\\\"");
        let out = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(format!(
                "do shell script \"{quoted}\" with administrator privileges"
            ))
            .output()?;
        if out.status.success() {
            return Ok(());
        }
        let message = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        // -128: the user closed the password prompt.
        if message.contains("-128") {
            return Err(AppError::new(ErrorCode::Cancelled, message));
        }
        Err(AppError::io(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_the_folder_once_and_keeps_the_rest_of_the_path() {
        let dir = r"C:\Users\Ann\AppData\Local\Hatoba\bin";
        assert_eq!(path_with("", dir).as_deref(), Some(dir));
        assert_eq!(
            path_with(r"C:\a;%USERPROFILE%\b", dir).as_deref(),
            Some(r"C:\a;%USERPROFILE%\b;C:\Users\Ann\AppData\Local\Hatoba\bin")
        );
        assert_eq!(
            path_with(r"C:\a;", dir).as_deref(),
            Some(r"C:\a;C:\Users\Ann\AppData\Local\Hatoba\bin")
        );
        // Already there, written another way.
        assert_eq!(
            path_with(r#"C:\a;"c:\users\ann\appdata\local\hatoba\BIN\""#, dir),
            None
        );
    }

    #[test]
    fn a_long_path_is_kept_whole() {
        let dir = r"C:\Hatoba\bin";
        let long = (0..400)
            .map(|i| format!(r"C:\tools\t{i}"))
            .collect::<Vec<_>>()
            .join(";");
        let added = path_with(&long, dir).unwrap();
        assert!(added.starts_with(&long) && added.ends_with(dir));
        assert_eq!(path_without(&added, dir).as_deref(), Some(long.as_str()));
    }

    #[test]
    fn removes_only_the_folder() {
        let dir = r"C:\Hatoba\bin";
        assert_eq!(path_without(r"C:\a;;C:\b", dir), None);
        assert_eq!(
            path_without(r"C:\a;C:\HATOBA\bin\;;C:\b;C:\Hatoba\bin", dir).as_deref(),
            Some(r"C:\a;;C:\b")
        );
        assert_eq!(path_without(r"C:\Hatoba\bin", dir).as_deref(), Some(""));
        // A folder inside it, or one that only starts the same, is someone else's.
        assert_eq!(path_without(r"C:\Hatoba\bin\x;C:\Hatoba\bins", dir), None);
    }

    #[test]
    fn the_comment_carries_the_marker() {
        assert!(COMMENT.starts_with(MARKER));
        assert!(!COMMENT.contains(['>', '<', '|', '&', '%']));
    }

    #[test]
    fn quotes_paths_for_sh() {
        assert_eq!(sh_quote("/opt/Hatoba"), "'/opt/Hatoba'");
        assert_eq!(sh_quote("/home/o'neil/x"), r"'/home/o'\''neil/x'");
        let launcher = detached_launcher(Path::new("/home/me/Apps/Hatoba.AppImage"));
        assert!(launcher.starts_with("#!/bin/sh\n# Hatoba command line launcher."));
        assert!(launcher.contains("nohup '/home/me/Apps/Hatoba.AppImage' \"$@\""));
        // HOST-15: `hatoba exec` and the rest run in the foreground.
        assert!(launcher.contains(
            "case \"$1\" in\n  hosts|exec|cp|help|-h|--help|--version) \
             exec '/home/me/Apps/Hatoba.AppImage' \"$@\" ;;\nesac\n"
        ));
    }
}
