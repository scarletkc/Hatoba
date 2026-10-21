//! What the login shell sets that an app started from Finder, the Dock or a desktop launcher
//! never sees.
//!
//! On macOS that is the `PATH` that MCP stdio servers are looked up on and started with (spec
//! §13.9, AI-32): such an app gets only `/usr/bin:/bin:/usr/sbin:/sbin`, so `npx`, `uvx` or `node`
//! from Homebrew, nvm, mise or Volta are not found. On macOS and Linux it is also the
//! `SSH_AUTH_SOCK` that `~/.zshrc` or `~/.bashrc` exports for an agent such as Bitwarden's
//! (SSH-09). Other platforms start the servers with the inherited `PATH`, as before.
//!
//! The shell runs once, with a short timeout, when the first stdio server starts or the first
//! connection signs in with the agent, and both values come from that run. When it fails, the
//! process `PATH` plus the Homebrew directories is used, and the inherited `SSH_AUTH_SOCK`.
//! Users who need neither never run it.
//!
//! The values are handed on through configuration (see `mcp::with_default_path` and
//! `ssh_agent`), never through `std::env::set_var`.

use std::ffi::OsStr;

/// What the login shell printed, each value `None` when it was unset or empty.
#[cfg(unix)]
#[derive(Debug, Default, PartialEq, Eq)]
struct LoginEnv {
    /// Read on macOS only: Linux starts the MCP servers with the inherited `PATH`.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    path: Option<String>,
    ssh_auth_sock: Option<String>,
}

/// The default `PATH` for MCP stdio servers; `None` where the inherited one is right.
#[cfg(target_os = "macos")]
pub fn default_path() -> Option<&'static OsStr> {
    use std::ffi::OsString;
    use std::sync::OnceLock;

    static PATH: OnceLock<OsString> = OnceLock::new();
    Some(PATH.get_or_init(|| {
        // Only the source and the entry count are logged: the value becomes part of each
        // server's environment, which logs never contain (SEC-04).
        match login_env().and_then(|env| env.path.as_deref()) {
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
    }))
}

#[cfg(not(target_os = "macos"))]
pub fn default_path() -> Option<&'static OsStr> {
    None
}

/// The `SSH_AUTH_SOCK` the login shell exports; `None` when it exports none or did not answer.
/// It blocks for the shell's run the first time.
#[cfg(unix)]
pub fn ssh_auth_sock() -> Option<&'static str> {
    login_env().and_then(|env| env.ssh_auth_sock.as_deref())
}

#[cfg(not(unix))]
pub fn ssh_auth_sock() -> Option<&'static str> {
    None
}

/// The one run of the login shell, made by the first caller.
#[cfg(unix)]
fn login_env() -> Option<&'static LoginEnv> {
    use std::sync::OnceLock;
    use std::time::Duration;

    const TIMEOUT: Duration = Duration::from_secs(3);
    static ENV: OnceLock<Option<LoginEnv>> = OnceLock::new();
    ENV.get_or_init(|| {
        let shell = login_shell(std::env::var("SHELL").ok())?;
        let marker = format!("__hatoba_{}__", uuid::Uuid::now_v7().simple());
        let env = run_login_shell(&shell, &marker, TIMEOUT);
        if env.is_none() {
            tracing::warn!("the login shell did not answer");
        }
        env
    })
    .as_ref()
}

/// `$SHELL` when it is an absolute path, else the login shell the platform has by default: zsh
/// on macOS. Linux has none to assume, so there the shell does not run.
#[cfg(any(unix, test))]
fn login_shell(shell: Option<String>) -> Option<String> {
    const DEFAULT: Option<&str> = if cfg!(target_os = "macos") {
        Some("/bin/zsh")
    } else {
        None
    };
    shell
        .filter(|s| s.starts_with('/'))
        .or_else(|| DEFAULT.map(str::to_owned))
}

/// The values `printf` printed between three `marker`s, ignoring whatever the rc files wrote
/// around them: `PATH`, then `SSH_AUTH_SOCK`.
#[cfg(any(unix, test))]
fn parse_marked(output: &str, marker: &str) -> Option<(Option<String>, Option<String>)> {
    let mut parts = output.split(marker);
    parts.next()?;
    let value = |v: &str| Some(v.trim()).filter(|v| !v.is_empty()).map(str::to_owned);
    let path = value(parts.next()?);
    let ssh_auth_sock = value(parts.next()?);
    // The closing marker must follow, or the shell was cut off.
    parts.next()?;
    Some((path, ssh_auth_sock))
}

