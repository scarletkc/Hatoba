//! The AI assistant's tools on this computer, for a local tab (spec §7.5): `run_command`, the file
//! tools, and `glob` and `grep`, with the results the same tools give on a host.
//!
//! A tab whose shell has `sh` behind it (macOS's and Linux's shells, and a WSL distribution) runs
//! the hosts' own scripts with it, so the tools act as they do over SSH, inside the distribution
//! for WSL. A Windows shell (PowerShell, Command Prompt, Git Bash) gets native file and search
//! tools on Windows paths, and runs `run_command` in the tab's shell ([`command`]).

pub mod command;
pub mod files;
pub mod search;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hatoba_ssh::local_sh::LocalSh;
use hatoba_ssh::remote_command::{self, CommandRun, Stopped};
use hatoba_ssh::remote_file::{self, Expect, RemoteDir, RemoteFile, RemoteFileError, RemoteLines};
use hatoba_ssh::remote_search::{self, GrepQuery, RemoteSearchError, Scope, SearchOutput};
use hatoba_ssh::session::ExecOutput;
use hatoba_ssh::{SshError, SshErrorKind};

use crate::shells::{Shell, ShellKind};
use command::NativeShell;

/// How a local tab's tools run.
#[derive(Clone, Debug)]
pub enum LocalTools {
    /// With `sh`: the hosts' scripts.
    Sh {
        sh: LocalSh,
        /// How the record of what the model has seen and the file snapshots name the place: the
        /// computer, or a WSL distribution, whose files are its own.
        server: String,
    },
    /// On Windows itself.
    Native { shell: NativeShell, home: PathBuf },
}

/// How the tools' records name this computer's own files (AI-41, "What the model has seen").
pub const LOCAL_SERVER: &str = "local";

impl LocalTools {
    /// The tools for a tab running `shell`. `env` goes to the `sh` of a macOS or Linux shell, such
    /// as the `PATH` of the login shell for an app started from the Dock.
    #[must_use]
    pub fn for_shell(shell: &Shell, mut env: Vec<(OsString, OsString)>) -> Self {
        let home = dirs::home_dir().unwrap_or_default();
        match &shell.kind {
            ShellKind::Unix => {
                // The command script runs commands with `$SHELL`: the tab's shell.
                env.push(("SHELL".into(), shell.program.clone().into_os_string()));
                env.push(("HOME".into(), home.clone().into_os_string()));
                // From the home directory, as a command over SSH starts.
                let cwd = if home.is_dir() {
                    home.clone()
                } else {
                    PathBuf::from("/")
                };
                Self::Sh {
                    sh: LocalSh::new("/bin/sh", Vec::new(), env, Some(cwd)),
                    server: LOCAL_SERVER.to_owned(),
                }
            }
            ShellKind::Wsl { distro } => Self::Sh {
                sh: LocalSh::new(
                    shell.program.clone(),
                    // From the home directory, as a command over SSH starts.
                    ["-d", distro.as_str(), "--cd", "~", "-e", "sh"]
                        .map(OsString::from)
                        .to_vec(),
                    Vec::new(),
                    None,
                ),
                server: format!("wsl:{distro}"),
            },
            ShellKind::PowerShell => Self::Native {
                shell: NativeShell::PowerShell(shell.program.clone()),
                home,
            },
            ShellKind::Cmd => Self::Native {
                shell: NativeShell::Cmd(shell.program.clone()),
                home,
            },
            ShellKind::GitBash => Self::Native {
                shell: NativeShell::GitBash(shell.program.clone()),
                home,
            },
        }
    }

    /// How the records of the file tools name where they ran.
    #[must_use]
    pub fn server(&self) -> &str {
        match self {
            Self::Sh { server, .. } => server,
            Self::Native { .. } => LOCAL_SERVER,
        }
    }

    /// Whether the tools take Windows paths.
    #[must_use]
    pub fn windows_paths(&self) -> bool {
        matches!(self, Self::Native { .. })
    }

    pub async fn read(&self, path: &str, max: u64) -> Result<RemoteFile, RemoteFileError> {
        match self {
            Self::Sh { sh, .. } => remote_file::read(sh, path, max).await,
            Self::Native { home, .. } => {
                let (path, home) = (path.to_owned(), home.clone());
                blocking(move || files::read(&path, max, &home)).await
            }
        }
    }

    pub async fn read_lines(
        &self,
        path: &str,
        from: u64,
        max: u64,
    ) -> Result<RemoteLines, RemoteFileError> {
        match self {
            Self::Sh { sh, .. } => remote_file::read_lines(sh, path, from, max).await,
            Self::Native { home, .. } => {
                let (path, home) = (path.to_owned(), home.clone());
                blocking(move || files::read_lines(&path, from, max, &home)).await
            }
        }
    }

    pub async fn list_dir(&self, path: &str, max: u64) -> Result<RemoteDir, RemoteFileError> {
        match self {
            Self::Sh { sh, .. } => remote_file::list_dir(sh, path, max).await,
            Self::Native { home, .. } => {
                let (path, home) = (path.to_owned(), home.clone());
                blocking(move || files::list_dir(&path, max, &home)).await
            }
        }
    }

