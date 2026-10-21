//! Port forwarding (local, remote and SOCKS) and ProxyJump integration tests (set
//! `HATOBA_SSH_IT=1`). The server runs on this machine, so a remote forward's port is reachable
//! here too.
#![cfg(unix)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use hatoba_ssh::{
    AuthMethod, ConnectConfig, DynamicForward, JumpHop, LocalForward, ProxyConfig, ProxyKind,
    RemoteForward, ShellEvent, ShellOptions, SshErrorKind, connect,
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
    assert_ne!(fwd.port(), 0);
    assert!(fwd.is_running());

    let mut conn = TcpStream::connect(("127.0.0.1", fwd.port())).await.unwrap();
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
    let port = fwd.port();

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
    let port = fwd.port();

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
    let mut conn = TcpStream::connect(("127.0.0.1", fwd.port())).await.unwrap();
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
            bind_port: first.port(),
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
    assert_eq!(second.port(), port);
    assert!(second.local_addr().unwrap().ip().is_loopback());
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

// --------------------------------------------------------------------------- remote forward

fn remote_to(port: u16) -> RemoteForward {
    RemoteForward {
        bind_address: "127.0.0.1".into(),
        bind_port: 0,
        dest_host: "127.0.0.1".into(),
        dest_port: port,
    }
}

/// A server that sends `tag` to each connection and closes it.
async fn tag_server(tag: &'static [u8]) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let _ = stream.write_all(tag).await;
        }
    });
    port
}

async fn read_all(port: u16) -> Vec<u8> {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let mut got = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), conn.read_to_end(&mut got))
        .await
        .expect("read")
        .unwrap();
    got
}

async fn echo_round_trip(port: u16, data: &[u8]) {
    let mut conn = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    conn.write_all(data).await.unwrap();
    let mut got = vec![0u8; data.len()];
    tokio::time::timeout(Duration::from_secs(10), conn.read_exact(&mut got))
        .await
        .expect("echo")
        .unwrap();
    assert_eq!(got, data);
}

/// Waits until nothing listens on the port any more.
async fn until_closed(port: u16) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the port should close");
}

async fn until_stopped(fwd: &hatoba_ssh::ForwardHandle) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while fwd.is_running() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("forward should stop with its session");
}

#[tokio::test]
async fn remote_forward_reaches_a_server_here() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let echo = echo_server().await;
    let fwd = session.remote_forward(remote_to(echo)).await.unwrap();
    assert_ne!(fwd.port(), 0, "the server names the port it picked");
    assert!(fwd.local_addr().is_none());
    assert!(fwd.is_running());

    let port = fwd.port();
    let tasks: Vec<_> = (0..4u64)
        .map(|i| {
            tokio::spawn(async move {
                echo_round_trip(port, &pseudo_random(256 * 1024 + 3, 200 + i)).await;
            })
        })
        .collect();
    for t in tasks {
        t.await.unwrap();
    }
    fwd.stop();
}

#[tokio::test]
async fn remote_forward_on_a_fixed_port_and_the_default_address() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let echo = echo_server().await;
    let port = free_port();
    let fwd = session
        .remote_forward(RemoteForward {
            bind_address: String::new(),
            bind_port: port,
            ..remote_to(echo)
        })
        .await
        .unwrap();
    assert_eq!(fwd.port(), port);
    echo_round_trip(port, b"localhost").await;
    fwd.stop();
}

#[tokio::test]
async fn remote_forwards_route_to_their_own_destinations() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let a = session
        .remote_forward(remote_to(tag_server(b"first").await))
        .await
        .unwrap();
    let b = session
        .remote_forward(remote_to(tag_server(b"second").await))
        .await
        .unwrap();
    assert_eq!(read_all(a.port()).await, b"first");
    assert_eq!(read_all(b.port()).await, b"second");
    a.stop();
    until_closed(a.port()).await;
    assert_eq!(
        read_all(b.port()).await,
        b"second",
        "the other keeps running"
    );
    b.stop();
}

