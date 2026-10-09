//! Connections through SOCKS5 and HTTP CONNECT proxies (SSH-13), against the scripted
//! in-process SSH server and small proxies written here. No sshd needed.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use common::ask_server::*;
use common::*;
use hatoba_ssh::{
    AuthMethod, ConnectConfig, ProxyConfig, ProxyKind, SshErrorKind, connect, tcp_probe_via,
};
use russh::MethodKind;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use zeroize::Zeroizing;

/// The only name the test proxies can reach. It does not resolve anywhere, so a connection
/// that works proves the name went to the proxy rather than to this machine's resolver.
const SERVER_NAME: &str = "ssh.hatoba.test";

struct TestProxy {
    port: u16,
    /// Targets asked for, as `host:port`.
    targets: Arc<Mutex<Vec<String>>>,
}

#[derive(Clone, Copy)]
struct Behaviour {
    kind: ProxyKind,
    /// Username and password the proxy wants.
    credentials: Option<(&'static str, &'static str)>,
    /// Confirm before connecting, like proxies that only then try the server.
    confirm_first: bool,
}

impl TestProxy {
    fn targets(&self) -> Vec<String> {
        self.targets.lock().unwrap().clone()
    }

    fn config(&self, kind: ProxyKind, credentials: Option<(&str, &str)>) -> ProxyConfig {
        let mut proxy = ProxyConfig::new(kind, "127.0.0.1", self.port);
        if let Some((user, pass)) = credentials {
            proxy.username = user.to_owned();
            proxy.password = Zeroizing::new(pass.to_owned());
        }
        proxy
    }
}

/// A proxy that maps [`SERVER_NAME`] to `127.0.0.1:upstream` and nothing else.
async fn start_proxy(behaviour: Behaviour, upstream: Option<u16>) -> TestProxy {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let targets = Arc::new(Mutex::new(Vec::new()));
    let seen = targets.clone();
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let seen = seen.clone();
            tokio::spawn(async move {
                let _ = match behaviour.kind {
                    ProxyKind::Socks5 => serve_socks5(client, behaviour, upstream, seen).await,
                    ProxyKind::Http => serve_http(client, behaviour, upstream, seen).await,
                };
            });
        }
    });
    TestProxy { port, targets }
}

async fn dial(host: &str, port: u16, upstream: Option<u16>) -> Option<TcpStream> {
    if host != SERVER_NAME || Some(port) != upstream {
        return None;
    }
    TcpStream::connect(("127.0.0.1", port)).await.ok()
}

async fn serve_socks5(
    mut client: TcpStream,
    behaviour: Behaviour,
    upstream: Option<u16>,
    seen: Arc<Mutex<Vec<String>>>,
) -> std::io::Result<()> {
    let mut head = [0u8; 2];
    client.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Ok(());
    }
    let mut methods = vec![0u8; usize::from(head[1])];
    client.read_exact(&mut methods).await?;
    if let Some((user, pass)) = behaviour.credentials {
        if !methods.contains(&2) {
            return client.write_all(&[5, 0xff]).await;
        }
        client.write_all(&[5, 2]).await?;
        let mut ver_len = [0u8; 2];
        client.read_exact(&mut ver_len).await?;
        let mut got_user = vec![0u8; usize::from(ver_len[1])];
        client.read_exact(&mut got_user).await?;
        let mut pass_len = [0u8; 1];
        client.read_exact(&mut pass_len).await?;
        let mut got_pass = vec![0u8; usize::from(pass_len[0])];
        client.read_exact(&mut got_pass).await?;
        let ok = got_user == user.as_bytes() && got_pass == pass.as_bytes();
        client.write_all(&[1, u8::from(!ok)]).await?;
        if !ok {
            return Ok(());
        }
    } else {
        client.write_all(&[5, 0]).await?;
    }
    let mut request = [0u8; 4];
    client.read_exact(&mut request).await?;
    let host = match request[3] {
        3 => {
            let mut len = [0u8; 1];
            client.read_exact(&mut len).await?;
            let mut name = vec![0u8; usize::from(len[0])];
            client.read_exact(&mut name).await?;
            String::from_utf8(name).unwrap()
        }
        1 => {
            let mut ip = [0u8; 4];
            client.read_exact(&mut ip).await?;
            std::net::Ipv4Addr::from(ip).to_string()
        }
        _ => return Ok(()),
    };
    let mut port = [0u8; 2];
    client.read_exact(&mut port).await?;
    let port = u16::from_be_bytes(port);
    seen.lock().unwrap().push(format!("{host}:{port}"));
    let success = [5, 0, 0, 1, 127, 0, 0, 1, 0, 0];
    if behaviour.confirm_first {
        client.write_all(&success).await?;
    }
    match dial(&host, port, upstream).await {
        Some(mut server) => {
            if !behaviour.confirm_first {
                client.write_all(&success).await?;
            }
            tokio::io::copy_bidirectional(&mut client, &mut server).await?;
        }
        None if !behaviour.confirm_first => {
            client.write_all(&[5, 4, 0, 1, 0, 0, 0, 0, 0, 0]).await?;
        }
        None => {}
    }
    Ok(())
}

