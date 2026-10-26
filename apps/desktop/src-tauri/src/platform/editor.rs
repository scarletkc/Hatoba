//! SFTP-05: opens the copy of a remote file in the program set in Settings, or in the app the
//! system opens that kind of file with. A file the system has no app for, and a program or script
//! whose app would run it rather than show it, opens in the platform's text editor.

use std::path::Path;
use std::process::{Child, Command, Stdio};

/// Kinds of file the system's app for them runs (programs, scripts, installers, shortcuts), so
/// that opening a copy would run it on this computer.
const RUNS: &[&str] = &[
    "app",
    "appimage",
    "appref-ms",
    "application",
    "bash",
    "bat",
    "cmd",
    "com",
    "command",
    "cpl",
    "csh",
    "desktop",
    "dmg",
    "exe",
    "fish",
    "gadget",
    "hta",
    "inf",
    "jar",
    "js",
    "jse",
    "ksh",
    "lnk",
    "mpkg",
    "msc",
    "msi",
    "msp",
    "pif",
    "pkg",
    "pl",
    "ps1",
    "psm1",
    "py",
    "pyw",
    "rb",
    "reg",
    "run",
    "scf",
    "scr",
    "sh",
    "terminal",
    "tool",
    "url",
    "vbe",
    "vbs",
    "webloc",
    "workflow",
    "ws",
    "wsf",
    "wsh",
    "zsh",
];

/// Whether the system's app for `path`'s kind of file would run it.
fn runs(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| RUNS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Opens `path` in `program`, or with the system's app for it while `program` is `None`. Blocks
/// for as long as the system takes to start the app.
pub fn open(path: &Path, program: Option<&str>) -> std::io::Result<()> {
    match program.filter(|p| !p.trim().is_empty()) {
        Some(program) => open_with(path, program),
        None if runs(path) => text_editor(path),
        None => default_app(path),
    }
}

/// Lets a started program run on its own; its exit is collected so it leaves no zombie.
fn detach(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

fn spawn(program: impl AsRef<std::ffi::OsStr>, args: &[&std::ffi::OsStr]) -> std::io::Result<()> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    detach(child);
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn open_with(path: &Path, program: &str) -> std::io::Result<()> {
    spawn(program, &[path.as_os_str()])
}

/// An app bundle opens through LaunchServices, which also brings a running one to the front.
#[cfg(target_os = "macos")]
fn open_with(path: &Path, program: &str) -> std::io::Result<()> {
    if Path::new(program).extension().is_some_and(|e| e == "app") {
        run_open(&["-a".as_ref(), program.as_ref(), path.as_os_str()])
    } else {
        spawn(program, &[path.as_os_str()])
    }
}

#[cfg(windows)]
fn default_app(path: &Path) -> std::io::Result<()> {
    let ext = path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    if ext.is_empty() || !windows::has_open_command(&ext) {
        return text_editor(path);
    }
    windows::shell_open(path)
}

#[cfg(windows)]
fn text_editor(path: &Path) -> std::io::Result<()> {
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let notepad = Path::new(&root).join("System32").join("notepad.exe");
    spawn(notepad, &[path.as_os_str()])
}

#[cfg(target_os = "macos")]
fn default_app(path: &Path) -> std::io::Result<()> {
    // `open` fails when no app opens the file's kind.
    run_open(&[path.as_os_str()]).or_else(|_| text_editor(path))
}

#[cfg(target_os = "macos")]
fn text_editor(path: &Path) -> std::io::Result<()> {
    run_open(&["-t".as_ref(), path.as_os_str()])
}

/// Runs `/usr/bin/open`, which returns once LaunchServices has the file.
#[cfg(target_os = "macos")]
fn run_open(args: &[&std::ffi::OsStr]) -> std::io::Result<()> {
    let out = Command::new("/usr/bin/open")
        .args(args)
        .stdin(Stdio::null())
        .output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(
            String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        ))
    }
}

/// `xdg-open` returns once the desktop has started the app, or runs a program without a desktop
/// to hand it to until the program ends. A failure comes quickly, so one still running after a
/// few seconds has opened the file.
#[cfg(target_os = "linux")]
fn default_app(path: &Path) -> std::io::Result<()> {
    let mut child = Command::new("xdg-open")
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(3) {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                text_editor(path)
            };
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    detach(child);
    Ok(())
}

/// The app the desktop opens plain text with, started through `gio launch`.
#[cfg(target_os = "linux")]
fn text_editor(path: &Path) -> std::io::Result<()> {
    let out = Command::new("xdg-mime")
        .args(["query", "default", "text/plain"])
        .stdin(Stdio::null())
        .output()?;
    let id = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    let desktop = (!id.is_empty() && !id.contains('/'))
        .then(|| {
            let home = std::env::var_os("XDG_DATA_HOME")
                .map(std::path::PathBuf::from)
                .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")));
            let dirs = std::env::var("XDG_DATA_DIRS")
                .ok()
                .filter(|d| !d.is_empty())
                .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
            home.into_iter()
                .chain(dirs.split(':').map(std::path::PathBuf::from))
                .map(|d| d.join("applications").join(&id))
                .find(|f| f.is_file())
        })
        .flatten()
        .ok_or_else(|| std::io::Error::other("no app opens plain text"))?;
    spawn(
        "gio",
        &["launch".as_ref(), desktop.as_os_str(), path.as_os_str()],
    )
}

#[cfg(windows)]
mod windows {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use ::windows::Win32::UI::Shell::{
        ASSOCF_INIT_IGNOREUNKNOWN, ASSOCSTR_COMMAND, AssocQueryStringW, ShellExecuteW,
    };
    use ::windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use ::windows::core::{HSTRING, PCWSTR, w};

    /// Whether `ext` has an app that opens it. Without `ASSOCF_INIT_IGNOREUNKNOWN` an unknown
    /// kind would resolve to the "Open with" dialog.
    pub fn has_open_command(ext: &str) -> bool {
        let ext = HSTRING::from(ext);
        let mut len = 0u32;
        // SAFETY: a null output buffer asks only for the length, written to `len`.
        let hr = unsafe {
            AssocQueryStringW(
                ASSOCF_INIT_IGNOREUNKNOWN,
                ASSOCSTR_COMMAND,
                &ext,
                w!("open"),
                None,
                &mut len,
            )
        };
        hr.is_ok() && len > 0
    }

    pub fn shell_open(path: &Path) -> std::io::Result<()> {
        let file: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: both strings are NUL-terminated and outlive the call.
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(file.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        // Values above 32 mean success.
        if result.0 as isize > 32 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::runs;
    use std::path::Path;

    #[test]
    fn scripts_and_programs_never_open_with_what_runs_them() {
        for name in [
            "deploy.sh",
            "tool.PY",
            "setup.bat",
            "x.ps1",
            "a.desktop",
            "b.jar",
        ] {
            assert!(runs(Path::new(name)), "{name}");
        }
        for name in [
            "nginx.conf",
            ".bashrc",
            "Makefile",
            "app.json",
            "notes.txt",
            "index.html",
        ] {
            assert!(!runs(Path::new(name)), "{name}");
        }
    }
}
