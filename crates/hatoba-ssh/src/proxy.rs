//! SOCKS5 (RFC 1928, RFC 1929) and HTTP CONNECT proxies for the TCP connection of the first
//! hop (SSH-13).
//!
//! The server's name goes to the proxy as it is, so the proxy resolves it: nothing looks the
//! server up on this device. Only the proxy's own address is resolved here. The SSH session
//! inside the tunnel is end-to-end encrypted and its host key is checked as usual; the proxy
//! sees which server is reached, and the proxy's own username and password cross the network
//! as the protocols define them (in the clear for both).

use std::fmt;
use std::net::IpAddr;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::Instant;
use zeroize::Zeroizing;

use crate::error::{SshError, SshErrorKind};
use crate::net::{ConnectClock, Phase, connect_first, resolve};

/// The longest HTTP response head read from a proxy.
const MAX_HTTP_HEAD: usize = 16 * 1024;

/// The protocol a [`ProxyConfig`] speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyKind {
    /// SOCKS5, with optional username and password authentication.
    Socks5,
    /// An HTTP proxy that supports `CONNECT`, with optional Basic authentication.
    Http,
}

impl ProxyKind {
    fn label(self) -> &'static str {
        match self {
            Self::Socks5 => "SOCKS5",
            Self::Http => "HTTP",
        }
    }
}

/// A proxy that the first hop's TCP connection goes through.
#[derive(Clone)]
pub struct ProxyConfig {
    /// Protocol.
    pub kind: ProxyKind,
    /// The proxy's address (DNS name or IP).
    pub host: String,
    /// The proxy's port.
    pub port: u16,
    /// Login name on the proxy; empty when it needs none.
    pub username: String,
    /// Password on the proxy, used only with a username.
    pub password: Zeroizing<String>,
}

impl ProxyConfig {
    /// A proxy without authentication.
    pub fn new(kind: ProxyKind, host: impl Into<String>, port: u16) -> Self {
        Self {
            kind,
            host: host.into(),
            port,
            username: String::new(),
            password: Zeroizing::new(String::new()),
        }
    }

    fn label(&self) -> String {
        authority(&self.host, self.port)
    }
}

impl fmt::Debug for ProxyConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyConfig")
            .field("kind", &self.kind)
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Connects to `proxy` and asks it for a connection to `host:port`. Returns the stream, ready
/// for the SSH handshake, and the time the proxy took to answer the request.
pub(crate) async fn open_tunnel(
    proxy: &ProxyConfig,
    host: &str,
    port: u16,
    clock: &ConnectClock,
) -> Result<(TcpStream, Duration), SshError> {
    clock.set_phase(Phase::Proxy);
    let mut stream = reach(proxy).await?;
    clock.set_phase(Phase::ProxyTunnel);
    let started = Instant::now();
    handshake(&mut stream, proxy, host, port).await?;
    Ok((stream, started.elapsed()))
}

/// Opens the TCP connection to the proxy. Every failure is [`SshErrorKind::ProxyUnreachable`],
/// so it is never mistaken for the server refusing the connection.
pub(crate) async fn reach(proxy: &ProxyConfig) -> Result<TcpStream, SshError> {
    let unreachable = |e: SshError| {
        SshError::new(
            SshErrorKind::ProxyUnreachable,
            format!("cannot reach the proxy {}: {}", proxy.label(), e.message),
        )
    };
    let addrs = resolve(&proxy.host, proxy.port)
        .await
        .map_err(unreachable)?;
    let (stream, _) = connect_first(&addrs).await.map_err(unreachable)?;
    Ok(stream)
}

/// Runs the proxy protocol on an open connection to the proxy.
pub(crate) async fn handshake<S>(
    stream: &mut S,
    proxy: &ProxyConfig,
    host: &str,
    port: u16,
) -> Result<(), SshError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let result = match proxy.kind {
        ProxyKind::Socks5 => socks5(stream, proxy, host, port).await,
        ProxyKind::Http => http_connect(stream, proxy, host, port).await,
    };
    result.map_err(|e| match e {
        Failure::Io(io) => {
            let detail = match io.kind() {
                std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe => format!(
                    "the proxy {} closed the connection during the {} handshake; check that it is a {} proxy",
                    proxy.label(),
                    proxy.kind.label(),
                    proxy.kind.label()
                ),
                _ => format!("talking to the proxy {}: {io}", proxy.label()),
            };
            SshError::new(SshErrorKind::Proxy, detail)
        }
        Failure::Ssh(e) => e,
    })
}

