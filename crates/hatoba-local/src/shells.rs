//! The shells a local terminal tab can run (LOCAL-02): on Windows PowerShell 7, Windows
//! PowerShell, Command Prompt, Git Bash and each WSL distribution; on macOS and Linux the login
//! shell (`$SHELL`) and the other shells `/etc/shells` lists. They are found when asked for, so a
//! shell installed meanwhile shows up the next time.

use std::path::{Path, PathBuf};

/// What kind of shell a tab runs. It decides how the assistant's tools run on the tab.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ShellKind {
    /// A shell of macOS or Linux, such as zsh, bash or fish.
    Unix,
    /// PowerShell 7 (`pwsh`) or Windows PowerShell.
    PowerShell,
    /// Command Prompt (`cmd.exe`).
    Cmd,
    /// Git Bash, the bash of Git for Windows.
    GitBash,
    /// A WSL distribution, by its name.
    Wsl { distro: String },
}

/// A shell this computer can run in a local tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shell {
    /// Stable on this device, so a preference can name it: `pwsh`, `powershell`, `cmd`,
    /// `git-bash` and `wsl:<distribution>` on Windows, the shell's path elsewhere.
    pub id: String,
    /// The name the shell picker shows, in English; the app translates the Windows shells' names.
    pub name: String,
    pub kind: ShellKind,
    /// The program the tab starts.
    pub program: PathBuf,
    /// Its arguments for an interactive terminal.
    pub args: Vec<String>,
}

impl Shell {
    /// Whether the shell runs on Windows itself rather than in a POSIX environment (Git Bash's,
    /// a WSL distribution's, or macOS's and Linux's own).
    #[must_use]
    pub fn is_windows_native(&self) -> bool {
        matches!(self.kind, ShellKind::PowerShell | ShellKind::Cmd)
    }
}

/// The shells this computer has, the default one first: PowerShell 7 where it is installed and
/// Windows PowerShell otherwise on Windows, the login shell elsewhere.
#[must_use]
pub fn detect() -> Vec<Shell> {
    #[cfg(windows)]
    {
        windows::detect()
    }
    #[cfg(not(windows))]
    {
        let etc = std::fs::read_to_string("/etc/shells").unwrap_or_default();
        unix_shells(
            std::env::var("SHELL").ok().as_deref(),
            &etc,
            cfg!(target_os = "macos"),
            |path| path.is_file(),
        )
    }
}

/// The shell with `id`, or the default when `id` is `None`.
#[must_use]
pub fn find(id: Option<&str>) -> Option<Shell> {
    let shells = detect();
    match id {
        Some(id) => shells.into_iter().find(|s| s.id == id),
        None => shells.into_iter().next(),
    }
}

/// The Unix shells: `$SHELL` when it names one that exists, else the platform's usual shell, then
/// those of `/etc/shells` (`etc`) that exist, each once. A login shell on macOS, as Terminal and
/// VS Code start it there, so `PATH` comes from the user's profile even for an app started from
/// the Dock; Linux terminals start an interactive shell that is not a login shell.
#[cfg_attr(windows, allow(dead_code))]
fn unix_shells(
    env_shell: Option<&str>,
    etc: &str,
    macos: bool,
    exists: impl Fn(&Path) -> bool,
) -> Vec<Shell> {
    let usual: &[&str] = if macos {
        &["/bin/zsh", "/bin/bash", "/bin/sh"]
    } else {
        &["/bin/bash", "/usr/bin/bash", "/bin/sh"]
    };
    let first = env_shell
        .filter(|s| s.starts_with('/') && exists(Path::new(s)))
        .or_else(|| usual.iter().copied().find(|s| exists(Path::new(s))));
    let listed = etc
        .lines()
        .map(|line| line.split('#').next().unwrap_or_default().trim())
        .filter(|line| line.starts_with('/'))
        .filter(|line| {
            let name = file_name(line);
            !matches!(name, "nologin" | "false" | "true" | "rbash" | "git-shell")
        })
        .filter(|line| exists(Path::new(line)));
    let mut paths: Vec<&str> = Vec::new();
    for path in first.into_iter().chain(listed) {
        // `/bin/bash` and `/usr/bin/bash` are one shell where `/bin` links to `/usr/bin`, and two
        // copies of one shell otherwise: either way the picker shows it once.
        if !paths
            .iter()
            .any(|p| *p == path || file_name(p) == file_name(path))
        {
            paths.push(path);
        }
    }
    let args = if macos {
        vec!["-l".to_owned()]
    } else {
        Vec::new()
    };
    paths
        .into_iter()
        .map(|path| Shell {
            id: path.to_owned(),
            name: file_name(path).to_owned(),
            kind: ShellKind::Unix,
            program: PathBuf::from(path),
            args: args.clone(),
        })
        .collect()
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(windows)]
mod windows {
    use std::path::{Path, PathBuf};

    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    use super::{Shell, ShellKind};

