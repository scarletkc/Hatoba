//! Running a command on the host so that stopping it ends it (spec AI-12, AI-18): the AI
//! assistant's `run_command`, and `hatoba exec` (HOST-15), which [`run_streaming`] passes the
//! output to as it comes.
//!
//! Closing an exec channel does not end its command: without a PTY, sshd only closes the
//! command's pipes, so a quiet command runs on, and a chatty one dies of SIGPIPE at whatever
//! line it writes next. A `signal` request reaches only the process sshd started, which is the
//! login shell, not the `make` of `cd dir && make`, and Dropbear and others ignore it.
//!
//! So the command runs under [`SCRIPT`], a fixed `sh -c` script, which starts it in a process
//! group of its own and keeps the channel's stdin open as the line that stops it. When stdin
//! reaches its end, because Hatoba sent EOF, closed the channel or lost the connection, the script
//! sends the group TERM, then KILL after a grace period, and prints whether any process of the
//! group is left. Only the stdin of the channel is used, so stopping works on every server,
//! whatever it does with signal requests. The command itself travels on stdin too, after the
//! script says it is ready, so it never goes through the quoting of the login shell, and a host
//! whose login shell cannot run the script (a router's command line, say) is sent nothing it
//! might take for a command: the command then runs as it is ([`plain`]), as on Windows, and
//! stopping it only closes its channel.

use std::collections::VecDeque;
use std::ffi::OsStr;
use std::future::Future;
use std::hash::{BuildHasher, RandomState};
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bytes::Bytes;
use russh::{ChannelMsg, ChannelReadHalf, Sig};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::Instant;

use crate::error::SshError;
use crate::local_sh::LocalSh;
use crate::remote_file::last_line;
use crate::server_os::ServerOs;
use crate::session::{ExecOutput, ExecSlot, ExecWriter, MAX_EXEC_OUTPUT, SshSession};
use crate::sudo::{self, RunAs};

/// Starts the command it reads from stdin in a process group of its own, and ends the group when
/// stdin ends. `$1` is a number: the script's marker is `hatoba-run-` and the number after it, a
/// line the command cannot print by chance, and that an echo of this command line does not hold.
///
/// It runs the command with the login shell (`$SHELL -c`), as sshd would, and in a new process
/// group: with `setsid` (util-linux and BusyBox), else with `perl`'s `setpgrp` (macOS and the
/// BSDs), else in its own group, which only TERM to the command's own process reaches. A
/// command started in the background of a script gets SIGINT and SIGQUIT ignored, which `env
/// --default-signal` (GNU) and `perl` undo. The script prints the marker and the way it found,
/// then reads the number of the command's lines, a line with the directory to start it in, the
/// command, and a last line of the marker and `-end`, so a command cut short is never run (exit
/// status 9). An empty directory leaves the command in the directory the script started in,
/// usually the home directory; `~`, `~/…` and any other relative path, also one that starts with
/// `-`, are from `$HOME`, not from where the login shell's startup files may have changed to, nor
/// from `CDPATH`. When `cd` fails, the script
/// ends with its status and message, and runs nothing. The command's stdin is `/dev/null`, as
/// the empty stdin sshd gives a command that is sent none, unless a stdin to pass is asked for, as
/// below. Its exit status is the script's; one ended by a signal is 128 and the signal's number.
///
/// As root through sudo ([`RunAs::Sudo`]), whose environment has root's `HOME` and `SHELL`, `$2`
/// and `$3` are the login user's, which the directory and the shell come from instead.
///
/// A fourth argument that is not empty asks for a stdin to pass: before the marker, the script
/// makes a named pipe in a folder of `mktemp -d`, which only its user can open, and prints the
/// marker and `-in` with its path, which [`INPUT_SCRIPT`] writes into as the same user; without
/// that line, it could not make one. The script holds both ends of the pipe from the start, so
/// the writer opens it without waiting, and the command takes the script's read end as its stdin.
/// The request comes once the writer has opened the pipe, and the script then lets go of its write
/// end, so the command's stdin ends where the writer's does, never before the writer came. The
/// folder goes when the script ends.
///
/// When stdin ends, a second process sends the group TERM, waits up to 2 seconds for its
/// processes to end, sends KILL to those left, waits up to 1 second more, and prints the marker
/// and `ended` (no process of the group is left), `running` (one is, or is another user's and
/// cannot be signalled), or `unknown` (no group: the command's own process ended, but not
/// necessarily those it started). `kill -0` counts a process that ended but that its parent has
/// not reaped yet, which an orphan stays until init reaps it, and some containers' init takes
/// seconds or never does: on Linux, such a group is looked up in `/proc` before it counts as
/// running. The signals go to the command's own process too, which a stop that comes before
/// `setsid` or `perl` has made the group reaches all the same. A command that ends on its own
/// stops the second process, and leaves the processes it put in the background running.
///
/// One line without single quotes or backslashes, like the file tools' scripts, so every common
/// login shell reads it the same way.
const SCRIPT: &str = "m=hatoba-run-$(($1+1)); s=${3:-${SHELL:-sh}}; o=${2:-$HOME}; \
     if command -v setsid >/dev/null 2>&1; then g=setsid; \
     elif command -v perl >/dev/null 2>&1; then g=perl; else g=none; fi; \
     exec 6</dev/null; if [ -n \"$4\" ] && e=$(mktemp -d 2>/dev/null); then \
     case $e in /*) ;; *) e=$PWD/$e;; esac; z() { rm -rf -- \"$e\"; }; trap z EXIT; \
     if mkfifo -m 600 \"$e/in\" 2>/dev/null; then \
     exec 7<>\"$e/in\" 6<\"$e/in\"; echo \"$m-in $e/in\"; fi; fi; \
     echo \"$m $g\" || exit 1; IFS= read -r n || exit 9; [ \"$n\" -gt 0 ] 2>/dev/null || exit 9; \
     IFS= read -r h || exit 9; \
     c=$(head -n $((n+1)); echo x); c=${c%x}; d=${c%\"$m\"-end?}; [ \"$d\" = \"$c\" ] && exit 9; \
     d=${d%?}; \
     case $h in \"\") ;; \"~\") cd \"$o\" || exit;; \"~/\"*) cd \"$o/${h#??}\" || exit;; \
     /*) cd \"$h\" || exit;; *) cd \"$o/$h\" || exit;; esac; \
     exec 3<&0 7>&-; case $g in \
     setsid) if env --default-signal=INT,QUIT true >/dev/null 2>&1; then \
     setsid env --default-signal=INT,QUIT \"$s\" -c \"$d\" <&6 3<&- 6<&- & \
     else setsid \"$s\" -c \"$d\" <&6 3<&- 6<&- & fi;; \
     perl) perl -e \"@SIG{qw(INT QUIT)}=qw(DEFAULT DEFAULT); setpgrp(0,0); exec @ARGV or exit 127\" \
     \"$s\" -c \"$d\" <&6 3<&- 6<&- & ;; \
     *) \"$s\" -c \"$d\" <&6 3<&- 6<&- & ;; esac; \
     p=$!; exec 6<&-; if [ $g = none ]; then t=$p; else t=-$p; fi; \
     a() { kill -0 $p 2>/dev/null && return 0; \
     case $(LC_ALL=C kill -0 $t 2>&1) in *[Nn]\"o such process\"*) return 1;; esac; }; \
     b() { [ -r /proc/self/stat ] || return 0; for f in /proc/[0-9]*/stat; do \
     read -r l 2>/dev/null <\"$f\" || continue; l=${l##*\") \"}; set -- $l; \
     [ \"$3\" = \"$p\" ] && case $1 in Z) ;; *) return 0;; esac; done; return 1; }; \
     ( IFS= read -r x; trap \"\" TERM; kill -TERM $t $p 2>/dev/null; \
     if sleep 0.1 2>/dev/null; then i=0.1 j=20 k=10; else i=1 j=2 k=1; fi; \
     while a && [ $j -gt 0 ]; do sleep $i; j=$((j-1)); done; \
     if a; then kill -KILL $t $p 2>/dev/null; \
     while a && [ $k -gt 0 ]; do sleep $i; k=$((k-1)); done; fi; \
     v=ended; if a; then v=running; [ $g = none ] || [ -n \"$(LC_ALL=C kill -0 $t 2>&1)\" ] || \
     b || v=ended; elif [ $g = none ]; then v=unknown; fi; echo \"$m $v\" ) <&3 3<&- 6<&- & \
     w=$!; exec 3<&-; wait $p 2>/dev/null; r=$?; kill $w 2>/dev/null; wait $w 2>/dev/null; \
     exit $r";