    pub async fn write(
        &self,
        path: &str,
        content: &[u8],
        expect: Expect<'_>,
        mode: Option<u32>,
    ) -> Result<String, RemoteFileError> {
        match self {
            Self::Sh { sh, .. } => remote_file::write(sh, path, content, expect, mode).await,
            Self::Native { home, .. } => {
                let (path, content, home) = (path.to_owned(), content.to_vec(), home.clone());
                let expect = match expect {
                    Expect::Any => Owned::Any,
                    Expect::Missing => Owned::Missing,
                    Expect::Content(bytes) => Owned::Content(bytes.to_vec()),
                };
                blocking(move || files::write(&path, &content, expect.borrow(), mode, &home)).await
            }
        }
    }

    pub async fn delete(&self, path: &str, expect: &[u8]) -> Result<String, RemoteFileError> {
        match self {
            Self::Sh { sh, .. } => remote_file::delete(sh, path, expect).await,
            Self::Native { home, .. } => {
                let (path, expect, home) = (path.to_owned(), expect.to_vec(), home.clone());
                blocking(move || files::delete(&path, &expect, &home)).await
            }
        }
    }

    pub async fn glob(
        &self,
        scope: &Scope<'_>,
        max: u64,
        time_limit: Duration,
    ) -> Result<SearchOutput, RemoteSearchError> {
        match self {
            Self::Sh { sh, .. } => remote_search::glob(sh, scope, max, time_limit).await,
            Self::Native { home, .. } => {
                let (scope, home) = (OwnedScope::of(scope), home.clone());
                searching(move |stop| search::glob(&scope.scope(), max, &home, stop)).await
            }
        }
    }

    pub async fn grep(
        &self,
        query: &GrepQuery<'_>,
        max: u64,
        time_limit: Duration,
    ) -> Result<SearchOutput, RemoteSearchError> {
        match self {
            Self::Sh { sh, .. } => remote_search::grep(sh, query, max, time_limit).await,
            Self::Native { home, .. } => {
                let scope = OwnedScope::of(&query.scope);
                let (pattern, home) = (query.pattern.to_owned(), home.clone());
                let (ignore_case, multiline, mode) =
                    (query.ignore_case, query.multiline, query.mode);
                searching(move |stop| {
                    let query = GrepQuery {
                        scope: scope.scope(),
                        pattern: &pattern,
                        extended: None,
                        ignore_case,
                        multiline,
                        mode,
                    };
                    search::grep(&query, max, &home, stop)
                })
                .await
            }
        }
    }

    /// AI-12: `command`, apart from the tab's shell, in `workdir` (absolute, or from the home
    /// directory) or the home directory. When `stop` completes first, the command and the
    /// processes it started are ended, and the result says whether they were.
    pub async fn run_command(
        &self,
        command: &str,
        workdir: Option<&str>,
        stop: impl std::future::Future<Output = ()>,
    ) -> Result<CommandRun, String> {
        match self {
            Self::Sh { sh, .. } => remote_command::run_local(sh, command, workdir, stop)
                .await
                .map_err(|e| e.message),
            Self::Native { shell, home } => {
                tokio::pin!(stop);
                let cwd = match workdir {
                    // A network folder can take long to answer; a stop does not wait for it.
                    Some(dir) => tokio::select! {
                        biased;
                        () = &mut stop => {
                            return Ok(CommandRun {
                                output: ExecOutput::default(),
                                stopped: Some(Stopped::NotStarted),
                            });
                        }
                        cwd = native_workdir(shell, dir, home) => cwd?,
                    },
                    None => home.clone(),
                };
                command::run(shell, command, &cwd, stop).await
            }
        }
    }
}

/// The directory a Windows shell starts a command in, read as the file tools read paths, or why
/// it cannot: for a missing one, the shell's own error would only say that it could not start.
async fn native_workdir(shell: &NativeShell, dir: &str, home: &Path) -> Result<PathBuf, String> {
    let path = files::resolve(dir, home);
    // Command Prompt starts in C:\Windows instead, after a warning. A network path starts with two
    // slashes of either kind.
    let network = path
        .to_string_lossy()
        .chars()
        .take(2)
        .filter(|c| matches!(c, '\\' | '/'))
        .count()
        == 2;
    if matches!(shell, NativeShell::Cmd(_)) && network {
        return Err(
            "Command Prompt cannot start in a network path: leave out workdir and use pushd in \
             the command"
                .to_owned(),
        );
    }
    let shown = files::shown(&path);
    match tokio::fs::metadata(&path).await {
        Ok(meta) if meta.is_dir() => Ok(path),
        Ok(_) => Err(format!("{shown} is not a directory")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Err(format!("the directory {shown} does not exist"))
        }
        Err(e) => Err(format!("the directory {shown} cannot be used: {e}")),
    }
}

/// An [`Expect`] that a blocking task can own.
enum Owned {
    Any,
    Missing,
    Content(Vec<u8>),
}

impl Owned {
    fn borrow(&self) -> Expect<'_> {
        match self {
            Self::Any => Expect::Any,
            Self::Missing => Expect::Missing,
            Self::Content(bytes) => Expect::Content(bytes),
        }
    }
}

