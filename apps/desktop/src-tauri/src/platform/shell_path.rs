//! The `PATH` that MCP stdio servers are looked up on and started with (spec §13.9, AI-32).
//!
//! An app started from Finder or the Dock gets only `/usr/bin:/bin:/usr/sbin:/sbin`, so `npx`,
//! `uvx` or `node` from Homebrew, nvm, mise or Volta are not found. On macOS the login shell's
//! `PATH` is resolved once, when the first stdio server starts: the shell runs once with a short
//! timeout, and when it fails the process `PATH` plus the Homebrew directories is used. Users who
//! never set up a stdio server never run it. Other platforms
//! start the servers with the inherited `PATH`, as before.
//!
//! The value is handed to the servers through their configuration (see
//! `mcp::with_default_path`), never through `std::env::set_var`.

use std::ffi::OsStr;

/// The default `PATH` for MCP stdio servers; `None` where the inherited one is right.
#[cfg(target_os = "macos")]
pub fn default_path() -> Option<&'static OsStr> {
    use std::ffi::OsString;
    use std::sync::OnceLock;

    static PATH: OnceLock<OsString> = OnceLock::new();
    Some(PATH.get_or_init(resolve))
}

#[cfg(not(target_os = "macos"))]
pub fn default_path() -> Option<&'static OsStr> {
    None
}

#[cfg(target_os = "macos")]
fn resolve() -> std::ffi::OsString {
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(3);
    let shell = login_shell(std::env::var("SHELL").ok());
    let marker = format!("__hatoba_{}__", uuid::Uuid::now_v7().simple());
    // Only the source and the entry count are logged: the value becomes part of each server's
    // environment, which logs never contain (SEC-04).
    match run_login_shell(&shell, &marker, TIMEOUT) {
        Some(path) => {
            tracing::info!(
                entries = path.split(':').count(),
                "MCP default PATH from the login shell"
            );
            path.into()
        }
        None => {
            tracing::warn!("login shell PATH unavailable, MCP default PATH is the fallback");
            fallback_path(std::env::var_os("PATH").as_deref())
        }
    }
}

/// `$SHELL` when it is an absolute path, else the macOS default.
#[cfg(any(target_os = "macos", test))]
fn login_shell(shell: Option<String>) -> String {
    shell
        .filter(|s| s.starts_with('/'))
        .unwrap_or_else(|| "/bin/zsh".to_owned())
}

/// What `printf` printed between two `marker`s, ignoring whatever the rc files wrote around it.
#[cfg(any(target_os = "macos", test))]
fn parse_marked(output: &str, marker: &str) -> Option<String> {
    let mut parts = output.split(marker);
    parts.next()?;
    let value = parts.next()?.trim();
    // The closing marker must follow, or the shell was cut off.
    parts.next()?;
    (!value.is_empty()).then(|| value.to_owned())
}

/// Runs `shell -ilc` (interactive login, so `.zprofile`, `.zshrc` and the like are read) and
/// returns its `PATH`; `None` when it fails, prints nothing usable, or takes longer than
/// `timeout`. It runs with no stdin, its stderr dropped, and in a process group of its own, so
/// giving up kills whatever its rc files started along with it.
#[cfg(target_os = "macos")]
fn run_login_shell(shell: &str, marker: &str, timeout: std::time::Duration) -> Option<String> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut child = Command::new(shell)
        .args([
            "-ilc",
            &format!("printf '%s%s%s' {marker} \"$PATH\" {marker}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    // A reader thread, so the wait below can give up: a shell that leaves a background process
    // holding the pipe would otherwise block the read after the shell itself is gone.
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = tx.send(bytes);
    });
    let deadline = std::time::Instant::now() + timeout;
    let output = rx.recv_timeout(timeout).ok();
    // The timeout covers the shell's exit too: an rc file can close stdout and keep running.
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            _ => {
                kill_group(child.id());
                let _ = child.wait();
                return None;
            }
        }
    }
    let Some(output) = output else {
        // The shell has exited, but something it started still holds the pipe.
        kill_group(child.id());
        return None;
    };
    parse_marked(&String::from_utf8_lossy(&output), marker)
}

/// Kills the process group that `run_login_shell` started the shell in (its id is the shell's).
#[cfg(target_os = "macos")]
fn kill_group(shell_pid: u32) {
    let Ok(pgid) = libc::pid_t::try_from(shell_pid) else {
        return;
    };
    // SAFETY: killpg only sends a signal; an empty or vanished group gives ESRCH, which is ignored.
    unsafe {
        libc::killpg(pgid, libc::SIGKILL);
    }
}