/// Writes what comes on its stdin after the first line into the named pipe on that line, the one
/// [`SCRIPT`] made for the command's stdin, and prints [`INPUT_OPEN`] once it has the pipe open.
/// `read` takes the line a byte at a time, so the rest is left for `cat`, as sudo does with the
/// password line before it.
const INPUT_SCRIPT: &str = "IFS= read -r f || exit 9; exec 5>\"$f\" || exit 1; \
     echo hatoba-input-open; exec cat >&5 5>&-";

/// The line [`INPUT_SCRIPT`] prints once the pipe is open.
const INPUT_OPEN: &str = "hatoba-input-open";

/// How long a stop waits for the script's report after it sent EOF: the script's grace period
/// and the KILL after it, with time for the round trip. The channel is closed then.
pub const STOP_WAIT: Duration = Duration::from_secs(5);

/// A command that [`run`] ran.
#[derive(Debug, Clone, Default)]
pub struct CommandRun {
    /// What the command printed, and its exit status. When it was stopped, what it printed until
    /// then, and the status it ended with, if it did.
    pub output: ExecOutput,
    /// `None` when the command ended on its own; otherwise what the stop did.
    pub stopped: Option<Stopped>,
}

/// What became of a command that [`run`] stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stopped {
    /// The stop came before the command reached the host, so it never ran.
    NotStarted,
    /// The command and every process in its process group ended.
    Ended,
    /// The command may still be running: the host had no way to give it a process group, a
    /// process of the group outlived KILL, or the host did not report in time.
    MayRun,
}

/// A piece of what a command that [`run_streaming`] runs printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    Stdout(Bytes),
    Stderr(Bytes),
}

/// Runs `command` with the login shell on a new exec channel of `session` and returns what it
/// printed, like [`SshSession::exec_output`] with no stdin. It starts in `workdir`, absolute or
/// from the home directory, or else in the home directory. When `stop` completes first, the
/// command and the processes it started are ended, and the result says whether they did
/// ([`CommandRun::stopped`]); that takes at most [`STOP_WAIT`] more.
pub async fn run(
    session: &SshSession,
    command: &str,
    workdir: Option<&str>,
    stop: impl Future<Output = ()>,
) -> Result<CommandRun, SshError> {
    run_with(session, command, workdir, &RunAs::Login, None, stop, None).await
}

/// [`run`], which sends what the command prints to `output` as it comes, and keeps none of it:
/// the result's [`CommandRun::output`] has only the exit status. Sending waits while `output` is
/// full. Once its receiver is gone, what the command prints is dropped.
pub async fn run_streaming(
    session: &SshSession,
    command: &str,
    workdir: Option<&str>,
    stop: impl Future<Output = ()>,
    output: mpsc::Sender<Output>,
) -> Result<CommandRun, SshError> {
    run_with(
        session,
        command,
        workdir,
        &RunAs::Login,
        None,
        stop,
        Some(&output),
    )
    .await
}

/// [`run_streaming`] as `run_as`: as root, the script itself runs through sudo, so a stop ends the
/// command and every process it started as it does for the login user. A host that cannot run the
/// script runs nothing as root.
///
/// With `stdin`, what comes on it is the command's stdin, which ends when its senders are gone:
/// under the script, it goes through a named pipe on the host that a second exec channel writes
/// as `run_as` ([`INPUT_SCRIPT`]); on a host that cannot run the script, straight to the
/// command's channel.
pub async fn run_streaming_as(
    session: &SshSession,
    command: &str,
    workdir: Option<&str>,
    run_as: &RunAs,
    stdin: Option<mpsc::Receiver<Vec<u8>>>,
    stop: impl Future<Output = ()>,
    output: mpsc::Sender<Output>,
) -> Result<CommandRun, SshError> {
    run_with(
        session,
        command,
        workdir,
        run_as,
        stdin,
        stop,
        Some(&output),
    )
    .await
}

async fn run_with(
    session: &SshSession,
    command: &str,
    workdir: Option<&str>,
    run_as: &RunAs,
    mut stdin: Option<mpsc::Receiver<Vec<u8>>>,
    stop: impl Future<Output = ()>,
    sink: Option<&mpsc::Sender<Output>>,
) -> Result<CommandRun, SshError> {
    check(command, workdir)?;
    tokio::pin!(stop);
    let shared = session.shared();
    // Win32-OpenSSH runs commands with cmd.exe or PowerShell, which have no `sh`.
    if !shared.plain_commands() && session.server_os() != Some(ServerOs::Windows) {
        match wrapped(
            session,
            command,
            workdir,
            run_as,
            &mut stdin,
            stop.as_mut(),
            sink,
        )
        .await?
        {
            Some(run) => return Ok(run),
            None => {
                tracing::info!("the host cannot run the command script; commands run as they are");
                shared.set_plain_commands();
            }
        }
    }
    if run_as.is_sudo() {
        return Err(SshError::other(
            "this host has no sh to run the command with sudo",
        ));
    }
    if workdir.is_some() {
        return Err(SshError::other(
            "this host has no sh to start the command in workdir: leave out workdir and change \
             the directory in the command",
        ));
    }
    plain(session, command, stdin, stop, sink).await
}

/// What [`SCRIPT`] cannot take: a NUL character, or a directory on more than one line.
fn check(command: &str, workdir: Option<&str>) -> Result<(), SshError> {
    if command.contains('\0') {
        return Err(SshError::other("the command has a NUL character"));
    }
    if workdir.is_some_and(|d| d.contains(['\n', '\0'])) {
        return Err(SshError::other(
            "workdir has a line break or a NUL character",
        ));
    }
    Ok(())
}

