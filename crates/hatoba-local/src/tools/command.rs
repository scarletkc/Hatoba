//! `run_command` in a Windows shell (AI-12, §7.5): PowerShell, Command Prompt or Git Bash, without
//! a console window, in a Job Object, so a stop or the timeout ends the command and every process
//! it started. The hosts' command script needs `sh`, which these shells are not.
//!
//! The command runs non-interactively, without the shell's profile, like a command over SSH,
//! with UTF-8 output: PowerShell through `-EncodedCommand`, so no quoting is involved; Command
//! Prompt in a second Command Prompt started after `chcp 65001`, which gets the line from an
//! environment variable, so the first one does not parse it; Git Bash, not as a login shell, with
//! the command on stdin, which it reads before the command runs, so the command's own stdin is
//! empty as on a host.
//!
//! A command that ends on its own leaves the processes it put in the background running, until
//! Hatoba quits: closing the Job Object would end them.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use base64::Engine;
use hatoba_ssh::remote_command::{CommandRun, Stopped};
use hatoba_ssh::session::ExecOutput;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::Instant;

/// The most output a command keeps of each stream, as over SSH.
const MAX_OUTPUT: usize = 64 * 1024 * 1024;
/// How long a stop waits for the Job Object's processes to be gone.
const KILL_WAIT: Duration = Duration::from_secs(3);

/// A Windows shell that runs `run_command`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeShell {
    PowerShell(PathBuf),
    Cmd(PathBuf),
    GitBash(PathBuf),
}

/// What PowerShell runs before the command: no progress bars, which would reach stderr as CLIXML,
/// and UTF-8 for what it prints and what it reads from programs. After it, the exit status of a
/// failed last command: a program's own, else 1.
const POWERSHELL_PRELUDE: &str = "$ProgressPreference = 'SilentlyContinue'\n\
     [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)\n\
     $OutputEncoding = [Console]::OutputEncoding\n";
const POWERSHELL_EPILOGUE: &str =
    "\nif (-not $?) { if ($LASTEXITCODE) { exit $LASTEXITCODE } else { exit 1 } }\n";

impl NativeShell {
    /// The shell's command for `command`, and what goes to its stdin.
    fn command(
        &self,
        command: &str,
        cwd: &Path,
    ) -> Result<(tokio::process::Command, Option<Vec<u8>>), String> {
        let mut stdin = None;
        let mut cmd = match self {
            Self::PowerShell(program) => {
                let script = format!("{POWERSHELL_PRELUDE}{command}{POWERSHELL_EPILOGUE}");
                let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
                let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
                // CreateProcess takes 32,767 characters in all.
                if encoded.len() > 30_000 {
                    return Err("The command is too long for PowerShell's command line; split it \
                                into shorter commands, or write a script with write_file and run it."
                        .to_owned());
                }
                let mut cmd = tokio::process::Command::new(program);
                cmd.args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-EncodedCommand",
                    &encoded,
                ]);
                cmd
            }
            Self::Cmd(program) => {
                if command.contains(['\n', '\r']) {
                    return Err(
                        "Command Prompt runs one line: join the commands with &, or use \
                                a batch file."
                            .to_owned(),
                    );
                }
                let mut cmd = tokio::process::Command::new(program);
                // `/s` keeps the line as it is between the outer quotes. Command Prompt's own
                // commands, such as `ver` and `dir`, print in the code page it started with, so the
                // command runs in a second one, started after `chcp`. The first one expands the
                // variable after it parsed its own line (`/v:on`), so the command's quotes, `&` and
                // `^` reach the second one as they were.
                cmd.env(COMMAND_VAR, command);
                cmd_line(
                    &mut cmd,
                    &format!(
                        "/d /v:on /s /c \"chcp 65001 >nul & \"{}\" /d /s /c \"!{COMMAND_VAR}!\"\"",
                        program.display()
                    ),
                );
                cmd
            }
            Self::GitBash(program) => {
                stdin = Some(command.as_bytes().to_vec());
                let mut cmd = tokio::process::Command::new(program);
                // `bin\bash.exe` sets PATH, HOME and MSYSTEM itself, as a login shell would.
                cmd.args(["-c", "eval \"$(cat)\""]);
                cmd
            }
        };
        cmd.current_dir(cwd)
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        Ok((cmd, stdin))
    }
}