async fn serve_http(
    mut client: TcpStream,
    behaviour: Behaviour,
    upstream: Option<u16>,
    seen: Arc<Mutex<Vec<String>>>,
) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        client.read_exact(&mut byte).await?;
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    let target = head
        .strip_prefix("CONNECT ")
        .and_then(|rest| rest.split(' ').next())
        .unwrap_or_default()
        .to_owned();
    seen.lock().unwrap().push(target.clone());
    if let Some((user, pass)) = behaviour.credentials {
        let want = format!(
            "Proxy-Authorization: Basic {}\r\n",
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))
        );
        if !head.contains(&want) {
            return client
                .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nContent-Length: 0\r\n\r\n")
                .await;
        }
    }
    let (host, port) = target.rsplit_once(':').unwrap_or_default();
    match dial(host, port.parse().unwrap_or(0), upstream).await {
        Some(mut server) => {
            client
                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                .await?;
            tokio::io::copy_bidirectional(&mut client, &mut server).await?;
        }
        None => {
            client
                .write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\n\r\n")
                .await?;
        }
    }
    Ok(())
}

async fn server() -> AskServer {
    start_ask_server(AskScript::offering(&[MethodKind::Password])).await
}

fn config(port: u16, proxy: ProxyConfig) -> ConnectConfig {
    let mut cfg = ConnectConfig::new(
        SERVER_NAME,
        port,
        "alice",
        AuthMethod::Password(Zeroizing::new(ASK_PASSWORD.to_owned())),
    );
    cfg.proxy = Some(proxy);
    cfg.connect_timeout = Duration::from_secs(10);
    cfg
}

fn socks(credentials: Option<(&'static str, &'static str)>) -> Behaviour {
    Behaviour {
        kind: ProxyKind::Socks5,
        credentials,
        confirm_first: false,
    }
}

fn http(credentials: Option<(&'static str, &'static str)>) -> Behaviour {
    Behaviour {
        kind: ProxyKind::Http,
        ..socks(credentials)
    }
}

#[tokio::test]
async fn socks5_carries_the_session_and_resolves_the_name() {
    let server = server().await;
    let proxy = start_proxy(socks(None), Some(server.port)).await;
    let verifier = Verifier::accepting();
    let session = connect(config(server.port, proxy.config(ProxyKind::Socks5, None)), verifier.clone())
        .await
        .expect("connected through the proxy");
    assert_eq!(proxy.targets(), vec![format!("{SERVER_NAME}:{}", server.port)]);
    // The host key is checked for the server, not the proxy.
    let calls = verifier.calls();
    assert_eq!((calls[0].0.as_str(), calls[0].1), (SERVER_NAME, server.port));
    assert!(session.latency_ms() >= 1);
    session.disconnect().await;
}