#[tokio::test]
async fn stopping_a_remote_forward_stops_the_server_listening() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let echo = echo_server().await;
    let fwd = session.remote_forward(remote_to(echo)).await.unwrap();
    let port = fwd.port();

    let mut conn = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    conn.write_all(b"ping").await.unwrap();
    let mut buf = [0u8; 4];
    conn.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");

    fwd.stop();
    assert!(!fwd.is_running());
    let n = tokio::time::timeout(Duration::from_secs(5), conn.read(&mut buf))
        .await
        .expect("closed")
        .unwrap_or(0);
    assert_eq!(n, 0, "the established connection ends");
    until_closed(port).await;
    // The port is free on the server again, and the session is unaffected.
    let again = session
        .remote_forward(RemoteForward {
            bind_port: port,
            ..remote_to(echo)
        })
        .await
        .unwrap();
    echo_round_trip(port, b"again").await;
    again.stop();
    assert_eq!(session.exec("echo fine", None).await.unwrap().1, b"fine\n");
}

#[tokio::test]
async fn remote_forward_to_a_closed_port_drops_the_connection() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let fwd = session
        .remote_forward(remote_to(free_port()))
        .await
        .unwrap();
    assert!(read_all(fwd.port()).await.is_empty());
    assert!(fwd.is_running(), "the next connection is tried again");
    fwd.stop();
}

#[tokio::test]
async fn remote_forward_on_a_taken_port_fails() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let taken = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let err = session
        .remote_forward(RemoteForward {
            bind_port: taken.local_addr().unwrap().port(),
            ..remote_to(server.port)
        })
        .await
        .err()
        .expect("the port is in use on the server");
    assert_eq!(err.kind, SshErrorKind::Channel, "{err}");
    assert!(err.message.contains("did not listen"), "{err}");
    // The session is unaffected.
    assert_eq!(session.exec("echo fine", None).await.unwrap().1, b"fine\n");
}

#[tokio::test]
async fn remote_forward_stops_when_the_session_ends() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let fwd = session
        .remote_forward(remote_to(server.port))
        .await
        .unwrap();
    session.disconnect().await;
    until_stopped(&fwd).await;
    until_closed(fwd.port()).await;
}

// --------------------------------------------------------------------------- dynamic forward

fn socks_on_loopback() -> DynamicForward {
    DynamicForward {
        bind_address: "127.0.0.1".into(),
        bind_port: 0,
    }
}

/// Greets the SOCKS proxy with `greeting` (SOCKS 5 only), sends `request` and returns the
/// connection and the reply.
async fn socks(
    proxy: u16,
    greeting: &[u8],
    request: &[u8],
    reply_len: usize,
) -> (TcpStream, Vec<u8>) {
    let mut conn = TcpStream::connect(("127.0.0.1", proxy)).await.unwrap();
    if !greeting.is_empty() {
        conn.write_all(greeting).await.unwrap();
        let mut method = [0u8; 2];
        conn.read_exact(&mut method).await.unwrap();
        assert_eq!(method, [5, 0]);
    }
    conn.write_all(request).await.unwrap();
    let mut reply = vec![0u8; reply_len];
    tokio::time::timeout(Duration::from_secs(10), conn.read_exact(&mut reply))
        .await
        .expect("SOCKS reply")
        .unwrap();
    (conn, reply)
}