    pub(super) fn detect() -> Vec<Shell> {
        let root = system_root();
        let system32 = root.join("System32");
        let mut shells = Vec::new();
        if let Some(pwsh) = pwsh() {
            shells.push(shell(
                "pwsh",
                "PowerShell",
                ShellKind::PowerShell,
                pwsh,
                &["-NoLogo"],
            ));
        }
        let powershell = system32.join(r"WindowsPowerShell\v1.0\powershell.exe");
        if powershell.is_file() {
            shells.push(shell(
                "powershell",
                "Windows PowerShell",
                ShellKind::PowerShell,
                powershell,
                &["-NoLogo"],
            ));
        }
        let cmd = std::env::var_os("ComSpec")
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .unwrap_or_else(|| system32.join("cmd.exe"));
        shells.push(shell("cmd", "Command Prompt", ShellKind::Cmd, cmd, &[]));
        if let Some(bash) = git_bash() {
            shells.push(shell(
                "git-bash",
                "Git Bash",
                ShellKind::GitBash,
                bash,
                &["--login", "-i"],
            ));
        }
        let wsl = system32.join("wsl.exe");
        if wsl.is_file() {
            for distro in wsl_distros() {
                shells.push(Shell {
                    id: format!("wsl:{distro}"),
                    name: distro.clone(),
                    args: vec!["-d".into(), distro.clone(), "--cd".into(), "~".into()],
                    kind: ShellKind::Wsl { distro },
                    program: wsl.clone(),
                });
            }
        }
        shells
    }

    fn shell(id: &str, name: &str, kind: ShellKind, program: PathBuf, args: &[&str]) -> Shell {
        Shell {
            id: id.to_owned(),
            name: name.to_owned(),
            kind,
            program,
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        }
    }

    fn system_root() -> PathBuf {
        std::env::var_os("SystemRoot")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
    }

    /// PowerShell 7 from its installer's folder, else from `PATH` (the Microsoft Store's alias).
    fn pwsh() -> Option<PathBuf> {
        let installed = ["ProgramFiles", "ProgramW6432"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|dir| Path::new(&dir).join(r"PowerShell\7\pwsh.exe"))
            .find(|p| p.is_file());
        installed.or_else(|| which::which("pwsh.exe").ok())
    }

    /// `bin\bash.exe` of Git for Windows, from the folder its installer records, or the usual
    /// folders.
    fn git_bash() -> Option<PathBuf> {
        let recorded = [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER]
            .into_iter()
            .filter_map(|hive| {
                RegKey::predef(hive)
                    .open_subkey(r"SOFTWARE\GitForWindows")
                    .ok()?
                    .get_value::<String, _>("InstallPath")
                    .ok()
            })
            .map(PathBuf::from);
        let usual = ["ProgramFiles", "ProgramW6432"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|dir| Path::new(&dir).join("Git"))
            .chain(
                std::env::var_os("LOCALAPPDATA").map(|dir| Path::new(&dir).join(r"Programs\Git")),
            );
        recorded
            .chain(usual)
            .map(|dir| dir.join(r"bin\bash.exe"))
            .find(|p| p.is_file())
    }

