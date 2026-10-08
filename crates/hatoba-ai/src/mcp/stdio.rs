//! Starting a stdio server (AI-32): command lookup on `PATH` (with `PATHEXT` on Windows),
//! spawning without a console window inside a Job Object (Windows) or a new process group (Unix),
//! the in-memory stderr ring, and stopping the whole process tree.
//!
//! `.cmd` and `.bat` files (such as `npx.cmd`) are found through `PATHEXT` and started by the
//! standard library, which runs them through `cmd.exe /c` with their arguments escaped, and
//! refuses arguments that `cmd.exe` cannot receive safely.

use std::collections::VecDeque;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout};
use zeroize::Zeroizing;

use super::McpError;

/// Lines of stderr kept per server (AI-32).
pub const STDERR_LINES: usize = 50;
/// Bytes kept of one stderr line; the rest of a longer line is dropped.
const MAX_LINE_BYTES: usize = 2048;
/// Characters kept of one cleaned stderr line.
const MAX_LINE_CHARS: usize = 500;
/// How long a stop waits for the tree to exit after the kill.
const KILL_WAIT: Duration = Duration::from_secs(2);

/// The last [`STDERR_LINES`] lines a stdio server wrote to stderr, in memory only (never logged).
/// Lines are decoded leniently, with terminal escape sequences and control characters removed.
///
/// Cloning shares the buffer, so the shell can keep one to show after a failed start
/// ([`super::McpConnection::connect_with_stderr`]). `Debug` prints only the line count.
#[derive(Clone, Default)]
pub struct StderrTail(Arc<Mutex<VecDeque<String>>>);

impl StderrTail {
    /// An empty buffer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The kept lines, oldest first.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .cloned()
            .collect()
    }

    fn push(&self, raw: &[u8]) {
        let line = clean_line(raw);
        if line.is_empty() {
            return;
        }
        let mut lines = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if lines.len() == STDERR_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }
}

impl fmt::Debug for StderrTail {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.0.lock().unwrap_or_else(PoisonError::into_inner).len();
        f.debug_struct("StderrTail").field("lines", &count).finish()
    }
}

/// One stderr line as text: lossy UTF-8, ANSI escape sequences (CSI, OSC and two-byte escapes)
/// and other control characters removed, tabs as spaces, cut to [`MAX_LINE_CHARS`].
fn clean_line(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let mut out = String::new();
    let mut count = 0;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.next() {
                // CSI: parameters and intermediates up to a final byte in @..~.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: up to BEL or ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let c = if c == '\t' { ' ' } else { c };
        if c.is_control() {
            continue;
        }
        if count == MAX_LINE_CHARS {
            out.push('…');
            break;
        }
        out.push(c);
        count += 1;
    }
    out.trim_end().to_owned()
}

/// Reads stderr to its end into `tail`. A lone carriage return starts the line over, as a
/// terminal would show progress output.
async fn read_stderr(mut stderr: impl AsyncRead + Unpin, tail: StderrTail) {
    let mut buf = [0u8; 8192];
    let mut line = Vec::new();
    let mut pending_cr = false;
    loop {
        let n = match stderr.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        for &b in &buf[..n] {
            if pending_cr {
                pending_cr = false;
                if b == b'\n' {
                    tail.push(&line);
                    line.clear();
                    continue;
                }
                line.clear();
            }
            match b {
                b'\n' => {
                    tail.push(&line);
                    line.clear();
                }
                b'\r' => pending_cr = true,
                _ if line.len() < MAX_LINE_BYTES => line.push(b),
                _ => {}
            }
        }
    }
    tail.push(&line);
}

/// Starts copying a child's stderr into `tail` on the current runtime; the task ends with the
/// stream.
pub(crate) fn capture_stderr(stderr: ChildStderr, tail: StderrTail) {
    tokio::spawn(read_stderr(stderr, tail));
}