enum Failure {
    Io(std::io::Error),
    Ssh(SshError),
}

impl From<std::io::Error> for Failure {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<SshError> for Failure {
    fn from(e: SshError) -> Self {
        Self::Ssh(e)
    }
}

fn proxy_error(message: impl Into<String>) -> Failure {
    Failure::Ssh(SshError::new(SshErrorKind::Proxy, message))
}

fn auth_error(message: impl Into<String>) -> Failure {
    Failure::Ssh(SshError::new(SshErrorKind::ProxyAuth, message))
}

/// `host:port`, with an IPv6 address in brackets.
fn authority(host: &str, port: u16) -> String {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if bare.contains(':') {
        format!("[{bare}]:{port}")
    } else {
        format!("{bare}:{port}")
    }
}

/// The target as an IP address when it is one (brackets allowed around IPv6).
fn ip_target(host: &str) -> Option<IpAddr> {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()
}

async fn socks5<S>(
    stream: &mut S,
    proxy: &ProxyConfig,
    host: &str,
    port: u16,
) -> Result<(), Failure>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    const NO_AUTH: u8 = 0x00;
    const USER_PASS: u8 = 0x02;
    const NO_ACCEPTABLE: u8 = 0xff;

    let with_auth = !proxy.username.is_empty();
    let greeting: &[u8] = if with_auth {
        &[5, 2, NO_AUTH, USER_PASS]
    } else {
        &[5, 1, NO_AUTH]
    };
    stream.write_all(greeting).await?;
    let mut choice = [0u8; 2];
    stream.read_exact(&mut choice).await?;
    if choice[0] != 5 {
        return Err(not_a_proxy(proxy));
    }
    match choice[1] {
        NO_AUTH => {}
        USER_PASS if with_auth => {
            let user = proxy.username.as_bytes();
            let pass = proxy.password.as_bytes();
            let (Ok(user_len), Ok(pass_len)) = (u8::try_from(user.len()), u8::try_from(pass.len()))
            else {
                return Err(auth_error(
                    "SOCKS5 usernames and passwords are at most 255 bytes",
                ));
            };
            let mut request = Zeroizing::new(Vec::with_capacity(3 + user.len() + pass.len()));
            request.push(1);
            request.push(user_len);
            request.extend_from_slice(user);
            request.push(pass_len);
            request.extend_from_slice(pass);
            stream.write_all(&request).await?;
            let mut status = [0u8; 2];
            stream.read_exact(&mut status).await?;
            if status[1] != 0 {
                return Err(auth_error(format!(
                    "the proxy {} rejected the username or password",
                    proxy.label()
                )));
            }
        }
        NO_ACCEPTABLE if !with_auth => {
            return Err(auth_error(format!(
                "the proxy {} requires a username and password",
                proxy.label()
            )));
        }
        NO_ACCEPTABLE => {
            return Err(auth_error(format!(
                "the proxy {} accepts none of the offered sign-in methods",
                proxy.label()
            )));
        }
        other => {
            return Err(proxy_error(format!(
                "the proxy {} chose a sign-in method that was not offered ({other:#04x})",
                proxy.label()
            )));
        }
    }

