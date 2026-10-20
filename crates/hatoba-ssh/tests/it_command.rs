//! Integration tests against a real OpenSSH `sshd` (set `HATOBA_SSH_IT=1`): the AI assistant's
//! `run_command` (AI-12), and what stopping it or its timeout leaves on the host (AI-18).
#![cfg(unix)]

mod common;

use std::process::Command;
use std::time::{Duration, Instant};

use common::*;
use hatoba_ssh::remote_command::{self, Output, Stopped};
use tokio_util::sync::CancellationToken;

macro_rules! server {
    () => {
        match SshdServer::start() {
            Some(s) => s,
            None => return,
        }
    };
}

/// The host's processes whose command line is exactly `args`.
fn processes(args: &str) -> Vec<String> {
    let out = Command::new("ps")
        .args(["-eo", "pid=,args="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| {
            l.trim_start()
                .split_once(' ')
                .is_some_and(|(_, a)| a == args)
        })
        .map(str::to_owned)
        .collect()
}

/// Waits until `args` runs on the host, so a stop finds the command started.
async fn started(args: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while processes(args).is_empty() {
        assert!(Instant::now() < deadline, "`{args}` did not start");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_command_runs_with_the_login_shell_and_keeps_its_output() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let command = "printf 'a\\nb'; echo \"$0 $(id -un)\" >&2\nexit 7";
    let run = remote_command::run(&session, command, None, std::future::pending())
        .await
        .unwrap();
    assert_eq!(run.stopped, None);
    assert_eq!(run.output.exit_status, Some(7));
    assert_eq!(run.output.stdout, b"a\nb");
    let shell = Command::new("sh")
        .args(["-c", "getent passwd \"$(id -un)\" | cut -d: -f7"])
        .output()
        .unwrap();
    let shell = String::from_utf8_lossy(&shell.stdout).trim().to_owned();
    assert_eq!(
        String::from_utf8_lossy(&run.output.stderr),
        format!("{shell} {}\n", server.key_user)
    );

    // The command gets an empty stdin, as `exec_output` without one gives it.
    let run = remote_command::run(&session, "cat; echo done", None, std::future::pending())
        .await
        .unwrap();
    assert_eq!(run.output.stdout, b"done\n");
}

/// AI-12: `workdir` is where the command starts: absolute, or from the home directory.
#[tokio::test]
async fn a_command_starts_in_workdir() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let dir = server.area("workdir").join("a b");
    std::fs::create_dir(&dir).unwrap();
    let run = remote_command::run(&session, "pwd -P", dir.to_str(), std::future::pending())
        .await
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&run.output.stdout),
        format!("{}\n", dir.canonicalize().unwrap().display())
    );
    let run = remote_command::run(
        &session,
        "[ \"$(pwd)\" = \"$HOME\" ] && echo home",
        Some("~"),
        std::future::pending(),
    )
    .await
    .unwrap();
    assert_eq!(run.output.stdout, b"home\n");
    // A directory it cannot change to runs nothing.
    let run = remote_command::run(
        &session,
        "echo ran",
        Some("/no/such/dir"),
        std::future::pending(),
    )
    .await
    .unwrap();
    assert_ne!(run.output.exit_status, Some(0));
    assert!(run.output.stdout.is_empty());
}

/// AI-18: stopping a command ends it and every process it started: `sh -c` runs `sleep` as a
/// child, and so does the login shell for `cd /tmp && sleep`, so ending only the process sshd
/// started would leave `sleep` running.
#[tokio::test]
async fn a_stopped_command_leaves_no_process_behind() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    for (command, sleep) in [
        ("echo started; sh -c 'sleep 301'", "sleep 301"),
        ("echo started; cd /tmp && sleep 302", "sleep 302"),
    ] {
        let stop = CancellationToken::new();
        let stopping = {
            let stop = stop.clone();
            async move {
                started(sleep).await;
                stop.cancel();
            }
        };
        let began = Instant::now();
        let (run, ()) = tokio::join!(
            remote_command::run(&session, command, None, stop.cancelled()),
            stopping
        );
        let run = run.unwrap();
        assert_eq!(run.stopped, Some(Stopped::Ended), "{command}");
        assert_eq!(run.output.stdout, b"started\n", "{command}");
        assert!(began.elapsed() < Duration::from_secs(10), "{command}");
        assert!(
            processes(sleep).is_empty(),
            "{command}: {:?}",
            processes(sleep)
        );
    }
}

/// AI-12: a command whose timeout passes is ended the same way.
#[tokio::test]
async fn a_timed_out_command_leaves_no_process_behind() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    for (command, sleep) in [
        ("sh -c 'sleep 303'", "sleep 303"),
        ("cd /tmp && sleep 304", "sleep 304"),
    ] {
        let run = remote_command::run(
            &session,
            command,
            None,
            tokio::time::sleep(Duration::from_secs(2)),
        )
        .await
        .unwrap();
        assert_eq!(run.stopped, Some(Stopped::Ended), "{command}");
        assert!(
            processes(sleep).is_empty(),
            "{command}: {:?}",
            processes(sleep)
        );
    }
}