/// The command line that starts [`SCRIPT`] with the number `n`, through sudo with the login user's
/// home directory and shell, which the login shell fills in, and asking for a stdin to pass when
/// `input` is set.
fn command_line(run_as: &RunAs, n: u32, input: bool) -> String {
    let input = if input { " 1" } else { "" };
    match run_as {
        RunAs::Login if input.is_empty() => format!("sh -c '{SCRIPT}' sh {n}"),
        RunAs::Login => format!("sh -c '{SCRIPT}' sh {n} \"\" \"\"{input}"),
        RunAs::Sudo(_) => format!(
            "{}sh -c '{SCRIPT}' sh {n} \"$HOME\" \"$SHELL\"{input}",
            run_as.prefix()
        ),
    }
}

/// What [`SCRIPT`] reads once it is ready: the number of the command's lines, the directory, the
/// command, and the line that ends it.
fn request(command: &str, workdir: Option<&str>, marker: &str) -> String {
    let lines = command.matches('\n').count() + 1;
    let workdir = workdir.unwrap_or("");
    format!("{lines}\n{workdir}\n{command}\n{marker}-end\n")
}

/// Runs `command` under [`SCRIPT`]. `None` when the channel ended before the script was ready, so
/// the host cannot run it and nothing was sent, to the host or to `sink`.
///
/// With a `sink`, the output goes there while the command runs, but not around the script's lines:
/// what comes before the script is ready is held back until it is, and after a stop, until the
/// script's report is taken out.
///
/// Through sudo, the password goes first, and a channel that ends before the script is ready is
/// sudo's refusal, which the error quotes.
///
/// `stdin` is taken once the script is ready, and left for [`plain`] when it never is.
async fn wrapped(
    session: &SshSession,
    command: &str,
    workdir: Option<&str>,
    run_as: &RunAs,
    stdin: &mut Option<mpsc::Receiver<Vec<u8>>>,
    mut stop: Pin<&mut impl Future<Output = ()>>,
    sink: Option<&mpsc::Sender<Output>>,
) -> Result<Option<CommandRun>, SshError> {
    let password = run_as.password_line()?;
    let gives_password = password.is_some();
    let n = nonce();
    let marker = marker(n);
    // Held until the stdin's channel is open beside the command's.
    let mut turn = None;
    if stdin.is_some() {
        tokio::select! {
            biased;
            () = &mut stop => return Ok(Some(not_started())),
            held = session.exec_pair_turn() => turn = Some(held),
        }
    }
    let Some((mut reader, writer, _slot)) = open(session, stop.as_mut(), 0).await? else {
        return Ok(Some(not_started()));
    };
    writer
        .half()
        .exec(true, command_line(run_as, n, stdin.is_some()))
        .await
        .map_err(SshError::from)?;
    if let Some(password) = password {
        writer
            .half()
            .data_bytes(Bytes::from(password))
            .await
            .map_err(SshError::from)?;
    }
    let mut out = Collected::default();
    let mut pipe = None;

    // Until the script says it is ready, nothing goes to the host but the command line above, and
    // sudo's password.
    loop {
        tokio::select! {
            biased;
            () = &mut stop => return Ok(Some(not_started())),
            msg = reader.wait() => match out.take(session, msg)? {
                Step::More => {
                    if take_line(&mut out.output.stdout, &marker, Find::First).is_some() {
                        if gives_password {
                            drop_sudo_line_break(&mut out.output.stderr);
                        }
                        if stdin.is_some() {
                            pipe = take_pipe_line(&mut out.output.stdout, &marker);
                        }
                        if sink.is_some() {
                            out.release();
                            out.forward = true;
                        }
                        break;
                    }
                }
                Step::Ended => {
                    writer.ended();
                    if session.is_closed() {
                        return Err(session.close_error());
                    }
                    if run_as.is_sudo() {
                        return Err(sudo::refused(&out.output.stderr));
                    }
                    return Ok(None);
                }
            },
        }
    }

    out.flush(sink).await;
    // A second channel writes the command's stdin into the pipe the script made, as the same user,
    // so only the command's own stdin reaches a command that runs as root. Dropping the first one
    // ends the script, which runs nothing and removes the pipe.
    let mut input = match stdin.take() {
        None => None,
        Some(stdin) => {
            let Some(pipe) = pipe else {
                return Err(SshError::other(
                    "can't pass stdin to the command: the host could not make a pipe for it with \
                     mktemp and mkfifo",
                ));
            };
            let opened = open(session, stop.as_mut(), 1).await.map_err(|e| {
                if session.is_closed() {
                    e
                } else {
                    SshError::other(format!(
                        "can't pass stdin to the command: the host refused a second session for \
                         it: {}",
                        e.message
                    ))
                }
            })?;
            let Some((mut in_reader, in_writer, in_slot)) = opened else {
                return Ok(Some(not_started()));
            };
            drop(turn.take());
            in_writer
                .half()
                .exec(true, sudo::root_command(run_as, INPUT_SCRIPT))
                .await
                .map_err(SshError::from)?;
            let mut first = run_as.password_line()?.unwrap_or_default();
            first.extend_from_slice(format!("{pipe}\n").as_bytes());
            in_writer
                .half()
                .data_bytes(Bytes::from(first))
                .await
                .map_err(SshError::from)?;
            // The request goes once the pipe has its writer, which the script waits for.
            tokio::select! {
                biased;
                () = &mut stop => return Ok(Some(not_started())),
                opened = input_opened(session, &mut in_reader) => opened?,
            }
            Some((in_reader, in_writer, in_slot, stdin))
        }
    };
    let passing = async {
        if let Some((reader, writer, _, stdin)) = input.as_mut() {
            pass_input(writer, reader, stdin).await;
        }
    };
    tokio::pin!(passing);
    let mut passed = false;
    let feed = writer
        .half()
        .data_bytes(Bytes::from(request(command, workdir, &marker)));
    tokio::pin!(feed);
    let mut fed = false;
    let mut deadline: Option<Instant> = None;
    let ended = loop {
        tokio::select! {
            biased;
            // Stopped once the whole command is on its way: the script never runs a command cut
            // short, but would not say so.
            () = &mut stop, if fed && deadline.is_none() => {
                deadline = Some(Instant::now() + STOP_WAIT);
                // The script's report comes next.
                out.forward = false;
                if writer.half().eof().await.is_err() {
                    break false;
                }
            }
            () = sleep_until(deadline), if deadline.is_some() => break false,
            result = &mut feed, if !fed => {
                fed = true;
                if let Err(e) = result {
                    tracing::debug!("run: sending the command ended early: {e}");
                }
            }
            () = &mut passing, if fed && !passed && deadline.is_none() => passed = true,
            () = out.send_next(sink), if !out.pending.is_empty() => {}
            // Output waits for the receiver, without holding up the stop; after it, the report
            // comes into `output`.
            msg = reader.wait(), if out.pending.is_empty() || deadline.is_some() => {
                match out.take(session, msg)? {
                    Step::More => {}
                    Step::Ended => break true,
                }
            }
        }
    };
    if ended {
        writer.ended();
    }
    let report = take_line(&mut out.output.stdout, &marker, Find::Last);
    if sink.is_some() {
        out.release();
        if deadline.is_some() {
            out.offer(sink);
        } else {
            out.flush(sink).await;
        }
    }
    let stopped = match (deadline, report.as_deref()) {
        (None, _) => None,
        (Some(_), Some("ended")) => Some(Stopped::Ended),
        (Some(_), Some(_)) => Some(Stopped::MayRun),
        // The command ended on its own as the stop came, which stopped the script's report.
        (Some(_), None) if ended && out.output.exit_status.is_some() => None,
        (Some(_), None) => Some(Stopped::MayRun),
    };
    Ok(Some(CommandRun {
        output: out.output,
        stopped,
    }))
}