    /// The installed WSL distributions, the default one first, from the registry key `wsl.exe`
    /// keeps them in. Docker Desktop's and Rancher Desktop's own distributions are left out, as
    /// Windows Terminal leaves them out.
    fn wsl_distros() -> Vec<String> {
        let Ok(lxss) = RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Lxss")
        else {
            return Vec::new();
        };
        let default: Option<String> = lxss.get_value("DefaultDistribution").ok();
        let mut distros: Vec<(bool, String)> = lxss
            .enum_keys()
            .filter_map(Result::ok)
            .filter_map(|guid| {
                let key = lxss.open_subkey(&guid).ok()?;
                // 1 is installed; others are installing or being removed.
                let state: u32 = key.get_value("State").unwrap_or(1);
                let name: String = key.get_value("DistributionName").ok()?;
                (state == 1 && !hidden(&name))
                    .then(|| (default.as_deref() == Some(guid.as_str()), name))
            })
            .collect();
        distros.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        distros.into_iter().map(|(_, name)| name).collect()
    }

    fn hidden(name: &str) -> bool {
        let lower = name.to_ascii_lowercase();
        lower.starts_with("docker-desktop") || lower.starts_with("rancher-desktop")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exists(paths: &'static [&'static str]) -> impl Fn(&Path) -> bool {
        move |p| paths.iter().any(|q| Path::new(q) == p)
    }

    #[test]
    fn login_shell_comes_first_then_etc_shells_once_each() {
        let etc = "# /etc/shells\n/bin/sh\n/bin/bash\n/usr/bin/bash\n/bin/zsh\n/usr/bin/fish\n\
                   /usr/sbin/nologin\n/bin/rbash\n/missing/ksh\n";
        let shells = unix_shells(
            Some("/bin/zsh"),
            etc,
            false,
            exists(&[
                "/bin/sh",
                "/bin/bash",
                "/usr/bin/bash",
                "/bin/zsh",
                "/usr/bin/fish",
                "/usr/sbin/nologin",
                "/bin/rbash",
            ]),
        );
        let ids: Vec<&str> = shells.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, ["/bin/zsh", "/bin/sh", "/bin/bash", "/usr/bin/fish"]);
        assert_eq!(shells[3].name, "fish");
        assert!(
            shells
                .iter()
                .all(|s| s.args.is_empty() && s.kind == ShellKind::Unix)
        );
    }

    #[test]
    fn macos_starts_a_login_shell_and_falls_back_to_zsh() {
        let shells = unix_shells(None, "", true, exists(&["/bin/zsh", "/bin/bash"]));
        assert_eq!(shells.len(), 1);
        assert_eq!(shells[0].id, "/bin/zsh");
        assert_eq!(shells[0].args, ["-l"]);
    }

    #[test]
    fn a_shell_variable_that_names_nothing_is_skipped() {
        let shells = unix_shells(Some("zsh"), "/bin/bash\n", false, exists(&["/bin/bash"]));
        assert_eq!(shells[0].id, "/bin/bash");
        let shells = unix_shells(Some("/opt/none"), "", false, exists(&["/bin/sh"]));
        assert_eq!(shells[0].id, "/bin/sh");
    }

    #[cfg(windows)]
    #[test]
    fn windows_has_command_prompt_and_a_powershell_first() {
        let shells = detect();
        assert!(
            shells
                .iter()
                .any(|s| s.id == "cmd" && s.kind == ShellKind::Cmd)
        );
        assert_eq!(shells[0].kind, ShellKind::PowerShell);
        assert!(shells.iter().all(|s| s.program.is_file()));
    }
}
