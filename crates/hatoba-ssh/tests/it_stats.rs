//! Resource usage sampling (TERM-12) against a real OpenSSH `sshd` (set `HATOBA_SSH_IT=1`).
#![cfg(unix)]

mod common;

use std::time::Duration;

use common::*;
use hatoba_ssh::{ServerStats, SshErrorKind, StatsEvent};
use tokio::sync::mpsc;

macro_rules! server {
    () => {
        match SshdServer::start() {
            Some(s) => s,
            None => return,
        }
    };
}

async fn next(rx: &mut mpsc::Receiver<StatsEvent>) -> Option<StatsEvent> {
    tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("no stats event within 10 s")
}

async fn reading(rx: &mut mpsc::Receiver<StatsEvent>) -> ServerStats {
    match next(rx).await {
        Some(StatsEvent::Stats(stats)) => *stats,
        other => panic!("expected a reading, got {other:?}"),
    }
}

/// Sampling loops of the given interval still running on the server.
async fn remote_loops(session: &hatoba_ssh::SshSession, secs: u32) -> u32 {
    // The brackets keep the pattern from matching the shell that runs pgrep.
    let (_, out) = session
        .exec(&format!("pgrep -fc '[s]h -s -- {secs}$' || true"), None)
        .await
        .unwrap();
    String::from_utf8_lossy(&out).trim().parse().unwrap_or(0)
}

#[tokio::test]
async fn readings_arrive_each_interval_and_stop_with_the_handle() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (handle, mut rx) = session.open_stats(Duration::from_secs(1)).await.unwrap();

    if !cfg!(target_os = "linux") {
        match next(&mut rx).await {
            Some(StatsEvent::Unsupported(system)) => assert!(!system.is_empty()),
            other => panic!("expected unsupported, got {other:?}"),
        }
        return;
    }

    let first = reading(&mut rx).await;
    assert_eq!(first.cpu_percent, None);
    assert!(first.cpus.unwrap() >= 1);
    assert!(first.mem_total.unwrap() > first.mem_used.unwrap());
    assert!(first.load.is_some());
    assert!(first.uptime_secs.is_some());
    assert!(first.disk_total.unwrap() >= first.disk_used.unwrap());

    let second = reading(&mut rx).await;
    let cpu = second.cpu_percent.expect("a CPU share from two readings");
    assert!((0.0..=100.0).contains(&cpu), "{cpu}");
    assert!(second.uptime_secs >= first.uptime_secs);
    assert!(remote_loops(&session, 1).await >= 1);

    handle.stop();
    assert!(!handle.is_running());
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(event) = rx.recv().await {
            assert!(
                matches!(event, StatsEvent::Stats(_)),
                "nothing but readings already on their way: {event:?}"
            );
        }
    })
    .await
    .expect("the event stream should end after stop");
    // The loop on the server ends with its channel, and the connection stays usable.
    tokio::time::timeout(Duration::from_secs(5), async {
        while remote_loops(&session, 1).await > 0 {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("the sampling loop should end on the server");
    assert_eq!(session.exec("echo fine", None).await.unwrap().1, b"fine\n");
}

#[tokio::test]
async fn dropping_the_handle_stops_sampling() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (handle, mut rx) = session.open_stats(Duration::from_secs(2)).await.unwrap();
    if cfg!(target_os = "linux") {
        reading(&mut rx).await;
    }
    drop(handle);
    tokio::time::timeout(Duration::from_secs(5), async {
        while rx.recv().await.is_some() {}
    })
    .await
    .expect("the event stream should end when the handle is dropped");
}

#[tokio::test]
async fn sampling_ends_with_the_connection() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (_handle, mut rx) = session.open_stats(Duration::from_secs(1)).await.unwrap();
    if !cfg!(target_os = "linux") {
        return;
    }
    reading(&mut rx).await;
    session.disconnect().await;
    loop {
        match next(&mut rx).await {
            Some(StatsEvent::Stats(_)) => {}
            Some(StatsEvent::Ended(err)) => {
                assert_eq!(err.kind, SshErrorKind::Disconnected, "{err:?}");
                break;
            }
            other => panic!("expected the end of sampling, got {other:?}"),
        }
    }
    assert!(next(&mut rx).await.is_none());
}
