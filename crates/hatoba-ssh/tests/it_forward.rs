//! Port forwarding and ProxyJump integration tests (set `HATOBA_SSH_IT=1`).
#![cfg(unix)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use hatoba_ssh::{
    AuthMethod, ConnectConfig, JumpHop, LocalForward, ShellEvent, ShellOptions, SshErrorKind,
    connect,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use zeroize::Zeroizing;

macro_rules! server {
    () => {
        match SshdServer::start() {
            Some(s) => s,
            None => return,
        }
    };
}

fn forward_to(port: u16) -> LocalForward {
    LocalForward {
        bind_address: "127.0.0.1".into(),
        bind_port: 0,
        dest_host: "127.0.0.1".into(),
        dest_port: port,
    }
}

/// Echo server on an ephemeral port.
async fn echo_server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let (mut r, mut w) = stream.split();
                let _ = tokio::io::copy(&mut r, &mut w).await;
            });
        }
    });
    port
}

// --------------------------------------------------------------------------- local forward

#[tokio::test]
async fn forward_reaches_the_ssh_banner() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let fwd = session
        .local_forward(forward_to(server.port))
        .await
        .unwrap();
    assert_ne!(fwd.local_port(), 0);
    assert!(fwd.is_running());

    let mut conn = TcpStream::connect(("127.0.0.1", fwd.local_port()))
        .await
        .unwrap();
    let mut banner = vec![0u8; 8];
    tokio::time::timeout(Duration::from_secs(5), conn.read_exact(&mut banner))
        .await
        .expect("banner")
        .unwrap();
    assert_eq!(&banner, b"SSH-2.0-");
    fwd.stop();
}

#[tokio::test]
async fn forward_moves_data_in_both_directions_concurrently() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let echo = echo_server().await;
    let fwd = session.local_forward(forward_to(echo)).await.unwrap();
    let port = fwd.local_port();

    let tasks: Vec<_> = (0..4u64)
        .map(|i| {
            tokio::spawn(async move {
                let data = pseudo_random(1024 * 1024 + 17, 100 + i);
                let mut conn = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                let (mut r, mut w) = conn.split();
                let expected = data.len();
                let writer = async {
                    w.write_all(&data).await.unwrap();
                    w.flush().await.unwrap();
                };
                let reader = async {
                    let mut got = vec![0u8; expected];
                    r.read_exact(&mut got).await.unwrap();
                    got
                };
                let (_, got) = tokio::time::timeout(Duration::from_secs(30), async {
                    tokio::join!(writer, reader)
                })
                .await
                .expect("echo round trip");
                assert_eq!(sha256_hex(&got), sha256_hex(&data));
            })
        })
        .collect();
    for t in tasks {
        t.await.unwrap();
    }
    fwd.stop();
}

#[tokio::test]
async fn stopping_a_forward_closes_listener_and_connections() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let echo = echo_server().await;
    let fwd = session.local_forward(forward_to(echo)).await.unwrap();
    let port = fwd.local_port();

    let mut conn = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    conn.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    conn.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");

    fwd.stop();
    assert!(!fwd.is_running());
    // The established connection ends...
    let n = tokio::time::timeout(Duration::from_secs(5), conn.read(&mut buf))
        .await
        .expect("closed")
        .unwrap_or(0);
    assert_eq!(n, 0);
    // ...and nobody listens any more.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_err());
    // The SSH session is unaffected.
    assert_eq!(session.exec("echo fine", None).await.unwrap().1, b"fine\n");
}

#[tokio::test]
async fn forward_to_a_closed_port_drops_the_local_connection() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let fwd = session
        .local_forward(forward_to(free_port()))
        .await
        .unwrap();
    let mut conn = TcpStream::connect(("127.0.0.1", fwd.local_port()))
        .await
        .unwrap();
    let mut buf = [0u8; 1];
    let n = tokio::time::timeout(Duration::from_secs(5), conn.read(&mut buf))
        .await
        .expect("connection should be closed")
        .unwrap_or(0);
    assert_eq!(n, 0);
    // The forward keeps working for the next connection.
    assert!(fwd.is_running());
}

#[tokio::test]
async fn forward_bind_errors_and_fixed_ports() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let first = session
        .local_forward(forward_to(server.port))
        .await
        .unwrap();
    // Same port again is an I/O error.
    let err = session
        .local_forward(LocalForward {
            bind_port: first.local_port(),
            ..forward_to(server.port)
        })
        .await
        .err()
        .expect("address in use");
    assert_eq!(err.kind, SshErrorKind::Io, "{err}");
    // An explicit free port is honoured; an empty bind address means loopback.
    let port = free_port();
    let second = session
        .local_forward(LocalForward {
            bind_address: String::new(),
            bind_port: port,
            ..forward_to(server.port)
        })
        .await
        .unwrap();
    assert_eq!(second.local_port(), port);
    assert!(second.local_addr().ip().is_loopback());
    first.stop();
    second.stop();
}

#[tokio::test]
async fn forward_stops_when_the_session_ends() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let fwd = session
        .local_forward(forward_to(server.port))
        .await
        .unwrap();
    session.disconnect().await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while fwd.is_running() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("forward should stop with its session");
}

// --------------------------------------------------------------------------- ProxyJump

fn hop(server: &SshdServer, host: &str, port: u16) -> JumpHop {
    JumpHop {
        host: host.into(),
        port,
        username: server.key_user.clone(),
        auth: key_auth("ed25519", None),
        agent_sockets: Vec::new(),
    }
}