#[tokio::test]
async fn proxy_credentials() {
    let server = server().await;
    let creds = Some(("proxyuser", "proxy-pass"));
    for behaviour in [socks(creds), http(creds)] {
        let proxy = start_proxy(behaviour, Some(server.port)).await;
        let session = connect(
            config(server.port, proxy.config(behaviour.kind, creds)),
            Verifier::accepting(),
        )
        .await
        .expect("right proxy credentials");
        session.disconnect().await;

        let err = connect(
            config(server.port, proxy.config(behaviour.kind, Some(("proxyuser", "wrong")))),
            Verifier::accepting(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind, SshErrorKind::ProxyAuth, "{err}");
        assert!(!err.message.contains("wrong"), "{err}");

        let err = connect(
            config(server.port, proxy.config(behaviour.kind, None)),
            Verifier::accepting(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind, SshErrorKind::ProxyAuth, "{err}");
    }
    // The SSH password never went anywhere but the server.
    assert!(server.received().iter().all(|p| p == ASK_PASSWORD));
}

#[tokio::test]
async fn http_connect_carries_the_session() {
    let server = server().await;
    let proxy = start_proxy(http(None), Some(server.port)).await;
    let session = connect(
        config(server.port, proxy.config(ProxyKind::Http, None)),
        Verifier::accepting(),
    )
    .await
    .expect("connected through the HTTP proxy");
    assert_eq!(proxy.targets(), vec![format!("{SERVER_NAME}:{}", server.port)]);
    session.disconnect().await;
}

#[tokio::test]
async fn unreachable_servers_and_proxies() {
    let server = server().await;
    let wrong_port = free_port();

    // The proxy says the server cannot be reached.
    let proxy = start_proxy(socks(None), Some(server.port)).await;
    let err = connect(
        config(wrong_port, proxy.config(ProxyKind::Socks5, None)),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Unreachable, "{err}");

    let proxy = start_proxy(http(None), Some(server.port)).await;
    let err = connect(
        config(wrong_port, proxy.config(ProxyKind::Http, None)),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Proxy, "{err}");
    assert!(err.message.contains("HTTP 502"), "{err}");

    // A proxy that confirms first, then hangs up because the server is not there.
    let proxy = start_proxy(
        Behaviour {
            confirm_first: true,
            ..socks(None)
        },
        Some(server.port),
    )
    .await;
    let err = connect(
        config(wrong_port, proxy.config(ProxyKind::Socks5, None)),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Proxy, "{err}");
    assert!(err.message.contains("closed before the SSH handshake"), "{err}");

    // No proxy listening.
    let nothing = ProxyConfig::new(ProxyKind::Socks5, "127.0.0.1", free_port());
    let err = connect(config(server.port, nothing), Verifier::accepting())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::ProxyUnreachable, "{err}");

    // The wrong protocol for the proxy.
    let proxy = start_proxy(socks(None), Some(server.port)).await;
    let err = connect(
        config(server.port, proxy.config(ProxyKind::Http, None)),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Proxy, "{err}");
}

#[tokio::test]
async fn probes_go_through_the_proxy_and_wait_for_the_server() {
    let server = server().await;
    let timeout = Duration::from_secs(3);
    let proxy = start_proxy(socks(None), Some(server.port)).await;
    let cfg = proxy.config(ProxyKind::Socks5, None);
    assert!(tcp_probe_via(Some(&cfg), SERVER_NAME, server.port, timeout).await.is_some());
    // Without the proxy the name does not resolve.
    assert!(tcp_probe_via(None, SERVER_NAME, server.port, timeout).await.is_none());

    // A proxy that confirms at once does not make a missing server look online.
    let eager = start_proxy(
        Behaviour {
            confirm_first: true,
            ..socks(None)
        },
        Some(server.port),
    )
    .await;
    let cfg = eager.config(ProxyKind::Socks5, None);
    assert!(tcp_probe_via(Some(&cfg), SERVER_NAME, free_port(), timeout).await.is_none());
    assert!(tcp_probe_via(Some(&cfg), SERVER_NAME, server.port, timeout).await.is_some());
}