/// The environment variable that takes the command to the second Command Prompt.
const COMMAND_VAR: &str = "HATOBA_COMMAND";

#[cfg(windows)]
fn cmd_line(cmd: &mut tokio::process::Command, line: &str) {
    cmd.raw_arg(line);
}

#[cfg(not(windows))]
fn cmd_line(cmd: &mut tokio::process::Command, line: &str) {
    cmd.arg(line);
}

fn wrap(cmd: tokio::process::Command) -> CommandWrap {
    let mut wrap = CommandWrap::from(cmd);
    wrap.wrap(KillOnDrop);
    #[cfg(windows)]
    {
        use process_wrap::tokio::{CreationFlags, JobObject};
        use windows::Win32::System::Threading::CREATE_NO_WINDOW;
        wrap.wrap(CreationFlags(CREATE_NO_WINDOW));
        wrap.wrap(JobObject);
    }
    #[cfg(unix)]
    wrap.wrap(process_wrap::tokio::ProcessGroup::leader());
    wrap
}

/// Runs `command` in `shell`, in `cwd`, and returns what it printed. When `stop` completes first,
/// the command and every process it started are ended, and the result says whether they were.
pub async fn run(
    shell: &NativeShell,
    command: &str,
    cwd: &Path,
    stop: impl Future<Output = ()>,
) -> Result<CommandRun, String> {
    if command.contains('\0') {
        return Err("The command has a NUL character.".to_owned());
    }
    let (cmd, input) = shell.command(command, cwd)?;
    let mut child: Box<dyn ChildWrapper> = wrap(cmd)
        .spawn()
        .map_err(|e| format!("The shell could not start: {e}"))?;
    let stdin = child.stdin().take();
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout().take(), child.stderr().take())
    else {
        return Err("The shell has no pipes.".to_owned());
    };
    if let (Some(mut pipe), Some(data)) = (stdin, input) {
        // Git Bash reads all of it before the command runs; closing it ends the command's stdin.
        let _ = pipe.write_all(&data).await;
    }
    tokio::pin!(stop);
    let mut output = ExecOutput::default();
    let (mut out_buf, mut err_buf) = (vec![0u8; 32 * 1024], vec![0u8; 32 * 1024]);
    let (mut out_open, mut err_open) = (true, true);
    let mut exited: Option<Option<u32>> = None;
    let mut drain: Option<Instant> = None;
    let mut stopped = None;
    loop {
        tokio::select! {
            biased;
            () = &mut stop, if exited.is_none() => {
                let _ = child.start_kill();
                let gone = tokio::time::timeout(KILL_WAIT, child.wait()).await;
                stopped = Some(if gone.is_ok() { Stopped::Ended } else { Stopped::MayRun });
                if let Ok(Ok(status)) = gone {
                    exited = Some(status.code().and_then(|c| u32::try_from(c).ok()));
                }
                break;
            }
            read = stdout.read(&mut out_buf), if out_open => match read {
                Ok(0) | Err(_) => out_open = false,
                Ok(n) => push(&mut output.stdout, &out_buf[..n])?,
            },
            read = stderr.read(&mut err_buf), if err_open => match read {
                Ok(0) | Err(_) => err_open = false,
                Ok(n) => push(&mut output.stderr, &err_buf[..n])?,
            },
            // The shell's own process: the background may hold the pipes, and the Job Object's
            // wait, for as long as it runs.
            status = child.inner_mut().wait(), if exited.is_none() => {
                exited = Some(status.ok().and_then(|s| s.code()).and_then(|c| u32::try_from(c).ok()));
                drain = Some(Instant::now() + Duration::from_millis(200));
            }
            () = sleep_until(drain), if drain.is_some() => break,
        }
        if exited.is_some() && !out_open && !err_open {
            break;
        }
    }
    output.exit_status = exited.flatten();
    if stopped.is_none() {
        // What the command left running keeps running until Hatoba quits.
        std::mem::forget(child);
    }
    Ok(CommandRun { output, stopped })
}

