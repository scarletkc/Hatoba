//! The `sh` of this computer, for a local tab (spec §7.5): the AI assistant's file and search tools
//! run their scripts with it ([`Exec`]), and `run_command` its command script
//! ([`crate::remote_command::run_local`]), as they run them on a host over SSH. On macOS and Linux
//! that is `/bin/sh`; for a WSL distribution, its `sh` through `wsl.exe`, so they act inside the
//! distribution.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Stdio;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};

use crate::error::SshError;
use crate::exec::Exec;
use crate::session::{ExecOutput, MAX_EXEC_OUTPUT};

/// How to start `sh` on this computer.
#[derive(Clone, Debug)]
pub struct LocalSh {
    program: PathBuf,
    /// Arguments before `sh`'s own, such as `-d <distribution> -e sh` for `wsl.exe`.
    prefix: Vec<OsString>,
    env: Vec<(OsString, OsString)>,
    cwd: Option<PathBuf>,
}

impl LocalSh {
    /// `program`, started with `prefix` before `sh`'s arguments, with `env` added to the app's
    /// environment, in `cwd`.
    #[must_use]
    pub fn new(
        program: impl Into<PathBuf>,
        prefix: Vec<OsString>,
        env: Vec<(OsString, OsString)>,
        cwd: Option<PathBuf>,
    ) -> Self {
        Self {
            program: program.into(),
            prefix,
            env,
            cwd,
        }
    }

    /// The program it starts.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }

    /// `sh` with `args`, its standard streams piped, in a process group of its own on Unix (so a
    /// search's watchdog that ends its group never reaches Hatoba), without a console window on
    /// Windows, and killed when the handle is dropped.
    pub(crate) fn command(&self, args: &[&OsStr]) -> Command {
        let mut command = Command::new(&self.program);
        command
            .args(&self.prefix)
            .args(args)
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(cwd) = &self.cwd {
            command.current_dir(cwd);
        }
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        {
            /// `CREATE_NO_WINDOW`: `wsl.exe` would open a console window of its own.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }
        command
    }

    /// Starts `sh` with `args`.
    pub(crate) fn spawn(&self, args: &[&OsStr]) -> Result<Child, SshError> {
        self.command(args)
            .spawn()
            .map_err(|e| SshError::other(format!("cannot start {}: {e}", self.program.display())))
    }
}

impl Exec for LocalSh {
    async fn exec_output(
        &self,
        command: &str,
        stdin: Option<Vec<u8>>,
    ) -> Result<ExecOutput, SshError> {
        let mut child = self.spawn(&[OsStr::new("-c"), OsStr::new(command)])?;
        let input = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let feed = async move {
            if let (Some(mut input), Some(data)) = (input, stdin) {
                // A script that stops reading early closes the pipe; what it read is what counts.
                let _ = input.write_all(&data).await;
            }
        };
        let (out, err, (), status) =
            tokio::join!(read_all(stdout), read_all(stderr), feed, child.wait());
        let status = status.map_err(|e| SshError::other(format!("sh failed: {e}")))?;
        Ok(ExecOutput {
            exit_status: status.code().and_then(|c| u32::try_from(c).ok()),
            stdout: out?,
            stderr: err?,
        })
    }
}

/// Everything a pipe gives, at most [`MAX_EXEC_OUTPUT`] bytes.
async fn read_all(pipe: Option<impl AsyncRead + Unpin>) -> Result<Vec<u8>, SshError> {
    let mut out = Vec::new();
    let Some(mut pipe) = pipe else {
        return Ok(out);
    };
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match pipe.read(&mut buf).await {
            Ok(0) | Err(_) => return Ok(out),
            Ok(n) => {
                if out.len() + n > MAX_EXEC_OUTPUT {
                    return Err(SshError::other("command output is too large"));
                }
                out.extend_from_slice(&buf[..n]);
            }
        }
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn sh() -> LocalSh {
        LocalSh::new("/bin/sh", Vec::new(), Vec::new(), Some(PathBuf::from("/")))
    }

    #[tokio::test]
    async fn runs_a_command_line_with_stdin() {
        let out = sh()
            .exec_output(
                "read -r a; echo got $a; echo err >&2; exit 4",
                Some(b"x\n".to_vec()),
            )
            .await
            .unwrap();
        assert_eq!(out.exit_status, Some(4));
        assert_eq!(out.stdout, b"got x\n");
        assert_eq!(out.stderr, b"err\n");
    }
}

/// A WSL distribution's `sh`, when `HATOBA_WSL_DISTRO` names one (Windows only).
#[cfg(all(test, windows))]
mod wsl_tests {
    use super::*;
    use crate::remote_command::{Stopped, run_local};
    use crate::remote_file;

    fn distro() -> Option<LocalSh> {
        let distro = std::env::var("HATOBA_WSL_DISTRO").ok()?;
        let wsl = PathBuf::from(std::env::var_os("SystemRoot")?).join(r"System32\wsl.exe");
        Some(LocalSh::new(
            wsl,
            ["-d", distro.as_str(), "-e", "sh"]
                .map(OsString::from)
                .to_vec(),
            Vec::new(),
            None,
        ))
    }

    #[tokio::test]
    async fn the_scripts_reach_the_distribution_unchanged() {
        let Some(sh) = distro() else { return };
        let out = sh
            .exec_output(
                "printf '%s|' \"a b\" 'c\"d' $HOME; cat",
                Some(b"in\n".to_vec()),
            )
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.starts_with("a b|c\"d|/"),
            "{text:?} {:?}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(text.ends_with("|in\n"), "{text:?}");
        let file = remote_file::read(&sh, "/etc/hostname", 4096).await;
        assert!(file.is_ok(), "{file:?}");
    }

    #[tokio::test]
    async fn a_command_runs_and_a_stop_ends_it() {
        let Some(sh) = distro() else { return };
        let run = run_local(&sh, "echo hi; exit 3", std::future::pending())
            .await
            .unwrap();
        assert_eq!(run.output.exit_status, Some(3));
        assert_eq!(run.output.stdout, b"hi\n");
        assert!(run.stopped.is_none());
        let stop = tokio::time::sleep(std::time::Duration::from_millis(800));
        let run = run_local(&sh, "echo start; sleep 30", stop).await.unwrap();
        assert_eq!(run.stopped, Some(Stopped::Ended), "{run:?}");
        assert!(String::from_utf8_lossy(&run.output.stdout).contains("start"));
    }
}