/// Whether two environment variable names are the same variable (Windows ignores case).
fn same_var(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// The `PATH` the child gets: the last `PATH` in `env`, else this process's.
fn child_path(env: &[(String, Zeroizing<String>)]) -> Option<OsString> {
    env.iter()
        .rev()
        .find(|(name, _)| same_var(name, "PATH"))
        .map(|(_, value)| OsString::from(value.as_str()))
        .or_else(|| std::env::var_os("PATH"))
}

/// Finds `command` the way a shell does: a path (absolute, or relative to the current
/// directory) is used as it is, a bare name is looked up on the child's `PATH`, and on Windows
/// the `PATHEXT` extensions are tried, so `npx` finds `npx.cmd`.
pub(crate) fn find_command(
    command: &str,
    env: &[(String, Zeroizing<String>)],
) -> Result<PathBuf, McpError> {
    let command = command.trim();
    if command.is_empty() {
        return Err(McpError::InvalidConfig("the command is empty".into()));
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    which::which_in(command, child_path(env), cwd)
        .map_err(|_| McpError::CommandNotFound(command.to_owned()))
}

/// Checks the variables before they reach `Command`, which would fail later with a less clear
/// error. Only names are named in messages.
fn check_env(env: &[(String, Zeroizing<String>)]) -> Result<(), McpError> {
    for (name, value) in env {
        if name.is_empty() || name.contains(['=', '\0']) {
            return Err(McpError::InvalidConfig(format!(
                "\"{name}\" is not a valid environment variable name"
            )));
        }
        if value.contains('\0') {
            return Err(McpError::InvalidConfig(format!(
                "the value of {name} contains a NUL character"
            )));
        }
    }
    Ok(())
}

/// A spawned server: its process tree and its pipes.
pub(crate) struct Spawned {
    pub child: ChildGuard,
    pub stdin: ChildStdin,
    pub stdout: ChildStdout,
    pub stderr: Option<ChildStderr>,
}

/// Starts `program` with piped stdio, the inherited environment plus `env`, no console window
/// (Windows), and its tree contained so [`ChildGuard`] can kill all of it: a Job Object that
/// also kills the tree if Hatoba itself exits (Windows), or a new process group (Unix).
///
/// The environment values are copied into the `Command` (dropped right after the spawn) and the
/// new process's environment block, neither of which can be wiped; nothing else keeps them.
pub(crate) fn spawn(
    program: &Path,
    args: &[String],
    env: &[(String, Zeroizing<String>)],
) -> Result<Spawned, McpError> {
    check_env(env)?;
    let mut command = tokio::process::Command::new(program);
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in env {
        command.env(name, value.as_str());
    }
    let mut wrap = CommandWrap::from(command);
    wrap.wrap(KillOnDrop);
    #[cfg(windows)]
    {
        use process_wrap::tokio::{CreationFlags, JobObject};
        use windows::Win32::System::Threading::CREATE_NO_WINDOW;
        // `CreationFlags` keeps the flag when `JobObject` adds its own (a flag set on the
        // command directly would be overwritten).
        wrap.wrap(CreationFlags(CREATE_NO_WINDOW));
        wrap.wrap(JobObject);
    }
    #[cfg(unix)]
    wrap.wrap(process_wrap::tokio::ProcessGroup::leader());

    let spawned = wrap.spawn();
    drop(wrap);
    let mut child = spawned.map_err(|e| spawn_error(&e))?;
    let pipes = (
        child.stdin().take(),
        child.stdout().take(),
        child.stderr().take(),
    );
    let mut child = ChildGuard(Some(child));
    match pipes {
        (Some(stdin), Some(stdout), stderr) => Ok(Spawned {
            child,
            stdin,
            stdout,
            stderr,
        }),
        _ => {
            child.kill_now();
            Err(McpError::Spawn(
                "the child's pipes are not available".into(),
            ))
        }
    }
}

fn spawn_error(err: &std::io::Error) -> McpError {
    match err.kind() {
        std::io::ErrorKind::NotFound => McpError::Spawn("the program file was not found".into()),
        std::io::ErrorKind::PermissionDenied => {
            McpError::Spawn("permission denied: the program cannot be run".into())
        }
        // Raised by the standard library for a .cmd or .bat file with an argument cmd.exe cannot
        // receive intact, such as one with a line break.
        std::io::ErrorKind::InvalidInput => McpError::Spawn(format!(
            "the arguments cannot be passed to this program: {err}"
        )),
        _ => McpError::Spawn(err.to_string()),
    }
}

/// Owns a server's process tree and kills all of it when dropped.
pub(crate) struct ChildGuard(Option<Box<dyn ChildWrapper>>);

impl ChildGuard {
    /// The direct child's exit status if it exits within `wait` (used to explain a failed start).
    pub async fn exit_status_within(&mut self, wait: Duration) -> Option<ExitStatus> {
        let child = self.0.as_mut()?;
        // The direct child only: waiting on the whole tree could outlast `wait` for a child that
        // left a process behind.
        tokio::time::timeout(wait, child.inner_mut().wait())
            .await
            .ok()?
            .ok()
    }

    /// Waits up to `grace` for the tree to exit by itself (its stdin is already closed), then
    /// kills whatever is left of it and waits for that.
    pub async fn stop(&mut self, grace: Duration) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        let _ = tokio::time::timeout(grace, child.wait()).await;
        // Also after a clean exit: the server may have left processes behind in its tree.
        let _ = child.start_kill();
        let _ = tokio::time::timeout(KILL_WAIT, child.wait()).await;
    }

    /// Kills the tree without waiting (also what dropping does).
    pub fn kill_now(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.start_kill();
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill_now();
    }
}

/// "exited with code 1", or the signal on Unix.
pub(crate) fn describe_exit(status: ExitStatus) -> String {
    if let Some(code) = status.code() {
        return format!("exited with code {code}");
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return format!("was stopped by signal {signal}");
        }
    }
    "exited".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_lines_are_cleaned() {
        assert_eq!(clean_line(b"\x1b[31mred\x1b[0m text"), "red text");
        assert_eq!(clean_line(b"\x1b]0;title\x07after"), "after");
        assert_eq!(clean_line(b"\x1b]8;;http://x\x1b\\link"), "link");
        assert_eq!(clean_line(b"a\tb\x07c  "), "a bc");
        assert_eq!(clean_line(b"bad \xff utf8"), "bad \u{fffd} utf8");
        let long = "x".repeat(MAX_LINE_CHARS + 10);
        assert_eq!(
            clean_line(long.as_bytes()).chars().count(),
            MAX_LINE_CHARS + 1
        );
    }

    #[tokio::test]
    async fn stderr_ring_keeps_the_last_lines() {
        let mut text = String::from("progress 10%\rprogress 100%\r\nignored blank:\n\n");
        for i in 1..=60 {
            text.push_str(&format!("line {i}\n"));
        }
        text.push_str("no newline at the end");
        let tail = StderrTail::new();
        read_stderr(text.as_bytes(), tail.clone()).await;
        let lines = tail.lines();
        assert_eq!(lines.len(), STDERR_LINES);
        assert_eq!(lines[0], "line 12");
        assert_eq!(lines[STDERR_LINES - 1], "no newline at the end");
        assert!(format!("{tail:?}").contains("lines: 50"));

        let tail = StderrTail::new();
        read_stderr(
            &b"progress 10%\rprogress 100%\r\nignored blank:\n\n"[..],
            tail.clone(),
        )
        .await;
        assert_eq!(tail.lines(), ["progress 100%", "ignored blank:"]);
    }

    #[test]
    fn env_names_and_values_are_checked() {
        let ok = vec![("API_KEY".to_owned(), Zeroizing::new("v".to_owned()))];
        assert!(check_env(&ok).is_ok());
        for name in ["", "A=B", "A\0"] {
            let bad = vec![(name.to_owned(), Zeroizing::new("v".to_owned()))];
            assert!(matches!(check_env(&bad), Err(McpError::InvalidConfig(_))));
        }
        let bad = vec![("K".to_owned(), Zeroizing::new("se\0cret".to_owned()))];
        let err = check_env(&bad).unwrap_err().to_string();
        assert!(!err.contains("se"), "{err}");
    }

    #[test]
    fn path_comes_from_env_when_given() {
        let name = if cfg!(windows) { "Path" } else { "PATH" };
        let env = vec![(name.to_owned(), Zeroizing::new("/custom/bin".to_owned()))];
        assert_eq!(child_path(&env), Some(OsString::from("/custom/bin")));
        assert_eq!(child_path(&[]), std::env::var_os("PATH"));
        assert!(matches!(
            find_command("  ", &[]),
            Err(McpError::InvalidConfig(_))
        ));
        assert_eq!(
            find_command("hatoba-no-such-command-x9", &[]),
            Err(McpError::CommandNotFound(
                "hatoba-no-such-command-x9".into()
            ))
        );
    }
}