/// A [`Scope`] that a blocking task can own.
struct OwnedScope {
    path: String,
    below: String,
    max_depth: Option<u32>,
    names: Vec<String>,
}

impl OwnedScope {
    fn of(scope: &Scope<'_>) -> Self {
        Self {
            path: scope.path.to_owned(),
            below: scope.below.to_owned(),
            max_depth: scope.max_depth,
            names: scope.names.to_vec(),
        }
    }

    fn scope(&self) -> Scope<'_> {
        Scope {
            path: &self.path,
            below: &self.below,
            max_depth: self.max_depth,
            names: &self.names,
        }
    }
}

/// A file operation on a blocking thread.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, RemoteFileError> + Send + 'static,
) -> Result<T, RemoteFileError> {
    tokio::task::spawn_blocking(f).await.unwrap_or_else(|e| {
        Err(RemoteFileError::Ssh(SshError::new(
            SshErrorKind::Other,
            e.to_string(),
        )))
    })
}

/// A search on a blocking thread, which stops when the caller drops the future (a stop, or its
/// time limit).
async fn searching(
    f: impl FnOnce(&AtomicBool) -> Result<SearchOutput, RemoteSearchError> + Send + 'static,
) -> Result<SearchOutput, RemoteSearchError> {
    struct StopOnDrop(Arc<AtomicBool>);
    impl Drop for StopOnDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Relaxed);
        }
    }
    let stop = Arc::new(AtomicBool::new(false));
    let guard = StopOnDrop(stop.clone());
    let result = tokio::task::spawn_blocking(move || f(&stop)).await;
    drop(guard);
    result.unwrap_or_else(|e| {
        Err(RemoteSearchError::Ssh(SshError::new(
            SshErrorKind::Other,
            e.to_string(),
        )))
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_command_starts_in_the_home_directory() {
        let shell = Shell {
            id: "/bin/sh".into(),
            name: "sh".into(),
            kind: ShellKind::Unix,
            program: "/bin/sh".into(),
            args: Vec::new(),
        };
        let tools = LocalTools::for_shell(&shell, Vec::new());
        let run = tools
            .run_command("pwd -P", None, std::future::pending())
            .await
            .unwrap();
        let home = dirs::home_dir().unwrap().canonicalize().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&run.output.stdout).trim_end(),
            home.to_str().unwrap()
        );
        let tmp = std::env::temp_dir().canonicalize().unwrap();
        let run = tools
            .run_command("pwd -P", tmp.to_str(), std::future::pending())
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&run.output.stdout).trim_end(),
            tmp.to_str().unwrap()
        );
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::*;
    use crate::shells;

    fn shell(kind: ShellKind) -> NativeShell {
        let found = shells::detect()
            .into_iter()
            .find(|s| s.kind == kind)
            .unwrap();
        match kind {
            ShellKind::PowerShell => NativeShell::PowerShell(found.program),
            ShellKind::Cmd => NativeShell::Cmd(found.program),
            _ => unreachable!(),
        }
    }

    #[tokio::test]
    async fn a_command_starts_in_workdir_from_the_home_directory() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("a b")).unwrap();
        std::fs::write(home.path().join("file"), "").unwrap();
        for kind in [ShellKind::PowerShell, ShellKind::Cmd] {
            let tools = LocalTools::Native {
                shell: shell(kind.clone()),
                home: home.path().to_path_buf(),
            };
            let line = if kind == ShellKind::Cmd {
                "cd"
            } else {
                "(Get-Location).Path"
            };
            for dir in ["a b", "~/a b"] {
                let run = tools
                    .run_command(line, Some(dir), std::future::pending())
                    .await
                    .unwrap();
                let printed = String::from_utf8_lossy(&run.output.stdout);
                // A temporary folder may be named in 8.3 form (RUNNER~1), which PowerShell prints
                // in full.
                assert_eq!(
                    std::fs::canonicalize(printed.trim_end()).unwrap(),
                    std::fs::canonicalize(home.path().join("a b")).unwrap(),
                    "{kind:?} {dir}"
                );
            }
            for (dir, error) in [
                ("missing", "does not exist"),
                ("file", "is not a directory"),
            ] {
                let refused = tools
                    .run_command(line, Some(dir), std::future::pending())
                    .await
                    .unwrap_err();
                assert!(refused.contains(error), "{refused}");
            }
            // A stop while the directory is looked up runs nothing.
            let run = tools
                .run_command(line, Some("a b"), std::future::ready(()))
                .await
                .unwrap();
            assert_eq!(run.stopped, Some(Stopped::NotStarted));
        }
        let cmd = LocalTools::Native {
            shell: shell(ShellKind::Cmd),
            home: home.path().to_path_buf(),
        };
        for dir in [r"\\server\share", r"/\server\share", r"\/server/share"] {
            let refused = cmd
                .run_command("cd", Some(dir), std::future::pending())
                .await
                .unwrap_err();
            assert!(refused.contains("pushd"), "{dir}: {refused}");
        }
    }
}