    let mut request = vec![5, 1, 0];
    match ip_target(host) {
        Some(IpAddr::V4(v4)) => {
            request.push(1);
            request.extend_from_slice(&v4.octets());
        }
        Some(IpAddr::V6(v6)) => {
            request.push(4);
            request.extend_from_slice(&v6.octets());
        }
        None => {
            let name = host.as_bytes();
            let Some(len) = u8::try_from(name.len()).ok().filter(|l| *l > 0) else {
                return Err(proxy_error("SOCKS5 host names are 1 to 255 bytes long"));
            };
            request.push(3);
            request.push(len);
            request.extend_from_slice(name);
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream.write_all(&request).await?;

    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await?;
    if head[0] != 5 {
        return Err(not_a_proxy(proxy));
    }
    if head[1] != 0 {
        return Err(Failure::Ssh(socks5_reply_error(
            head[1],
            &authority(host, port),
        )));
    }
    // The address the proxy bound for the connection, which is of no use here.
    let address_len = match head[3] {
        1 => 4,
        4 => 16,
        3 => {
            let mut len = [0u8; 1];
            stream.read_exact(&mut len).await?;
            usize::from(len[0])
        }
        other => {
            return Err(proxy_error(format!(
                "the proxy {} answered with an unknown address type ({other:#04x})",
                proxy.label()
            )));
        }
    };
    let mut bound = vec![0u8; address_len + 2];
    stream.read_exact(&mut bound).await?;
    Ok(())
}

/// The error for a SOCKS5 reply code other than success. Codes that describe the server keep
/// the server's error kinds, so the message names the server rather than the proxy.
fn socks5_reply_error(code: u8, target: &str) -> SshError {
    let (kind, what) = match code {
        0x01 => (SshErrorKind::Proxy, "general failure"),
        0x02 => (SshErrorKind::Proxy, "connection not allowed by its rules"),
        0x03 => (SshErrorKind::Unreachable, "network unreachable"),
        0x04 => (SshErrorKind::Unreachable, "host unreachable"),
        0x05 => (SshErrorKind::Refused, "connection refused"),
        0x06 => (SshErrorKind::Timeout, "TTL expired"),
        0x07 => (SshErrorKind::Proxy, "CONNECT not supported"),
        0x08 => (SshErrorKind::Proxy, "address type not supported"),
        _ => (SshErrorKind::Proxy, "unknown reply"),
    };
    SshError::new(
        kind,
        format!("the proxy could not connect to {target}: {what} ({code:#04x})"),
    )
}

fn not_a_proxy(proxy: &ProxyConfig) -> Failure {
    proxy_error(format!(
        "the proxy {} did not answer as a {} proxy",
        proxy.label(),
        proxy.kind.label()
    ))
}

async fn http_connect<S>(
    stream: &mut S,
    proxy: &ProxyConfig,
    host: &str,
    port: u16,
) -> Result<(), Failure>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    if host.is_empty() || host.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(proxy_error(
            "the server address cannot be sent to an HTTP proxy",
        ));
    }
    let target = authority(host, port);
    let mut request = Zeroizing::new(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n"));
    if !proxy.username.is_empty() {
        if proxy.username.contains(':') {
            return Err(auth_error("HTTP proxy usernames cannot contain a colon"));
        }
        let credentials = Zeroizing::new(format!("{}:{}", proxy.username, *proxy.password));
        let encoded = Zeroizing::new(BASE64.encode(credentials.as_bytes()));
        request.push_str("Proxy-Authorization: Basic ");
        request.push_str(&encoded);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await?;

    let head = read_head(stream, proxy).await?;
    let status = parse_status(&head).ok_or_else(|| not_a_proxy(proxy))?;
    match status {
        200..=299 => Ok(()),
        407 if proxy.username.is_empty() => Err(auth_error(format!(
            "the proxy {} requires a username and password (HTTP 407)",
            proxy.label()
        ))),
        407 => Err(auth_error(format!(
            "the proxy {} rejected the username or password (HTTP 407)",
            proxy.label()
        ))),
        504 => Err(Failure::Ssh(SshError::new(
            SshErrorKind::Timeout,
            format!("the proxy timed out connecting to {target} (HTTP 504)"),
        ))),
        _ => Err(proxy_error(format!(
            "the proxy could not connect to {target} (HTTP {status})"
        ))),
    }
}

/// Reads the response head up to and including the blank line. One byte at a time: whatever
/// follows the head is the server's SSH identification string and must stay in the stream.
async fn read_head<S>(stream: &mut S, proxy: &ProxyConfig) -> Result<Vec<u8>, Failure>
where
    S: AsyncRead + Unpin,
{
    let mut head = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= MAX_HTTP_HEAD {
            return Err(proxy_error(format!(
                "the proxy {} sent a response head longer than {} KB",
                proxy.label(),
                MAX_HTTP_HEAD / 1024
            )));
        }
        stream.read_exact(&mut byte).await?;
        head.push(byte[0]);
    }
    Ok(head)
}

/// The status code of an `HTTP/1.x NNN reason` status line.
fn parse_status(head: &[u8]) -> Option<u16> {
    let line = head.split(|b| *b == b'\n').next()?;
    let line = std::str::from_utf8(line).ok()?.trim_end();
    let mut parts = line.split(' ');
    if !parts.next()?.starts_with("HTTP/1.") {
        return None;
    }
    let code = parts.next()?;
    (code.len() == 3).then(|| code.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    fn socks(username: &str, password: &str) -> ProxyConfig {
        ProxyConfig {
            username: username.to_owned(),
            password: Zeroizing::new(password.to_owned()),
            ..ProxyConfig::new(ProxyKind::Socks5, "127.0.0.1", 1080)
        }
    }

    fn http(username: &str, password: &str) -> ProxyConfig {
        ProxyConfig {
            kind: ProxyKind::Http,
            ..socks(username, password)
        }
    }

    /// Runs the handshake against `server`, which plays the proxy on the other end of a pipe.
    async fn run<F, Fut>(
        proxy: ProxyConfig,
        host: &str,
        port: u16,
        server: F,
    ) -> Result<Vec<u8>, SshError>
    where
        F: FnOnce(tokio::io::DuplexStream) -> Fut,
        Fut: std::future::Future<Output = ()> + Send + 'static,
    {
        let (mut client, far) = duplex(4096);
        let task = tokio::spawn(server(far));
        let result = handshake(&mut client, &proxy, host, port).await;
        // What the proxy left in the stream after the handshake.
        let mut rest = Vec::new();
        if result.is_ok() {
            client.read_to_end(&mut rest).await.unwrap();
        }
        // Unblocks a proxy still writing into a full pipe.
        drop(client);
        task.await.unwrap();
        result.map(|()| rest)
    }

    async fn expect(far: &mut tokio::io::DuplexStream, bytes: &[u8]) {
        let mut got = vec![0u8; bytes.len()];
        far.read_exact(&mut got).await.unwrap();
        assert_eq!(got, bytes);
    }

    #[tokio::test]
    async fn socks5_without_auth_sends_the_name_for_the_proxy_to_resolve() {
        let rest = run(socks("", ""), "example.com", 22, |mut far| async move {
            expect(&mut far, &[5, 1, 0]).await;
            far.write_all(&[5, 0]).await.unwrap();
            let mut want = vec![5, 1, 0, 3, 11];
            want.extend_from_slice(b"example.com");
            want.extend_from_slice(&22u16.to_be_bytes());
            expect(&mut far, &want).await;
            // Bound address as a domain name, then the server's banner right behind it.
            far.write_all(&[5, 0, 0, 3, 3, b'a', b'b', b'c', 0, 80])
                .await
                .unwrap();
            far.write_all(b"SSH-2.0-test\r\n").await.unwrap();
        })
        .await
        .unwrap();
        assert_eq!(rest, b"SSH-2.0-test\r\n");
    }

    #[tokio::test]
    async fn socks5_with_auth_and_ip_targets() {
        run(
            socks("alice", "s3cret"),
            "10.0.0.7",
            2222,
            |mut far| async move {
                expect(&mut far, &[5, 2, 0, 2]).await;
                far.write_all(&[5, 2]).await.unwrap();
                expect(&mut far, b"\x01\x05alice\x06s3cret").await;
                far.write_all(&[1, 0]).await.unwrap();
                expect(&mut far, &[5, 1, 0, 1, 10, 0, 0, 7, 0x08, 0xae]).await;
                far.write_all(&[5, 0, 0, 1, 0, 0, 0, 0, 0, 0])
                    .await
                    .unwrap();
            },
        )
        .await
        .unwrap();

        run(socks("", ""), "[2001:db8::1]", 22, |mut far| async move {
            expect(&mut far, &[5, 1, 0]).await;
            far.write_all(&[5, 0]).await.unwrap();
            let mut want = vec![5, 1, 0, 4];
            want.extend_from_slice(
                &"2001:db8::1"
                    .parse::<std::net::Ipv6Addr>()
                    .unwrap()
                    .octets(),
            );
            want.extend_from_slice(&22u16.to_be_bytes());
            expect(&mut far, &want).await;
            let mut reply = vec![5, 0, 0, 4];
            reply.extend_from_slice(&[0; 18]);
            far.write_all(&reply).await.unwrap();
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn socks5_auth_failures() {
        let err = run(socks("alice", "wrong"), "h", 22, |mut far| async move {
            expect(&mut far, &[5, 2, 0, 2]).await;
            far.write_all(&[5, 2]).await.unwrap();
            let mut buf = [0u8; 13];
            far.read_exact(&mut buf).await.unwrap();
            far.write_all(&[1, 1]).await.unwrap();
        })
        .await
        .unwrap_err();
        assert_eq!(err.kind, SshErrorKind::ProxyAuth);
        assert!(err.message.contains("rejected"), "{err}");
        assert!(
            !err.message.contains("wrong"),
            "no password in messages: {err}"
        );

        let err = run(socks("", ""), "h", 22, |mut far| async move {
            expect(&mut far, &[5, 1, 0]).await;
            far.write_all(&[5, 0xff]).await.unwrap();
        })
        .await
        .unwrap_err();
        assert_eq!(err.kind, SshErrorKind::ProxyAuth);
        assert!(err.message.contains("requires a username"), "{err}");
    }

    #[tokio::test]
    async fn socks5_reply_codes_name_the_server() {
        for (code, kind) in [
            (0x02, SshErrorKind::Proxy),
            (0x04, SshErrorKind::Unreachable),
            (0x05, SshErrorKind::Refused),
            (0x06, SshErrorKind::Timeout),
        ] {
            let err = run(
                socks("", ""),
                "db.internal",
                22,
                move |mut far| async move {
                    expect(&mut far, &[5, 1, 0]).await;
                    far.write_all(&[5, 0]).await.unwrap();
                    let mut buf = vec![0u8; 7 + "db.internal".len()];
                    far.read_exact(&mut buf).await.unwrap();
                    far.write_all(&[5, code, 0, 1, 0, 0, 0, 0, 0, 0])
                        .await
                        .unwrap();
                },
            )
            .await
            .unwrap_err();
            assert_eq!(err.kind, kind, "{code:#04x}: {err}");
            assert!(err.message.contains("db.internal:22"), "{err}");
        }
    }

    #[tokio::test]
    async fn wrong_protocol_or_closed_connection_is_a_proxy_error() {
        // An HTTP proxy configured as SOCKS5.
        let err = run(socks("", ""), "h", 22, |mut far| async move {
            let mut buf = [0u8; 3];
            far.read_exact(&mut buf).await.unwrap();
            far.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n")
                .await
                .unwrap();
        })
        .await
        .unwrap_err();
        assert_eq!(err.kind, SshErrorKind::Proxy);
        assert!(
            err.message.contains("did not answer as a SOCKS5 proxy"),
            "{err}"
        );

        // A SOCKS5 proxy configured as HTTP hangs up on the request.
        let err = run(http("", ""), "h", 22, |far| async move { drop(far) })
            .await
            .unwrap_err();
        assert_eq!(err.kind, SshErrorKind::Proxy);
        assert!(err.message.contains("closed the connection"), "{err}");
    }

    #[tokio::test]
    async fn http_connect_keeps_what_follows_the_head() {
        let rest = run(http("", ""), "example.com", 22, |mut far| async move {
            expect(
                &mut far,
                b"CONNECT example.com:22 HTTP/1.1\r\nHost: example.com:22\r\n\r\n",
            )
            .await;
            far.write_all(b"HTTP/1.1 200 Connection established\r\nVia: x\r\n\r\nSSH-2.0-x\r\n")
                .await
                .unwrap();
        })
        .await
        .unwrap();
        assert_eq!(rest, b"SSH-2.0-x\r\n");
    }

    #[tokio::test]
    async fn http_connect_with_basic_auth_and_ipv6() {
        run(http("alice", "pa:ss"), "::1", 2222, |mut far| async move {
            let want = format!(
                "CONNECT [::1]:2222 HTTP/1.1\r\nHost: [::1]:2222\r\nProxy-Authorization: Basic {}\r\n\r\n",
                BASE64.encode("alice:pa:ss")
            );
            expect(&mut far, want.as_bytes()).await;
            far.write_all(b"HTTP/1.0 200 OK\r\n\r\n").await.unwrap();
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn http_connect_status_codes() {
        for (status, kind, needle) in [
            (
                "407 Proxy Authentication Required",
                SshErrorKind::ProxyAuth,
                "requires a username",
            ),
            ("403 Forbidden", SshErrorKind::Proxy, "HTTP 403"),
            ("502 Bad Gateway", SshErrorKind::Proxy, "HTTP 502"),
            ("504 Gateway Timeout", SshErrorKind::Timeout, "HTTP 504"),
        ] {
            let err = run(http("", ""), "h", 22, move |mut far| async move {
                let mut buf = vec![0u8; "CONNECT h:22 HTTP/1.1\r\nHost: h:22\r\n\r\n".len()];
                far.read_exact(&mut buf).await.unwrap();
                far.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\n\r\n").as_bytes())
                    .await
                    .unwrap();
            })
            .await
            .unwrap_err();
            assert_eq!(err.kind, kind, "{status}: {err}");
            assert!(err.message.contains(needle), "{err}");
        }
    }

    #[tokio::test]
    async fn http_connect_rejects_bad_input_and_endless_heads() {
        let err = run(http("", ""), "h\r\nX: y", 22, |_| async {})
            .await
            .unwrap_err();
        assert_eq!(err.kind, SshErrorKind::Proxy);

        let err = run(http("a:b", ""), "h", 22, |_| async {})
            .await
            .unwrap_err();
        assert_eq!(err.kind, SshErrorKind::ProxyAuth);

        let err = run(http("", ""), "h", 22, |mut far| async move {
            let mut buf = vec![0u8; "CONNECT h:22 HTTP/1.1\r\nHost: h:22\r\n\r\n".len()];
            far.read_exact(&mut buf).await.unwrap();
            far.write_all(b"HTTP/1.1 200 OK\r\n").await.unwrap();
            let filler = vec![b'x'; 1024];
            // The client stops reading once the head is too long, so writes may fail.
            for _ in 0..20 {
                if far.write_all(&filler).await.is_err() {
                    break;
                }
            }
        })
        .await
        .unwrap_err();
        assert_eq!(err.kind, SshErrorKind::Proxy);
        assert!(err.message.contains("longer than"), "{err}");
    }

    #[test]
    fn status_lines() {
        assert_eq!(parse_status(b"HTTP/1.1 200 OK\r\n\r\n"), Some(200));
        assert_eq!(parse_status(b"HTTP/1.0 407\r\n\r\n"), Some(407));
        assert_eq!(parse_status(b"SSH-2.0-OpenSSH\r\n\r\n"), None);
        assert_eq!(parse_status(b"HTTP/1.1 20 OK\r\n\r\n"), None);
    }

    #[test]
    fn authorities() {
        assert_eq!(authority("example.com", 22), "example.com:22");
        assert_eq!(authority("::1", 22), "[::1]:22");
        assert_eq!(authority("[::1]", 22), "[::1]:22");
    }

    #[test]
    fn debug_redacts_the_password() {
        let text = format!("{:?}", socks("alice", "s3cret"));
        assert!(text.contains("alice") && !text.contains("s3cret"), "{text}");
    }

    #[tokio::test]
    async fn unreachable_proxy_is_its_own_kind() {
        // A port that was free a moment ago: the connection is refused.
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let proxy = ProxyConfig::new(ProxyKind::Socks5, "127.0.0.1", port);
        let err = open_tunnel(&proxy, "example.com", 22, &ConnectClock::new())
            .await
            .unwrap_err();
        assert_eq!(err.kind, SshErrorKind::ProxyUnreachable, "{err}");
        assert!(err.message.contains(&format!("127.0.0.1:{port}")), "{err}");
    }
}