/// Runs `shell -ilc` (interactive login, so `.zprofile`, `.zshrc` and the like are read) and
/// returns its `PATH` and `SSH_AUTH_SOCK`; `None` when it fails, prints nothing usable, or takes
/// longer than `timeout`. It runs with no stdin, its stderr dropped, and in a session of its own,
/// so giving up kills whatever its rc files started along with it.
#[cfg(unix)]
fn run_login_shell(shell: &str, marker: &str, timeout: std::time::Duration) -> Option<LoginEnv> {
    use std::io::Read;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut command = Command::new(shell);
    command
        .args([
            "-ilc",
            &format!("printf '%s%s%s%s%s' {marker} \"$PATH\" {marker} \"$SSH_AUTH_SOCK\" {marker}"),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    // A session rather than only a process group: with no controlling terminal, an interactive
    // shell does not stop on SIGTTIN when the app itself runs in a terminal (the `hatoba`
    // command, `tauri dev`). Its process group id is still its pid, which `kill_group` uses.
    // SAFETY: `setsid` is async-signal-safe, which is all `pre_exec` may call.
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = command.spawn().ok()?;
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
    let (path, ssh_auth_sock) = parse_marked(&String::from_utf8_lossy(&output), marker)?;
    Some(LoginEnv {
        path,
        ssh_auth_sock,
    })
}

/// Kills the process group that `run_login_shell` started the shell in (its id is the shell's).
#[cfg(unix)]
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
    fn the_values_are_read_between_the_markers() {
        let out = format!(
            "Last login: today\nmotd\n{M}/opt/homebrew/bin:/usr/bin{M}/tmp/agent.sock{M}\n"
        );
        assert_eq!(
            parse_marked(&out, M),
            Some((
                Some("/opt/homebrew/bin:/usr/bin".into()),
                Some("/tmp/agent.sock".into())
            ))
        );
        // A shell that exports no `SSH_AUTH_SOCK` prints nothing between the last two markers.
        assert_eq!(
            parse_marked(&format!("{M}/usr/bin{M}{M}"), M),
            Some((Some("/usr/bin".into()), None))
        );
    }

    #[test]
    fn output_without_a_complete_marked_value_is_rejected() {
        assert_eq!(parse_marked("", M), None);
        assert_eq!(parse_marked("/usr/bin", M), None);
        assert_eq!(parse_marked(&format!("{M}/usr/bin"), M), None);
        assert_eq!(parse_marked(&format!("{M}/usr/bin{M}/tmp/a"), M), None);
        assert_eq!(
            parse_marked(&format!("{M}  {M}  {M}"), M),
            Some((None, None))
        );
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
        let default = cfg!(target_os = "macos").then(|| "/bin/zsh".to_owned());
        assert_eq!(
            login_shell(Some("/bin/bash".into())).as_deref(),
            Some("/bin/bash")
        );
        assert_eq!(login_shell(Some("zsh".into())), default);
        assert_eq!(login_shell(Some(String::new())), default);
        assert_eq!(login_shell(None), default);
    }

    #[cfg(unix)]
    mod shell {
        use std::os::unix::fs::PermissionsExt;
        use std::time::{Duration, Instant};

        use super::super::{LoginEnv, run_login_shell};

        /// A script standing in for the login shell; it gets `-ilc <script>` like a real one.
        fn fake_shell(name: &str, body: &str) -> String {
            let path =
                std::env::temp_dir().join(format!("hatoba-test-{name}-{}", std::process::id()));
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path.to_string_lossy().into_owned()
        }

        #[test]
        fn the_values_survive_noise_from_the_rc_files() {
            let shell = fake_shell(
                "noisy",
                "echo 'banner'; echo 'oops' >&2; PATH=/opt/x/bin:/usr/bin; \
                 SSH_AUTH_SOCK=/tmp/bw.sock; eval \"$2\"",
            );
            let env = run_login_shell(&shell, "__m__", Duration::from_secs(5));
            std::fs::remove_file(&shell).unwrap();
            assert_eq!(
                env,
                Some(LoginEnv {
                    path: Some("/opt/x/bin:/usr/bin".into()),
                    ssh_auth_sock: Some("/tmp/bw.sock".into()),
                })
            );
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

        #[cfg(target_os = "macos")]
        #[test]
        fn the_real_login_shell_reports_a_path() {
            let env = run_login_shell("/bin/zsh", "__hatoba_real__", Duration::from_secs(10));
            assert!(
                env.and_then(|e| e.path)
                    .is_some_and(|p| p.contains("/usr/bin"))
            );
        }
    }
}
