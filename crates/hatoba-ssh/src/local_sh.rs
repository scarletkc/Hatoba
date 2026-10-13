//! For tests: the exec scripts of [`crate::remote_file`] and [`crate::remote_search`] against the
//! local `sh` and tools (GNU on Linux, BSD on macOS), started the way sshd and a login shell start
//! them: in `/`, so relative paths show that they are taken from `$HOME`, and in a process group of
//! their own, which a search's watchdog may end.

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::session::ExecOutput;

pub(crate) fn run(home: &Path, command: &str, stdin: &[u8]) -> ExecOutput {
    run_with(home, command, stdin, &[])
}

/// [`run`] with these variables set as well.
pub(crate) fn run_with(
    home: &Path,
    command: &str,
    stdin: &[u8],
    vars: &[(&str, &str)],
) -> ExecOutput {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .current_dir("/")
        .env("HOME", home)
        .envs(vars.iter().copied())
        .process_group(0)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(stdin).unwrap();
    let out = child.wait_with_output().unwrap();
    ExecOutput {
        exit_status: out.status.code().map(|c| c as u32),
        stdout: out.stdout,
        stderr: out.stderr,
    }
}

pub(crate) fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .is_ok_and(|o| o.stdout.starts_with(b"0\n"))
}