fn socks5_connect(host: &str, port: u16) -> Vec<u8> {
    let mut request = vec![5, 1, 0];
    match host.parse::<std::net::Ipv4Addr>() {
        Ok(ip) => {
            request.push(1);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            request.push(3);
            request.push(u8::try_from(host.len()).unwrap());
            request.extend_from_slice(host.as_bytes());
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    request
}

/// SOCKS 4 to 127.0.0.1, or SOCKS 4a to `host`.
fn socks4_connect(port: u16, host: Option<&str>) -> Vec<u8> {
    let mut request = vec![4, 1];
    request.extend_from_slice(&port.to_be_bytes());
    request.extend_from_slice(if host.is_some() {
        &[0, 0, 0, 1]
    } else {
        &[127, 0, 0, 1]
    });
    request.extend_from_slice(b"hatoba\0");
    if let Some(host) = host {
        request.extend_from_slice(host.as_bytes());
        request.push(0);
    }
    request
}

async fn assert_echoes(mut conn: TcpStream, data: &[u8]) {
    conn.write_all(data).await.unwrap();
    let mut got = vec![0u8; data.len()];
    tokio::time::timeout(Duration::from_secs(10), conn.read_exact(&mut got))
        .await
        .expect("echo")
        .unwrap();
    assert_eq!(got, data);
}

#[tokio::test]
async fn socks_proxy_connects_in_each_version() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let echo = echo_server().await;
    let fwd = session.dynamic_forward(socks_on_loopback()).await.unwrap();
    assert_ne!(fwd.port(), 0);
    assert!(fwd.local_addr().unwrap().ip().is_loopback());
    let proxy = fwd.port();

    let (conn, reply) = socks(proxy, &[5, 1, 0], &socks5_connect("127.0.0.1", echo), 10).await;
    assert_eq!(reply[..2], [5, 0]);
    assert_echoes(conn, &pseudo_random(512 * 1024, 7)).await;

    // The server resolves the name.
    let (conn, reply) = socks(proxy, &[5, 1, 0], &socks5_connect("localhost", echo), 10).await;
    assert_eq!(reply[..2], [5, 0]);
    assert_echoes(conn, b"by name").await;

    let (conn, reply) = socks(proxy, &[], &socks4_connect(echo, None), 8).await;
    assert_eq!(reply[..2], [0, 90]);
    assert_echoes(conn, b"socks4").await;

    let (conn, reply) = socks(proxy, &[], &socks4_connect(echo, Some("localhost")), 8).await;
    assert_eq!(reply[..2], [0, 90]);
    assert_echoes(conn, b"socks4a").await;
    fwd.stop();
}

#[tokio::test]
async fn socks_proxy_reports_a_closed_destination() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let fwd = session.dynamic_forward(socks_on_loopback()).await.unwrap();
    let closed = free_port();

    let (mut conn, reply) = socks(
        fwd.port(),
        &[5, 1, 0],
        &socks5_connect("127.0.0.1", closed),
        10,
    )
    .await;
    assert_eq!(reply[..2], [5, 5], "connection refused");
    let mut rest = Vec::new();
    conn.read_to_end(&mut rest).await.unwrap();
    assert!(rest.is_empty());

    let (_, reply) = socks(fwd.port(), &[], &socks4_connect(closed, None), 8).await;
    assert_eq!(reply[..2], [0, 91]);
    assert!(fwd.is_running());
    fwd.stop();
}

#[tokio::test]
async fn ssh_through_the_socks_proxy() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let fwd = session.dynamic_forward(socks_on_loopback()).await.unwrap();
    let mut cfg = server.config(key_auth("ed25519", None));
    cfg.proxy = Some(ProxyConfig::new(ProxyKind::Socks5, "127.0.0.1", fwd.port()));
    let inner = connect(cfg, Verifier::accepting()).await.unwrap();
    assert_eq!(inner.exec("echo inner", None).await.unwrap().1, b"inner\n");
    inner.disconnect().await;
    fwd.stop();
}

#[tokio::test]
async fn socks_proxy_stops_with_stop_and_with_the_session() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let fwd = session.dynamic_forward(socks_on_loopback()).await.unwrap();
    fwd.stop();
    until_closed(fwd.port()).await;

    let fwd = session.dynamic_forward(socks_on_loopback()).await.unwrap();
    session.disconnect().await;
    until_stopped(&fwd).await;
    until_closed(fwd.port()).await;
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