/// Runs `command` as it is, for a host that cannot run [`SCRIPT`], with what comes on `stdin` as
/// its stdin. A stop sends the command's process TERM, which servers may ignore, and closes the
/// channel, so the command may still run.
async fn plain(
    session: &SshSession,
    command: &str,
    mut stdin: Option<mpsc::Receiver<Vec<u8>>>,
    mut stop: Pin<&mut impl Future<Output = ()>>,
    sink: Option<&mpsc::Sender<Output>>,
) -> Result<CommandRun, SshError> {
    let Some((mut reader, writer, _slot)) = open(session, stop.as_mut(), 0).await? else {
        return Ok(not_started());
    };
    writer
        .half()
        .exec(true, command.as_bytes().to_vec())
        .await
        .map_err(SshError::from)?;
    let passing = async {
        if let Some(stdin) = stdin.as_mut() {
            send_input(&writer, stdin).await;
        }
        let _ = writer.half().eof().await;
    };
    tokio::pin!(passing);
    let mut passed = false;
    let mut out = Collected {
        forward: sink.is_some(),
        ..Collected::default()
    };
    loop {
        tokio::select! {
            biased;
            () = &mut stop => {
                let _ = writer.half().signal(Sig::TERM).await;
                out.offer(sink);
                return Ok(CommandRun {
                    output: out.output,
                    stopped: Some(Stopped::MayRun),
                });
            }
            () = &mut passing, if !passed => passed = true,
            () = out.send_next(sink), if !out.pending.is_empty() => {}
            msg = reader.wait(), if out.pending.is_empty() => {
                if let Step::Ended = out.take(session, msg)? {
                    writer.ended();
                    out.flush(sink).await;
                    return Ok(CommandRun {
                        output: out.output,
                        stopped: None,
                    });
                }
            }
        }
    }
}

/// Waits for [`INPUT_OPEN`] on the channel of [`INPUT_SCRIPT`]; an error with what the host said
/// when it ends first, as sudo does when it refuses.
async fn input_opened(session: &SshSession, reader: &mut ChannelReadHalf) -> Result<(), SshError> {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    loop {
        match reader.wait().await {
            Some(ChannelMsg::Data { data }) => {
                stdout.extend_from_slice(&data);
                if String::from_utf8_lossy(&stdout)
                    .lines()
                    .any(|line| line == INPUT_OPEN)
                {
                    return Ok(());
                }
                if stdout.len() > MAX_EXEC_OUTPUT {
                    return Err(SshError::other(
                        "the host printed too much before the stdin",
                    ));
                }
            }
            Some(ChannelMsg::ExtendedData { data, .. }) => {
                if stderr.len() < MAX_EXEC_OUTPUT {
                    stderr.extend_from_slice(&data);
                }
            }
            Some(ChannelMsg::Failure) => {
                return Err(SshError::channel(
                    "can't pass stdin to the command: the server refused to run its writer",
                ));
            }
            Some(ChannelMsg::Close) | None => {
                if session.is_closed() {
                    return Err(session.close_error());
                }
                let said = last_line(&stderr);
                return Err(SshError::other(if said.is_empty() {
                    "can't pass stdin to the command: the host ended its channel".to_owned()
                } else {
                    format!("can't pass stdin to the command: {said}")
                }));
            }
            Some(_) => {}
        }
    }
}

/// Sends what comes on `stdin` over the channel of `writer`, until its senders are gone or the
/// channel is.
async fn send_input(writer: &ExecWriter, stdin: &mut mpsc::Receiver<Vec<u8>>) {
    while let Some(data) = stdin.recv().await {
        if writer.half().data_bytes(Bytes::from(data)).await.is_err() {
            return;
        }
    }
}

/// Passes what comes on `stdin` to [`INPUT_SCRIPT`] on its channel, then EOF, which ends the
/// command's stdin. The channel's messages are read meanwhile: the window adjustments that let
/// more go would otherwise pile up unread.
async fn pass_input(
    writer: &ExecWriter,
    reader: &mut ChannelReadHalf,
    stdin: &mut mpsc::Receiver<Vec<u8>>,
) {
    let send = async {
        send_input(writer, stdin).await;
        let _ = writer.half().eof().await;
    };
    let read = async { while reader.wait().await.is_some() {} };
    tokio::join!(send, read);
}