/// A command that ignores TERM gets KILL after the grace period.
#[tokio::test]
async fn a_command_that_ignores_term_is_killed() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let stop = async {
        started("sleep 305").await;
    };
    let began = Instant::now();
    let run = remote_command::run(&session, "trap '' TERM; sleep 305; echo after", None, stop)
        .await
        .unwrap();
    assert_eq!(run.stopped, Some(Stopped::Ended));
    assert!(began.elapsed() >= Duration::from_secs(2));
    assert!(began.elapsed() < remote_command::STOP_WAIT + Duration::from_secs(5));
    assert!(run.output.stdout.is_empty());
    assert!(processes("sleep 305").is_empty());
}

/// A stop before the command reached the host runs nothing.
#[tokio::test]
async fn a_stop_before_the_command_starts_runs_nothing() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let marker = server.area("not_started").join("ran");
    let command = format!("touch '{}'", marker.display());
    let run = remote_command::run(&session, &command, None, std::future::ready(()))
        .await
        .unwrap();
    assert_eq!(run.stopped, Some(Stopped::NotStarted));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!marker.exists());
}

/// A login shell that cannot run the script, here because sshd runs a command of its own, gets
/// the command as it is, as before.
#[tokio::test]
async fn a_host_that_cannot_run_the_script_runs_the_command_as_it_is() {
    let Some(server) =
        SshdServer::start_configured(HostKeyType::Ed25519, "ForceCommand echo forced\n")
    else {
        return;
    };
    let session = server.connect_key("ed25519", None).await;
    for _ in 0..2 {
        let run = remote_command::run(&session, "echo asked", None, std::future::pending())
            .await
            .unwrap();
        assert_eq!(run.stopped, None);
        assert_eq!(run.output.stdout, b"forced\n");
        assert_eq!(run.output.exit_status, Some(0));
    }
    // Without the script, nothing can start the command in a directory.
    let refused = remote_command::run(&session, "echo asked", Some("/tmp"), std::future::pending())
        .await
        .unwrap_err();
    assert!(refused.message.contains("workdir"), "{}", refused.message);
}

/// What `run_streaming` sent, split into stdout and stderr.
async fn received(mut rx: tokio::sync::mpsc::Receiver<Output>) -> (Vec<u8>, Vec<u8>) {
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    while let Some(piece) = rx.recv().await {
        match piece {
            Output::Stdout(b) => stdout.extend_from_slice(&b),
            Output::Stderr(b) => stderr.extend_from_slice(&b),
        }
    }
    (stdout, stderr)
}

/// HOST-15: `hatoba exec` gets the output while the command still runs, and keeps none of it.
#[tokio::test]
async fn streamed_output_arrives_while_the_command_runs() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    let command = "echo first; sleep 2; echo second; echo err >&2; exit 4";
    let run = remote_command::run_streaming(&session, command, None, std::future::pending(), tx);
    tokio::pin!(run);
    let began = Instant::now();
    let first = tokio::select! {
        piece = rx.recv() => piece.unwrap(),
        _ = &mut run => panic!("the command ended before its first line arrived"),
    };
    assert!(began.elapsed() < Duration::from_secs(2));
    assert_eq!(first, Output::Stdout(bytes::Bytes::from_static(b"first\n")));
    let (run, (stdout, stderr)) = tokio::join!(run, received(rx));
    let run = run.unwrap();
    assert_eq!(run.stopped, None);
    assert_eq!(run.output.exit_status, Some(4));
    assert!(run.output.stdout.is_empty() && run.output.stderr.is_empty());
    assert_eq!(stdout, b"second\n");
    assert_eq!(stderr, b"err\n");
}

/// The script's report on a stop is not passed on as output.
#[tokio::test]
async fn a_stopped_streamed_command_passes_on_only_its_own_output() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (tx, rx) = tokio::sync::mpsc::channel(8);
    let stop = async {
        started("sleep 306").await;
    };
    let (run, (stdout, _)) = tokio::join!(
        remote_command::run_streaming(&session, "echo started; sleep 306", None, stop, tx),
        received(rx)
    );
    let run = run.unwrap();
    assert_eq!(run.stopped, Some(Stopped::Ended));
    assert_eq!(stdout, b"started\n");
    assert!(processes("sleep 306").is_empty());
}

/// HOST-15: a program that stops reading the output, as a pager does, does not hold up a stop.
#[tokio::test]
async fn a_stop_ends_a_streamed_command_whose_output_nobody_reads() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (tx, _unread) = tokio::sync::mpsc::channel(1);
    let command = "while :; do echo line; sleep 0.01; done";
    let stop = tokio::time::sleep(Duration::from_secs(1));
    let run = tokio::time::timeout(
        Duration::from_secs(20),
        remote_command::run_streaming(&session, command, None, stop, tx),
    )
    .await
    .expect("the stop waited for the output to be read")
    .unwrap();
    assert_eq!(run.stopped, Some(Stopped::Ended));
}