#[tokio::test]
async fn proxy_jump_through_one_hop() {
    let server = server!();
    // client -> proxy -> sshd (jump) -> direct-tcpip -> sshd (target)
    let proxy = Proxy::start(server.port).await;
    let mut cfg = server.config(key_auth("rsa", None));
    cfg.jump = vec![hop(&server, "127.0.0.1", proxy.port)];
    let verifier = Verifier::accepting();
    let session = connect(cfg, verifier.clone()).await.unwrap();

    // Every hop's host key goes through the verifier, jump first.
    let calls = verifier.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].1, proxy.port);
    assert_eq!(calls[1].1, server.port);
    assert!(
        calls
            .iter()
            .all(|c| c.2.fingerprint == server.host_fingerprint)
    );

    assert_eq!(
        session.exec("echo via-jump", None).await.unwrap().1,
        b"via-jump\n"
    );
    assert!(session.latency_ms() >= 1);
    // Shell and SFTP work over the tunnel.
    let (shell, mut rx) = session.open_shell(ShellOptions::default()).await.unwrap();
    shell
        .write(b"echo jumped-$((6*7))\n".to_vec())
        .await
        .unwrap();
    read_until(&mut rx, Duration::from_secs(10), |o| {
        contains(o, "jumped-42")
    })
    .await;
    let sftp = session.sftp().await.unwrap();
    assert!(sftp.home().await.unwrap().starts_with('/'));

    // The session really depends on the jump connection: cutting it kills the session.
    proxy.sever();
    tokio::time::timeout(Duration::from_secs(10), session.closed())
        .await
        .expect("session should end");
    assert!(session.is_closed());
    let mut saw_terminal = false;
    while let Ok(Some(ev)) = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await {
        saw_terminal |= matches!(ev, ShellEvent::Error(_) | ShellEvent::Closed { .. });
    }
    assert!(saw_terminal);
}

#[tokio::test]
async fn proxy_jump_through_two_hops() {
    let server = server!();
    let mut cfg = server.config(key_auth("ed25519", None));
    cfg.jump = vec![
        hop(&server, "127.0.0.1", server.port),
        hop(&server, "localhost", server.port),
    ];
    let verifier = Verifier::accepting();
    let session = connect(cfg, verifier.clone()).await.unwrap();
    let calls = verifier.calls();
    assert_eq!(
        calls.iter().map(|c| c.0.as_str()).collect::<Vec<_>>(),
        ["127.0.0.1", "localhost", "127.0.0.1"]
    );
    assert_eq!(session.exec("echo deep", None).await.unwrap().1, b"deep\n");
    session.disconnect().await;
    assert!(session.is_closed());
}

#[tokio::test]
async fn proxy_jump_failures_name_the_failing_hop() {
    let server = server!();

    // Jump host auth fails.
    let mut cfg = server.config(key_auth("ed25519", None));
    let stranger = hatoba_ssh::generate_key(hatoba_ssh::GenerateKind::Ed25519, "x", None).unwrap();
    cfg.jump = vec![JumpHop {
        auth: AuthMethod::PrivateKey {
            openssh: stranger.openssh_private.clone(),
            passphrase: None,
        },
        ..hop(&server, "127.0.0.1", server.port)
    }];
    let err = connect(cfg, Verifier::accepting()).await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("hop 1/2"), "{err}");

    // The jump host cannot reach the target.
    let dead = free_port();
    let mut cfg = ConnectConfig::new(
        "127.0.0.1",
        dead,
        server.key_user.clone(),
        key_auth("ed25519", None),
    );
    cfg.jump = vec![hop(&server, "127.0.0.1", server.port)];
    let err = connect(cfg, Verifier::accepting()).await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Channel, "{err}");
    assert!(err.message.contains("hop 2/2"), "{err}");
    assert!(err.message.contains(&format!("127.0.0.1:{dead}")), "{err}");

    // First hop unreachable keeps its precise kind.
    let mut cfg = server.config(key_auth("ed25519", None));
    cfg.jump = vec![hop(&server, "127.0.0.1", free_port())];
    let err = connect(cfg, Verifier::accepting()).await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Refused, "{err}");
    assert!(err.message.contains("hop 1/2"), "{err}");

    // Rejecting the second hop's host key.
    struct RejectSecond(std::sync::atomic::AtomicUsize);
    #[async_trait::async_trait]
    impl hatoba_ssh::HostKeyVerifier for RejectSecond {
        async fn verify(&self, _: &str, _: u16, _: &hatoba_ssh::HostKeyInfo) -> bool {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0
        }
    }
    let mut cfg = server.config(key_auth("ed25519", None));
    cfg.jump = vec![hop(&server, "127.0.0.1", server.port)];
    let err = connect(cfg, Arc::new(RejectSecond(Default::default())))
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::HostKeyRejected, "{err}");
    assert!(err.message.contains("hop 2/2"), "{err}");
}

#[tokio::test]
async fn password_jump_host_and_key_target() {
    let server = server!();
    let Some((user, password)) = server.password_user else {
        eprintln!("password user unavailable: skipping");
        return;
    };
    let mut cfg = server.config(key_auth("ed25519", None));
    cfg.jump = vec![JumpHop {
        host: "127.0.0.1".into(),
        port: server.port,
        username: user.into(),
        auth: AuthMethod::Password(Zeroizing::new(password.into())),
        agent_sockets: Vec::new(),
    }];
    let session = connect(cfg, Verifier::accepting()).await.unwrap();
    assert_eq!(
        session.exec("id -un", None).await.unwrap().1,
        format!("{}\n", server.key_user).as_bytes()
    );
}