/// Runs `command` like [`run`], with the `sh` of this computer (a local tab, spec §7.5): under
/// [`SCRIPT`], with `sh`'s stdin as the line that stops it, so a stop ends the command and every
/// process it started as it does on a host. The script runs the command with `$SHELL`, which `sh`
/// sets to the tab's shell, in `workdir` or the directory `sh` starts in.
pub async fn run_local(
    sh: &LocalSh,
    command: &str,
    workdir: Option<&str>,
    stop: impl Future<Output = ()>,
) -> Result<CommandRun, SshError> {
    check(command, workdir)?;
    tokio::pin!(stop);
    let n = nonce();
    let marker = marker(n);
    let number = n.to_string();
    let mut child = sh.spawn(&[
        OsStr::new("-c"),
        OsStr::new(SCRIPT),
        OsStr::new("sh"),
        OsStr::new(&number),
    ])?;
    let (Some(input), Some(mut stdout), Some(mut stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(SshError::other("sh has no pipes"));
    };
    let mut pipes = LocalPipes::default();

    // Until the script says it is ready, nothing goes to `sh` but its arguments.
    loop {
        tokio::select! {
            biased;
            () = &mut stop => return Ok(not_started()),
            read = stdout.read(&mut pipes.out_buf), if pipes.out_open => {
                pipes.take_out(read)?;
                if take_line(&mut pipes.output.stdout, &marker, Find::First).is_some() {
                    break;
                }
            }
            read = stderr.read(&mut pipes.err_buf), if pipes.err_open => pipes.take_err(read)?,
            status = child.wait() => {
                let status = status.map(|s| s.code()).unwrap_or_default();
                return Err(SshError::other(format!(
                    "sh could not run the command (status {}): {}",
                    status.map_or_else(|| "none".to_owned(), |s| s.to_string()),
                    String::from_utf8_lossy(&pipes.output.stderr).trim()
                )));
            }
        }
    }

    let request = request(command, workdir, &marker);
    let feed = async move {
        let mut input = input;
        input.write_all(request.as_bytes()).await.map(|()| input)
    };
    tokio::pin!(feed);
    // Kept open while the command runs: its end is the stop.
    let mut held = None;
    let mut fed = false;
    let mut deadline: Option<Instant> = None;
    let mut exited: Option<Option<u32>> = None;
    let mut drain: Option<Instant> = None;
    loop {
        tokio::select! {
            biased;
            // Stopped once the whole command is on its way, as on a host.
            () = &mut stop, if fed && deadline.is_none() => {
                held = None;
                deadline = Some(Instant::now() + STOP_WAIT);
            }
            () = sleep_until(deadline), if deadline.is_some() && exited.is_none() => break,
            result = &mut feed, if !fed => {
                fed = true;
                match result {
                    Ok(input) => held = Some(input),
                    Err(e) => tracing::debug!("run_local: sending the command ended early: {e}"),
                }
            }
            read = stdout.read(&mut pipes.out_buf), if pipes.out_open => pipes.take_out(read)?,
            read = stderr.read(&mut pipes.err_buf), if pipes.err_open => pipes.take_err(read)?,
            status = child.wait(), if exited.is_none() => {
                exited = Some(status.ok().and_then(|s| s.code()).and_then(|c| u32::try_from(c).ok()));
                // What it printed last is still read, unless a process it left holds the pipes.
                drain = Some(Instant::now() + Duration::from_millis(200));
            }
            () = sleep_until(drain), if drain.is_some() => break,
        }
        if exited.is_some() && !pipes.out_open && !pipes.err_open {
            break;
        }
    }
    drop(held);
    let mut output = pipes.output;
    output.exit_status = exited.flatten();
    let report = take_line(&mut output.stdout, &marker, Find::Last);
    let stopped = match (deadline, report.as_deref()) {
        (None, _) => None,
        (Some(_), Some("ended")) => Some(Stopped::Ended),
        (Some(_), Some(_)) => Some(Stopped::MayRun),
        (Some(_), None) if exited.is_some() && output.exit_status.is_some() => None,
        (Some(_), None) => Some(Stopped::MayRun),
    };
    Ok(CommandRun { output, stopped })
}

/// What `sh`'s pipes have given [`run_local`].
struct LocalPipes {
    output: ExecOutput,
    out_buf: Vec<u8>,
    err_buf: Vec<u8>,
    out_open: bool,
    err_open: bool,
}

impl Default for LocalPipes {
    fn default() -> Self {
        Self {
            output: ExecOutput::default(),
            out_buf: vec![0; 32 * 1024],
            err_buf: vec![0; 32 * 1024],
            out_open: true,
            err_open: true,
        }
    }
}

impl LocalPipes {
    fn take_out(&mut self, read: std::io::Result<usize>) -> Result<(), SshError> {
        match read {
            Ok(0) | Err(_) => self.out_open = false,
            Ok(n) => push(&mut self.output.stdout, &self.out_buf[..n])?,
        }
        Ok(())
    }

    fn take_err(&mut self, read: std::io::Result<usize>) -> Result<(), SshError> {
        match read {
            Ok(0) | Err(_) => self.err_open = false,
            Ok(n) => push(&mut self.output.stderr, &self.err_buf[..n])?,
        }
        Ok(())
    }
}

fn push(to: &mut Vec<u8>, data: &[u8]) -> Result<(), SshError> {
    if to.len() + data.len() > MAX_EXEC_OUTPUT {
        return Err(SshError::other("command output is too large"));
    }
    to.extend_from_slice(data);
    Ok(())
}

/// Opens an exec channel, counted among the connection's exec channels while the slot lives, so a
/// channel the server refuses is opened again once another closes, other than the `held` ones the
/// caller has open, unless `stop` completes first.
async fn open(
    session: &SshSession,
    stop: Pin<&mut impl Future<Output = ()>>,
    held: usize,
) -> Result<Option<(ChannelReadHalf, ExecWriter, ExecSlot)>, SshError> {
    tokio::select! {
        biased;
        () = stop => Ok(None),
        opened = session.open_exec_channel_beside(held) => opened.map(Some),
    }
}

/// A command stopped before anything was sent to the host.
fn not_started() -> CommandRun {
    CommandRun {
        output: ExecOutput::default(),
        stopped: Some(Stopped::NotStarted),
    }
}

/// What a channel has sent.
#[derive(Default)]
struct Collected {
    output: ExecOutput,
    /// The server accepted the exec request.
    accepted: bool,
    /// Output goes to `pending`, for [`Self::send_next`] or [`Self::flush`] to send, instead of
    /// into `output`.
    forward: bool,
    pending: VecDeque<Output>,
}

/// Whether a channel goes on after a message.
enum Step {
    More,
    Ended,
}

impl Collected {
    /// Adds a message of the channel, as [`SshSession::exec_output`] reads it.
    fn take(&mut self, session: &SshSession, msg: Option<ChannelMsg>) -> Result<Step, SshError> {
        let out = &mut self.output;
        match msg {
            Some(ChannelMsg::Success) => self.accepted = true,
            Some(ChannelMsg::Failure) => {
                return Err(SshError::channel("the server refused to run the command"));
            }
            Some(ChannelMsg::Data { data }) if self.forward => {
                self.pending
                    .push_back(Output::Stdout(Bytes::copy_from_slice(&data)));
            }
            Some(ChannelMsg::ExtendedData { data, .. }) if self.forward => {
                self.pending
                    .push_back(Output::Stderr(Bytes::copy_from_slice(&data)));
            }
            Some(ChannelMsg::Data { data }) => {
                if out.stdout.len() + data.len() > MAX_EXEC_OUTPUT {
                    return Err(SshError::other("command output is too large"));
                }
                out.stdout.extend_from_slice(&data);
            }
            Some(ChannelMsg::ExtendedData { data, .. }) => {
                if out.stderr.len() + data.len() > MAX_EXEC_OUTPUT {
                    return Err(SshError::other("command output is too large"));
                }
                out.stderr.extend_from_slice(&data);
            }
            Some(ChannelMsg::ExitStatus { exit_status }) => out.exit_status = Some(exit_status),
            Some(ChannelMsg::Close) => return Ok(Step::Ended),
            Some(_) => {}
            None => {
                if out.exit_status.is_some() || self.accepted {
                    return Ok(Step::Ended);
                }
                return Err(session.close_error());
            }
        }
        Ok(Step::More)
    }

    /// Moves what `output` holds to `pending`.
    fn release(&mut self) {
        let stdout = std::mem::take(&mut self.output.stdout);
        let stderr = std::mem::take(&mut self.output.stderr);
        if !stdout.is_empty() {
            self.pending.push_back(Output::Stdout(stdout.into()));
        }
        if !stderr.is_empty() {
            self.pending.push_back(Output::Stderr(stderr.into()));
        }
    }

    /// Sends the first piece of `pending` once `sink` has room for it. A receiver that is gone
    /// takes nothing more.
    async fn send_next(&mut self, sink: Option<&mpsc::Sender<Output>>) {
        let permit = match sink {
            Some(sink) => sink.reserve().await.ok(),
            None => None,
        };
        match (permit, self.pending.pop_front()) {
            (Some(permit), Some(piece)) => permit.send(piece),
            _ => self.pending.clear(),
        }
    }

    /// Sends what of `pending` `sink` has room for now, in order, and drops the rest: after a stop,
    /// a receiver that stopped reading would hold up the end.
    fn offer(&mut self, sink: Option<&mpsc::Sender<Output>>) {
        let Some(sink) = sink else {
            return;
        };
        for piece in std::mem::take(&mut self.pending) {
            if sink.try_send(piece).is_err() {
                return;
            }
        }
    }

    /// Sends `pending` to `sink`, in order.
    async fn flush(&mut self, sink: Option<&mpsc::Sender<Output>>) {
        let Some(sink) = sink else {
            return;
        };
        for piece in std::mem::take(&mut self.pending) {
            // A receiver that is gone takes nothing more.
            if sink.send(piece).await.is_err() {
                return;
            }
        }
    }
}

/// Which of the marker's lines [`take_line`] takes.
#[derive(Clone, Copy)]
enum Find {
    First,
    Last,
}

/// Removes the line `{marker} {word}\n` from `stdout` and returns the word. What comes before it
/// on its line stays: output without a line break at its end, or that the login shell's startup
/// files printed. `None` until the whole line is there.
fn take_line(stdout: &mut Vec<u8>, marker: &str, find: Find) -> Option<String> {
    let prefix = format!("{marker} ");
    let prefix = prefix.as_bytes();
    let last = stdout.len().checked_sub(prefix.len())?;
    let line = |start: usize| {
        if !stdout[start..].starts_with(prefix) {
            return None;
        }
        let word = start + prefix.len();
        let end = stdout[word..].iter().position(|&b| b == b'\n')? + word;
        let text = &stdout[word..end];
        (!text.is_empty() && text.iter().all(u8::is_ascii_lowercase)).then_some((start, word, end))
    };
    let (start, word, end) = match find {
        Find::First => (0..=last).find_map(line)?,
        Find::Last => (0..=last).rev().find_map(line)?,
    };
    let text = String::from_utf8_lossy(&stdout[word..end]).into_owned();
    stdout.drain(start..=end);
    Some(text)
}

/// Removes the line `{marker}-in {path}\n`, which [`SCRIPT`] prints when it made a pipe for the
/// command's stdin, and returns the path.
fn take_pipe_line(stdout: &mut Vec<u8>, marker: &str) -> Option<String> {
    let prefix = format!("{marker}-in ");
    let start = stdout
        .windows(prefix.len())
        .position(|w| w == prefix.as_bytes())?;
    let path = start + prefix.len();
    let end = stdout[path..].iter().position(|&b| b == b'\n')? + path;
    let text = String::from_utf8(stdout[path..end].to_vec()).ok()?;
    stdout.drain(start..=end);
    (!text.is_empty()).then_some(text)
}

/// Takes out the line break that sudo-rs before 0.2.15 prints on stderr once it has read the
/// password, even with an empty prompt, from what came before the script was ready: alone, or after
/// a line of the login shell's startup files.
fn drop_sudo_line_break(stderr: &mut Vec<u8>) {
    if stderr == b"\n" || stderr.ends_with(b"\n\n") {
        stderr.pop();
    }
}

/// The number the script is given: random, so that output cannot hold its marker by chance, and
/// small enough for the arithmetic of any `sh`.
fn nonce() -> u32 {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let n = RandomState::new().hash_one(COUNT.fetch_add(1, Ordering::Relaxed));
    u32::try_from(n % 1_000_000_000).unwrap_or(0)
}

/// The script's marker for the number `n` it is given: the number after it, which the script
/// works out, so that the command line does not hold the marker.
fn marker(n: u32) -> String {
    format!("hatoba-run-{}", n + 1)
}

async fn sleep_until(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sudo::tests::sudo_with;

    /// As for the file tools' scripts: one line without single quotes or backslashes, and no
    /// `!` but that of `$!`, which csh and tcsh leave alone even with their history on.
    #[test]
    fn script_survives_any_login_shell() {
        for script in [SCRIPT, INPUT_SCRIPT] {
            assert!(!script.contains(['\'', '\\', '\n']));
            assert!(!script.replace("$!", "").contains('!'));
        }
    }

    #[test]
    fn the_pipe_line_gives_the_path() {
        let mut out = b"motd\nhatoba-run-7-in /tmp/tmp.a b/in\nhatoba-run-7 setsid\n".to_vec();
        assert_eq!(
            take_pipe_line(&mut out, "hatoba-run-7").as_deref(),
            Some("/tmp/tmp.a b/in")
        );
        assert_eq!(out, b"motd\nhatoba-run-7 setsid\n");
        let mut out = b"hatoba-run-7 setsid\n".to_vec();
        assert_eq!(take_pipe_line(&mut out, "hatoba-run-7"), None);
    }

    #[test]
    fn take_line_keeps_what_comes_before_and_after_the_marker() {
        let mut out = b"motd\nhatoba-run-7 setsid\nrest".to_vec();
        assert_eq!(
            take_line(&mut out, "hatoba-run-7", Find::First).as_deref(),
            Some("setsid")
        );
        assert_eq!(out, b"motd\nrest");

        let mut out = b"no line break at its endhatoba-run-7 ended\n".to_vec();
        assert_eq!(
            take_line(&mut out, "hatoba-run-7", Find::Last).as_deref(),
            Some("ended")
        );
        assert_eq!(out, b"no line break at its end");

        // Incomplete, or another number.
        for text in [
            &b"hatoba-run-7 sets"[..],
            b"hatoba-run-71 ended\n",
            b"hatoba-run-7 \n",
        ] {
            let mut out = text.to_vec();
            assert_eq!(take_line(&mut out, "hatoba-run-7", Find::Last), None);
            assert_eq!(out, text);
        }

        let mut out = b"hatoba-run-7 a\nx\nhatoba-run-7 b\n".to_vec();
        assert_eq!(
            take_line(&mut out, "hatoba-run-7", Find::Last).as_deref(),
            Some("b")
        );
        assert_eq!(out, b"hatoba-run-7 a\nx\n");
    }

    #[test]
    fn sudo_rs_line_break_is_dropped() {
        for (before, after) in [
            (&b"\n"[..], &b""[..]),
            (b"motd\n\n", b"motd\n"),
            (b"", b""),
            (b"motd\n", b"motd\n"),
        ] {
            let mut stderr = before.to_vec();
            drop_sudo_line_break(&mut stderr);
            assert_eq!(stderr, after);
        }
    }

    #[test]
    fn the_command_line_does_not_hold_the_marker() {
        for n in [0, 1, 9, 41, 999_999_999]
            .into_iter()
            .chain((0..100).map(|_| nonce()))
        {
            for run_as in [RunAs::Login, RunAs::Sudo(None), sudo_with("x")] {
                for input in [false, true] {
                    let line = command_line(&run_as, n, input);
                    assert!(!line.contains(&marker(n)), "{line}");
                }
            }
        }
    }

    /// The script with the local `sh`, started as a login shell would start it.
    #[cfg(unix)]
    mod local_sh {
        use std::io::{BufRead, BufReader, Read, Write};
        use std::process::{Child, Command, Stdio};
        use std::thread;
        #[cfg(target_os = "linux")]
        use std::time::Instant;

        use super::super::*;
        use crate::sudo::tests::sudo_with;

        struct Local {
            child: Child,
            marker: String,
            ready: String,
        }

        /// Starts the script as sshd would, with `SHELL` set, and sends `command` once it is ready.
        fn start(path: Option<&str>, command: &str) -> Local {
            let mut cmd = Command::new("sh");
            if let Some(path) = path {
                cmd.env("PATH", path);
            }
            start_in(cmd, "", command)
        }

        /// [`start`] with `cmd`, which may set the environment and the directory, and the
        /// directory line `workdir`.
        fn start_in(mut cmd: Command, workdir: &str, command: &str) -> Local {
            let n = nonce();
            let marker = marker(n);
            cmd.args(["-c", SCRIPT, "sh", &n.to_string()])
                .env("SHELL", "/bin/sh")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            let mut child = cmd.spawn().unwrap();
            let mut ready = String::new();
            BufReader::new(child.stdout.as_mut().unwrap())
                .read_line(&mut ready)
                .unwrap();
            let lines = command.matches('\n').count() + 1;
            write!(
                child.stdin.as_mut().unwrap(),
                "{lines}\n{workdir}\n{command}\n{marker}-end\n"
            )
            .unwrap();
            Local {
                child,
                marker,
                ready,
            }
        }

        /// Waits for the script, after closing its stdin first when `stop` is given (in seconds),
        /// and returns its exit status, stdout and stderr.
        fn finish(mut local: Local, stop: Option<f64>) -> (Option<i32>, String, String) {
            let mut stdout = local.child.stdout.take().unwrap();
            let mut stderr = local.child.stderr.take().unwrap();
            let out = thread::spawn(move || {
                let mut s = String::new();
                stdout.read_to_string(&mut s).unwrap();
                s
            });
            let err = thread::spawn(move || {
                let mut s = String::new();
                stderr.read_to_string(&mut s).unwrap();
                s
            });
            let stdin = local.child.stdin.take();
            if let Some(secs) = stop {
                thread::sleep(Duration::from_secs_f64(secs));
                drop(stdin);
            } else {
                // Kept open until the script ends, as the channel keeps it.
                let status = local.child.wait().unwrap();
                drop(stdin);
                return (status.code(), out.join().unwrap(), err.join().unwrap());
            }
            let status = local.child.wait().unwrap();
            (status.code(), out.join().unwrap(), err.join().unwrap())
        }

        /// The processes whose command line holds `needle`.
        #[cfg(target_os = "linux")]
        fn running(needle: &str) -> Vec<String> {
            let out = Command::new("ps")
                .args(["-eo", "pid=,args="])
                .output()
                .unwrap();
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .filter(|l| l.contains(needle) && !l.contains("ps -eo"))
                .map(str::to_owned)
                .collect()
        }

        /// A stdin to pass goes through the pipe the script makes, from a second `sh` that runs
        /// [`INPUT_SCRIPT`], as a second channel would, and the pipe is gone once the script ends.
        #[test]
        fn script_passes_a_stdin_through_its_pipe() {
            let n = nonce();
            let marker = marker(n);
            let mut child = Command::new("sh")
                .args(["-c", SCRIPT, "sh", &n.to_string(), "", "", "1"])
                .env("SHELL", "/bin/sh")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            let mut pipe_line = String::new();
            stdout.read_line(&mut pipe_line).unwrap();
            let mut ready = String::new();
            stdout.read_line(&mut ready).unwrap();
            assert!(ready.starts_with(&format!("{marker} ")), "{ready}");
            let mut held = pipe_line.into_bytes();
            let pipe = take_pipe_line(&mut held, &marker).unwrap();
            assert!(held.is_empty());

            let mut input = Command::new("sh")
                .args(["-c", INPUT_SCRIPT])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            // The whole stdin, and its end, before the command starts: it still gets all of it.
            let mut input_stdin = input.stdin.take().unwrap();
            write!(input_stdin, "{pipe}\nfirst\nsecond\n").unwrap();
            drop(input_stdin);
            let mut opened = String::new();
            BufReader::new(input.stdout.take().unwrap())
                .read_line(&mut opened)
                .unwrap();
            assert_eq!(opened, format!("{INPUT_OPEN}\n"));
            assert!(input.wait().unwrap().success());

            let command = "cat; echo done";
            let mut stdin = child.stdin.take().unwrap();
            write!(stdin, "1\n\n{command}\n{marker}-end\n").unwrap();
            let status = child.wait().unwrap();
            drop(stdin);
            let mut rest = String::new();
            stdout.read_to_string(&mut rest).unwrap();
            assert_eq!(status.code(), Some(0));
            assert_eq!(rest, "first\nsecond\ndone\n");
            assert!(!std::path::Path::new(&pipe).exists());
        }

        #[test]
        fn script_runs_a_command_and_keeps_its_output_and_status() {
            let local = start(None, "echo out; printf 'two\\nlines' >&2\nexit 3");
            assert!(local.ready.starts_with(&format!("{} ", local.marker)));
            let (status, stdout, stderr) = finish(local, None);
            assert_eq!(status, Some(3));
            assert_eq!(stdout, "out\n");
            assert_eq!(stderr, "two\nlines");
        }

        /// Through sudo (the tests' own, which gives the command a `HOME` and `SHELL` of its own),
        /// the password goes first, and the directory and the shell are still the login user's.
        #[test]
        fn script_runs_through_sudo_from_the_login_users_home() {
            let home = tempfile::tempdir().unwrap();
            let home = home.path().canonicalize().unwrap();
            std::fs::create_dir(home.join("sub")).unwrap();
            let (_sudo, path) = crate::sudo::tests::fake::sudo_dir();
            let started = |password: &str| {
                let n = nonce();
                let mut child = Command::new("sh")
                    .args(["-c", &command_line(&sudo_with("x"), n, false)])
                    .current_dir(&home)
                    .env("HOME", &home)
                    .env("SHELL", "/bin/sh")
                    .env("PATH", &path)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap();
                writeln!(child.stdin.as_mut().unwrap(), "{password}").unwrap();
                (child, marker(n))
            };

            let (mut child, marker) = started("secret");
            let mut ready = String::new();
            BufReader::new(child.stdout.as_mut().unwrap())
                .read_line(&mut ready)
                .unwrap();
            write!(
                child.stdin.as_mut().unwrap(),
                "1\n~/sub\npwd -P; echo \"$HOME\"; cat; echo read $?\n{marker}-end\n"
            )
            .unwrap();
            let local = Local {
                child,
                marker,
                ready,
            };
            assert!(local.ready.starts_with(&format!("{} ", local.marker)));
            let (status, stdout, stderr) = finish(local, None);
            assert_eq!(status, Some(0), "{stderr}");
            assert_eq!(
                stdout,
                format!(
                    "{}\n/nonexistent-root\nread 0\n",
                    home.join("sub").display()
                )
            );

            // A wrong password runs nothing, and sudo says why.
            let (child, _) = started("guess");
            let out = child.wait_with_output().unwrap();
            assert_eq!(out.status.code(), Some(1));
            assert!(out.stdout.is_empty());
            assert_eq!(
                sudo::refused(&out.stderr).message,
                "sudo: 1 incorrect password attempt"
            );
        }

        #[test]
        fn script_gives_the_command_an_empty_stdin() {
            let local = start(None, "cat; echo read $?");
            let (status, stdout, _) = finish(local, None);
            assert_eq!((status, stdout.as_str()), (Some(0), "read 0\n"));
        }

        /// With GNU `env`, the command does not inherit the SIGINT and SIGQUIT that a script
        /// ignores for what it starts in the background.
        #[cfg(target_os = "linux")]
        #[test]
        fn script_gives_the_command_default_signals() {
            let gnu_env = Command::new("env")
                .args(["--default-signal=INT,QUIT", "true"])
                .status()
                .is_ok_and(|s| s.success());
            let local = start(None, "grep SigIgn /proc/$$/status");
            assert!(local.ready.ends_with(" setsid\n"), "{}", local.ready);
            let (_, stdout, _) = finish(local, None);
            if gnu_env {
                assert_eq!(stdout, "SigIgn:\t0000000000000000\n");
            }
        }

        #[test]
        fn script_does_not_run_a_command_cut_short() {
            let mut child = Command::new("sh")
                .args(["-c", SCRIPT, "sh", "41"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"2\n\necho first\n")
                .unwrap();
            let out = child.wait_with_output().unwrap();
            assert_eq!(out.status.code(), Some(9));
            assert!(!String::from_utf8_lossy(&out.stdout).contains("first"));
        }

        #[test]
        fn script_starts_the_command_in_workdir() {
            let home = tempfile::tempdir().unwrap();
            let home = home.path().canonicalize().unwrap();
            for dir in ["a b", "-x", "sub", "sub/a b"] {
                std::fs::create_dir(home.join(dir)).unwrap();
            }
            let pwd = |workdir: &str| {
                let mut cmd = Command::new("sh");
                // A relative directory is from HOME, not from where the script started (as after
                // a startup file's cd) or from CDPATH, both of which hold an `a b` of their own.
                cmd.current_dir(home.join("sub"))
                    .env("HOME", &home)
                    .env("CDPATH", home.join("sub"));
                finish(start_in(cmd, workdir, "pwd -P"), None)
            };
            let absolute = home.join("sub");
            for (workdir, expected) in [
                ("", home.join("sub")),
                ("~", home.clone()),
                ("~/a b", home.join("a b")),
                ("a b", home.join("a b")),
                ("-x", home.join("-x")),
                (absolute.to_str().unwrap(), absolute.clone()),
            ] {
                let (status, stdout, stderr) = pwd(workdir);
                assert_eq!(
                    (status, stdout),
                    (Some(0), format!("{}\n", expected.display())),
                    "{workdir}: {stderr}"
                );
            }
            // A directory it cannot change to runs nothing, and says why.
            let (status, stdout, stderr) = pwd("missing");
            assert_ne!(status, Some(0));
            assert_eq!(stdout, "");
            assert!(stderr.contains("missing"), "{stderr}");
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn script_ends_the_whole_command_when_its_stdin_ends() {
            let tag = format!("31{}", nonce() % 1000);
            // `sleep` is not the shell's last command, so the shell forks it and waits.
            for command in [
                format!("sleep {tag}"),
                format!("cd /tmp && sleep {tag}; echo after"),
            ] {
                let local = start(None, &command);
                let marker = local.marker.clone();
                let began = Instant::now();
                let (_, stdout, _) = finish(local, Some(0.5));
                assert!(began.elapsed() < Duration::from_secs(3), "{command}");
                assert_eq!(stdout, format!("{marker} ended\n"), "{command}");
                assert!(running(&format!("sleep {tag}")).is_empty(), "{command}");
            }
        }

        /// A stop that comes with the command, before the command's process group exists,
        /// still ends it: with TERM, not KILL after the grace period, and nothing runs after it.
        #[cfg(target_os = "linux")]
        #[test]
        fn script_ends_a_command_stopped_as_it_arrives() {
            for _ in 0..5 {
                let tag = format!("35{}", nonce() % 1000);
                let local = start(None, &format!("sleep {tag}; echo survived"));
                let marker = local.marker.clone();
                let began = Instant::now();
                let (status, stdout, _) = finish(local, Some(0.0));
                assert_eq!(stdout, format!("{marker} ended\n"));
                assert_eq!(status, Some(143));
                assert!(began.elapsed() < Duration::from_secs(2));
                assert!(running(&format!("sleep {tag}")).is_empty());
            }
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn script_kills_what_ignores_term() {
            let tag = format!("32{}", nonce() % 1000);
            let local = start(None, &format!("trap '' TERM; sleep {tag}; echo after"));
            let marker = local.marker.clone();
            let (status, stdout, _) = finish(local, Some(0.3));
            assert_eq!(stdout, format!("{marker} ended\n"));
            assert_eq!(status, Some(137));
            assert!(running(&format!("sleep {tag}")).is_empty());
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn script_leaves_a_finished_commands_background_jobs_alone() {
            let tag = format!("33{}", nonce() % 1000);
            let local = start(
                None,
                &format!("sleep {tag} >/dev/null 2>&1 </dev/null & echo started"),
            );
            let (status, stdout, _) = finish(local, None);
            assert_eq!((status, stdout.as_str()), (Some(0), "started\n"));
            let left = running(&format!("sleep {tag}"));
            assert_eq!(left.len(), 1, "{left:?}");
            let pid = left[0].split_whitespace().next().unwrap();
            Command::new("kill").arg(pid).status().unwrap();
        }

        /// A directory of links to the tools the script needs, without `setsid` and with or without
        /// `perl`, as on macOS and the BSDs.
        #[cfg(target_os = "linux")]
        fn tools_without_setsid(perl: bool) -> tempfile::TempDir {
            let dir = tempfile::tempdir().unwrap();
            let mut tools = vec!["sh", "head", "env", "sleep", "cat", "grep"];
            if perl {
                tools.push("perl");
            }
            for tool in tools {
                let out = Command::new("sh")
                    .args(["-c", &format!("command -v {tool}")])
                    .output()
                    .unwrap();
                let path = String::from_utf8_lossy(&out.stdout).trim().to_owned();
                std::os::unix::fs::symlink(path, dir.path().join(tool)).unwrap();
            }
            dir
        }

        #[cfg(target_os = "linux")]
        #[test]
        fn script_falls_back_to_perl_and_then_to_the_commands_own_process() {
            let has_perl = Command::new("sh")
                .args(["-c", "command -v perl"])
                .output()
                .is_ok_and(|o| o.status.success());
            for (perl, way, report) in [(true, "perl", "ended"), (false, "none", "unknown")] {
                if perl && !has_perl {
                    continue;
                }
                let dir = tools_without_setsid(perl);
                let path = dir.path().to_str().unwrap();
                let tag = format!("34{}", nonce() % 1000);
                // Without a group, TERM reaches only the command's own process.
                let local = start(Some(path), &format!("exec sleep {tag}"));
                assert_eq!(local.ready, format!("{} {way}\n", local.marker));
                let marker = local.marker.clone();
                let (_, stdout, _) = finish(local, Some(0.3));
                assert_eq!(stdout, format!("{marker} {report}\n"), "{way}");
                assert!(running(&format!("sleep {tag}")).is_empty(), "{way}");

                let local = start(Some(path), "grep SigIgn /proc/$$/status");
                let (_, stdout, _) = finish(local, None);
                if perl {
                    assert_eq!(stdout, "SigIgn:\t0000000000000000\n");
                }
            }
        }
    }
}