/// The process `PATH` with the Homebrew directories (Apple Silicon, then Intel) added at the end
/// when missing. `split_paths` and `join_paths` use the platform's separator, so the tests that
/// spell out `:` run on Unix only.
#[cfg(any(target_os = "macos", all(test, unix)))]
fn fallback_path(process_path: Option<&OsStr>) -> std::ffi::OsString {
    let mut dirs: Vec<std::path::PathBuf> = process_path
        .map(|p| std::env::split_paths(p).collect())
        .unwrap_or_default();
    for extra in ["/opt/homebrew/bin", "/usr/local/bin"] {
        if !dirs.iter().any(|d| d == std::path::Path::new(extra)) {
            dirs.push(extra.into());
        }
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const M: &str = "__hatoba_m__";

    #[test]
    fn the_path_is_read_between_the_markers() {
        let out = format!("Last login: today\nmotd\n{M}/opt/homebrew/bin:/usr/bin{M}\n");
        assert_eq!(
            parse_marked(&out, M),
            Some("/opt/homebrew/bin:/usr/bin".into())
        );
    }

    #[test]
    fn output_without_a_complete_marked_value_is_rejected() {
        assert_eq!(parse_marked("", M), None);
        assert_eq!(parse_marked("/usr/bin", M), None);
        assert_eq!(parse_marked(&format!("{M}/usr/bin"), M), None);
        assert_eq!(parse_marked(&format!("{M}  {M}"), M), None);
    }

    #[cfg(unix)]
    #[test]
    fn the_fallback_adds_the_homebrew_directories_once() {
        let fallback = |p: Option<&str>| fallback_path(p.map(OsStr::new));
        assert_eq!(
            fallback(Some("/usr/bin:/bin")),
            "/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin"
        );
        assert_eq!(
            fallback(Some("/usr/local/bin:/usr/bin")),
            "/usr/local/bin:/usr/bin:/opt/homebrew/bin"
        );
        assert_eq!(fallback(None), "/opt/homebrew/bin:/usr/local/bin");
    }

    #[test]
    fn only_an_absolute_shell_path_is_used() {
        assert_eq!(login_shell(Some("/bin/bash".into())), "/bin/bash");
        assert_eq!(login_shell(Some("zsh".into())), "/bin/zsh");
        assert_eq!(login_shell(Some(String::new())), "/bin/zsh");
        assert_eq!(login_shell(None), "/bin/zsh");
    }

    #[cfg(target_os = "macos")]
    mod shell {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

        use super::super::run_login_shell;

        /// A script standing in for the login shell; it gets `-ilc <script>` like a real one.
        fn fake_shell(name: &str, body: &str) -> String {
            let path =
                std::env::temp_dir().join(format!("hatoba-test-{name}-{}", std::process::id()));
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path.to_string_lossy().into_owned()
        }

        #[test]
        fn the_path_survives_noise_from_the_rc_files() {
            let shell = fake_shell(
                "noisy",
                "echo 'banner'; echo 'oops' >&2; PATH=/opt/x/bin:/usr/bin; eval \"$2\"",
            );
            let path = run_login_shell(&shell, "__m__", Duration::from_secs(5));
            std::fs::remove_file(&shell).unwrap();
            assert_eq!(path, Some("/opt/x/bin:/usr/bin".into()));
        }

        #[test]
        fn a_shell_that_hangs_is_killed_after_the_timeout() {
            let shell = fake_shell("hang", "exec sleep 30");
            let started = Instant::now();
            let path = run_login_shell(&shell, "__m__", Duration::from_millis(300));
            std::fs::remove_file(&shell).unwrap();
            assert_eq!(path, None);
            assert!(started.elapsed() < Duration::from_secs(10));
        }

        #[test]
        fn a_shell_that_closes_stdout_and_hangs_is_killed_after_the_timeout() {
            let shell = fake_shell("closed", "exec >/dev/null; sleep 30");
            let started = Instant::now();
            let path = run_login_shell(&shell, "__m__", Duration::from_millis(300));
            std::fs::remove_file(&shell).unwrap();
            assert_eq!(path, None);
            assert!(started.elapsed() < Duration::from_secs(10));
        }

        #[test]
        fn what_the_shell_started_is_killed_with_it() {
            let pid_file =
                std::env::temp_dir().join(format!("hatoba-test-child-pid-{}", std::process::id()));
            let shell = fake_shell(
                "child",
                &format!("sleep 30 & echo $! > '{}'; wait", pid_file.display()),
            );
            let path = run_login_shell(&shell, "__m__", Duration::from_millis(300));
            std::fs::remove_file(&shell).unwrap();
            assert_eq!(path, None);
            let pid = std::fs::read_to_string(&pid_file).unwrap();
            std::fs::remove_file(&pid_file).unwrap();
            let alive = || {
                std::process::Command::new("/bin/kill")
                    .args(["-0", pid.trim()])
                    .stderr(std::process::Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success())
            };
            let started = Instant::now();
            while alive() && started.elapsed() < Duration::from_secs(2) {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(
                !alive(),
                "the shell's background sleep outlived the timeout"
            );
        }

        #[test]
        fn a_missing_shell_gives_nothing() {
            let path = run_login_shell("/nonexistent/shell", "__m__", Duration::from_secs(1));
            assert_eq!(path, None);
        }

        #[test]
        fn the_real_login_shell_reports_a_path() {
            let path = run_login_shell("/bin/zsh", "__hatoba_real__", Duration::from_secs(10));
            assert!(path.is_some_and(|p| p.contains("/usr/bin")));
        }
    }
}