fn push(to: &mut Vec<u8>, data: &[u8]) -> Result<(), String> {
    if to.len() + data.len() > MAX_OUTPUT {
        return Err("The command's output is too large.".to_owned());
    }
    to.extend_from_slice(data);
    Ok(())
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::shells::{self, ShellKind};

    fn shell(kind: ShellKind) -> Option<NativeShell> {
        let found = shells::detect().into_iter().find(|s| s.kind == kind)?;
        Some(match kind {
            ShellKind::PowerShell => NativeShell::PowerShell(found.program),
            ShellKind::Cmd => NativeShell::Cmd(found.program),
            ShellKind::GitBash => NativeShell::GitBash(found.program),
            _ => return None,
        })
    }

    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).replace("\r\n", "\n")
    }

    #[tokio::test]
    async fn powershell_prints_utf8_and_returns_the_exit_status() {
        let ps = shell(ShellKind::PowerShell).unwrap();
        let home = std::env::temp_dir();
        let run = run(
            &ps,
            "Write-Output '波止場'; cmd /c exit 7",
            &home,
            std::future::pending(),
        )
        .await
        .unwrap();
        assert_eq!(text(&run.output.stdout), "波止場\n");
        assert_eq!(run.output.exit_status, Some(7));
        assert!(run.stopped.is_none());
        let run = run_ps_error(&ps, &home).await;
        assert_eq!(run.output.exit_status, Some(1));
        assert!(!run.output.stderr.is_empty());
    }

    async fn run_ps_error(ps: &NativeShell, home: &Path) -> CommandRun {
        run(
            ps,
            "Get-Item C:\\no\\such\\file",
            home,
            std::future::pending(),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn cmd_runs_one_line() {
        let cmd = shell(ShellKind::Cmd).unwrap();
        let home = std::env::temp_dir();
        let run = run(
            &cmd,
            "echo a \"b c\" & exit /b 2",
            &home,
            std::future::pending(),
        )
        .await
        .unwrap();
        assert_eq!(text(&run.output.stdout).trim_end(), "a \"b c\"");
        assert_eq!(run.output.exit_status, Some(2));
        // Quotes, carets, `!` and `%` mean what they mean when typed at the prompt.
        let run = super::run(
            &cmd,
            "echo \"a & b|c\" x^&y! %OS%",
            &home,
            std::future::pending(),
        )
        .await
        .unwrap();
        assert_eq!(
            text(&run.output.stdout).trim_end(),
            "\"a & b|c\" x&y! Windows_NT"
        );
        // Command Prompt's own commands print UTF-8 too, whatever the system's code page.
        let run = super::run(&cmd, "ver", &home, std::future::pending())
            .await
            .unwrap();
        let out = std::str::from_utf8(&run.output.stdout).expect("UTF-8");
        assert!(
            out.contains("10.0.") && !out.contains('\u{fffd}'),
            "{out:?}"
        );
        assert!(
            super::run(&cmd, "echo a\necho b", &home, std::future::pending())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn a_stop_ends_the_command_and_what_it_started() {
        let ps = shell(ShellKind::PowerShell).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("started");
        // The stop comes once the command has started, however long PowerShell takes to.
        let stop = async {
            while !marker.exists() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        };
        let started = Instant::now();
        let run = run(
            &ps,
            &format!(
                "Write-Output start; New-Item -ItemType File '{}' | Out-Null; \
                 ping -n 30 127.0.0.1 >$null",
                marker.display()
            ),
            dir.path(),
            stop,
        )
        .await
        .unwrap();
        assert_eq!(run.stopped, Some(Stopped::Ended));
        // ping alone takes 29 seconds.
        assert!(started.elapsed() < Duration::from_secs(20));
        assert!(text(&run.output.stdout).contains("start"));
    }

    #[tokio::test]
    async fn git_bash_reads_the_command_from_stdin() {
        let Some(bash) = shell(ShellKind::GitBash) else {
            return;
        };
        let home = std::env::temp_dir();
        let run = run(
            &bash,
            "printf '%s|' \"a b\" 'c\"d'; read -r x; echo \"[$x]\"; exit 5",
            &home,
            std::future::pending(),
        )
        .await
        .unwrap();
        assert_eq!(text(&run.output.stdout), "a b|c\"d|[]\n");
        assert_eq!(run.output.exit_status, Some(5));
    }
}
