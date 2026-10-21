//! The server side of SOCKS for dynamic forwards (FWD-04): SOCKS 5 CONNECT without
//! authentication, and SOCKS 4 and 4a CONNECT, as `ssh -D` offers. The client side, for reaching
//! the first hop through a proxy, is in [`crate::proxy`].

use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Longest SOCKS 4 user id or 4a host name read.
const MAX_NAME: usize = 255;

/// A SOCKS client's CONNECT request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Request {
    /// An IP address or a host name, which the SSH server resolves.
    pub(crate) host: String,
    pub(crate) port: u16,
    version: Version,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Version {
    V4,
    V5,
}

/// Why a request was not served, as the reply tells a SOCKS 5 client. SOCKS 4 has one code for
/// every failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Failure {
    General,
    NotAllowed,
    Refused,
    CommandNotSupported,
    AddressNotSupported,
}

impl Failure {
    fn code(self) -> u8 {
        match self {
            Self::General => 1,
            Self::NotAllowed => 2,
            Self::Refused => 5,
            Self::CommandNotSupported => 7,
            Self::AddressNotSupported => 8,
        }
    }
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Reads the client's greeting and request. When this returns an error the client has been told
/// so where the protocol has a reply for it, and the connection is to be closed.
pub(crate) async fn accept<S>(stream: &mut S) -> io::Result<Request>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    match stream.read_u8().await? {
        4 => accept_v4(stream).await,
        5 => accept_v5(stream).await,
        v => Err(invalid(format!("not a SOCKS request (version byte {v})"))),
    }
}

async fn accept_v4<S>(stream: &mut S) -> io::Result<Request>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let command = stream.read_u8().await?;
    let port = stream.read_u16().await?;
    let mut ip = [0u8; 4];
    stream.read_exact(&mut ip).await?;
    // The user id, which nothing checks: any bytes, as clients send the local user's name in its
    // code page.
    read_nul_terminated(stream).await?;
    // SOCKS 4a: 0.0.0.x with x > 0 means the host name follows.
    let host = if ip[..3] == [0, 0, 0] && ip[3] != 0 {
        read_name(stream).await?
    } else {
        Ipv4Addr::from(ip).to_string()
    };
    let request = Request {
        host,
        port,
        version: Version::V4,
    };
    if command != 1 {
        request.failed(stream, Failure::CommandNotSupported).await?;
        return Err(invalid(format!(
            "SOCKS 4 command {command} is not supported"
        )));
    }
    if request.host.is_empty() {
        request.failed(stream, Failure::General).await?;
        return Err(invalid("SOCKS 4a request without a host name"));
    }
    Ok(request)
}

/// A NUL-terminated field, read a byte at a time so that nothing after it is consumed.
async fn read_nul_terminated<S>(stream: &mut S) -> io::Result<Vec<u8>>
where
    S: AsyncRead + Unpin,
{
    let mut field = Vec::new();
    loop {
        match stream.read_u8().await? {
            0 => return Ok(field),
            _ if field.len() == MAX_NAME => return Err(invalid("SOCKS 4 name too long")),
            b => field.push(b),
        }
    }
}

/// A NUL-terminated host name.
async fn read_name<S>(stream: &mut S) -> io::Result<String>
where
    S: AsyncRead + Unpin,
{
    String::from_utf8(read_nul_terminated(stream).await?)
        .map_err(|_| invalid("SOCKS 4 name is not UTF-8"))
}

async fn accept_v5<S>(stream: &mut S) -> io::Result<Request>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let count = stream.read_u8().await?;
    let mut methods = vec![0u8; usize::from(count)];
    stream.read_exact(&mut methods).await?;
    if !methods.contains(&0) {
        stream.write_all(&[5, 0xff]).await?;
        return Err(invalid("the SOCKS 5 client wants authentication"));
    }
    stream.write_all(&[5, 0]).await?;

    let mut head = [0u8; 4];
    stream.read_exact(&mut head).await?;
    let [version, command, _reserved, address_type] = head;
    if version != 5 {
        return Err(invalid(format!("SOCKS 5 request with version {version}")));
    }
    let mut request = Request {
        host: String::new(),
        port: 0,
        version: Version::V5,
    };
    request.host = match address_type {
        1 => {
            let mut ip = [0u8; 4];
            stream.read_exact(&mut ip).await?;
            Ipv4Addr::from(ip).to_string()
        }
        3 => {
            let len = stream.read_u8().await?;
            let mut name = vec![0u8; usize::from(len)];
            stream.read_exact(&mut name).await?;
            match String::from_utf8(name) {
                Ok(name) if !name.is_empty() => name,
                _ => {
                    request.failed(stream, Failure::General).await?;
                    return Err(invalid("SOCKS 5 request with an unusable host name"));
                }
            }
        }
        4 => {
            let mut ip = [0u8; 16];
            stream.read_exact(&mut ip).await?;
            Ipv6Addr::from(ip).to_string()
        }
        other => {
            request.failed(stream, Failure::AddressNotSupported).await?;
            return Err(invalid(format!(
                "SOCKS 5 address type {other} is not supported"
            )));
        }
    };
    request.port = stream.read_u16().await?;
    if command != 1 {
        request.failed(stream, Failure::CommandNotSupported).await?;
        return Err(invalid(format!(
            "SOCKS 5 command {command} is not supported"
        )));
    }
    Ok(request)
}

impl Request {
    /// Tells the client the connection is open.
    pub(crate) async fn succeeded<S>(&self, stream: &mut S) -> io::Result<()>
    where
        S: AsyncWrite + Unpin,
    {
        stream.write_all(&self.reply(None)).await
    }

    /// Tells the client the connection did not open.
    pub(crate) async fn failed<S>(&self, stream: &mut S, failure: Failure) -> io::Result<()>
    where
        S: AsyncWrite + Unpin,
    {
        stream.write_all(&self.reply(Some(failure))).await
    }

    /// The reply, with an unspecified bound address: the server's end of the connection is not
    /// known here.
    fn reply(&self, failure: Option<Failure>) -> Vec<u8> {
        match self.version {
            Version::V4 => vec![0, if failure.is_none() { 90 } else { 91 }, 0, 0, 0, 0, 0, 0],
            Version::V5 => vec![5, failure.map_or(0, Failure::code), 0, 1, 0, 0, 0, 0, 0, 0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    /// Runs [`accept`] on `sent` and returns its result and everything it wrote back.
    async fn run(sent: &[u8]) -> (io::Result<Request>, Vec<u8>) {
        let (mut client, mut server) = duplex(1024);
        client.write_all(sent).await.unwrap();
        client.shutdown().await.unwrap();
        let result = accept(&mut server).await;
        drop(server);
        let mut back = Vec::new();
        client.read_to_end(&mut back).await.unwrap();
        (result, back)
    }

    #[tokio::test]
    async fn socks5_connect_to_each_address_type() {
        let (r, back) = run(&[5, 1, 0, 5, 1, 0, 1, 10, 0, 0, 7, 0x1f, 0x90]).await;
        let r = r.unwrap();
        assert_eq!((r.host.as_str(), r.port), ("10.0.0.7", 8080));
        assert_eq!(
            back,
            [5, 0],
            "no authentication, nothing more until the channel opens"
        );

        let mut sent = vec![5, 2, 2, 0, 5, 1, 0, 3, 11];
        sent.extend_from_slice(b"example.com");
        sent.extend_from_slice(&443u16.to_be_bytes());
        let r = run(&sent).await.0.unwrap();
        assert_eq!((r.host.as_str(), r.port), ("example.com", 443));

        let mut sent = vec![5, 1, 0, 5, 1, 0, 4];
        sent.extend_from_slice(&Ipv6Addr::LOCALHOST.octets());
        sent.extend_from_slice(&22u16.to_be_bytes());
        let r = run(&sent).await.0.unwrap();
        assert_eq!((r.host.as_str(), r.port), ("::1", 22));
    }

    #[tokio::test]
    async fn socks5_replies() {
        let r = run(&[5, 1, 0, 5, 1, 0, 1, 127, 0, 0, 1, 0, 80])
            .await
            .0
            .unwrap();
        assert_eq!(r.reply(None), [5, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            r.reply(Some(Failure::Refused)),
            [5, 5, 0, 1, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(r.reply(Some(Failure::NotAllowed))[1], 2);
        assert_eq!(r.reply(Some(Failure::General))[1], 1);
    }

    #[tokio::test]
    async fn socks5_refusals() {
        // Only username/password offered.
        let (r, back) = run(&[5, 1, 2]).await;
        assert!(r.is_err());
        assert_eq!(back, [5, 0xff]);
        // BIND and UDP ASSOCIATE.
        for command in [2, 3] {
            let (r, back) = run(&[5, 1, 0, 5, command, 0, 1, 127, 0, 0, 1, 0, 80]).await;
            assert!(r.is_err());
            assert_eq!(back[2..], [5, 7, 0, 1, 0, 0, 0, 0, 0, 0]);
        }
        // An unknown address type, and an empty host name.
        let (r, back) = run(&[5, 1, 0, 5, 1, 0, 9]).await;
        assert!(r.is_err());
        assert_eq!(back[2..4], [5, 8]);
        let (r, back) = run(&[5, 1, 0, 5, 1, 0, 3, 0, 0, 80]).await;
        assert!(r.is_err());
        assert_eq!(back[2..4], [5, 1]);
        // Neither 4 nor 5, and a request that ends early.
        let (r, back) = run(b"GET / HTTP/1.1\r\n").await;
        assert!(r.is_err());
        assert!(back.is_empty());
        let (r, _) = run(&[5, 1, 0, 5, 1, 0, 1, 127]).await;
        assert_eq!(r.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    }

    #[tokio::test]
    async fn socks4_and_4a_connect() {
        let mut sent = vec![4, 1, 0, 80, 192, 168, 1, 2];
        sent.extend_from_slice(b"user\0");
        let (r, back) = run(&sent).await;
        let r = r.unwrap();
        assert_eq!((r.host.as_str(), r.port), ("192.168.1.2", 80));
        assert!(back.is_empty(), "the reply waits for the channel");
        assert_eq!(r.reply(None), [0, 90, 0, 0, 0, 0, 0, 0]);
        assert_eq!(r.reply(Some(Failure::Refused)), [0, 91, 0, 0, 0, 0, 0, 0]);

        let mut sent = vec![4, 1, 0x1f, 0x90, 0, 0, 0, 1, 0];
        sent.extend_from_slice(b"db.internal\0");
        let r = run(&sent).await.0.unwrap();
        assert_eq!((r.host.as_str(), r.port), ("db.internal", 8080));

        // A user name in a legacy code page is not UTF-8.
        let r = run(&[4, 1, 0, 80, 192, 168, 1, 2, 0xc0, 0xee, 0])
            .await
            .0
            .unwrap();
        assert_eq!(r.host, "192.168.1.2");
    }

    #[tokio::test]
    async fn socks4_refusals() {
        let (r, back) = run(&[4, 2, 0, 80, 127, 0, 0, 1, 0]).await;
        assert!(r.is_err());
        assert_eq!(back, [0, 91, 0, 0, 0, 0, 0, 0]);
        let (r, back) = run(&[4, 1, 0, 80, 0, 0, 0, 1, 0, 0]).await;
        assert!(r.is_err(), "4a without a host name");
        assert_eq!(back[1], 91);
        let mut sent = vec![4, 1, 0, 80, 127, 0, 0, 1];
        sent.extend(std::iter::repeat_n(b'a', MAX_NAME + 1));
        sent.push(0);
        assert!(run(&sent).await.0.is_err(), "a user id that never ends");
    }

    #[tokio::test]
    async fn data_after_the_request_stays_unread() {
        let (mut client, mut server) = duplex(1024);
        let mut sent = vec![4, 1, 0, 80, 127, 0, 0, 1, 0];
        sent.extend_from_slice(b"hello");
        client.write_all(&sent).await.unwrap();
        accept(&mut server).await.unwrap();
        let mut rest = [0u8; 5];
        server.read_exact(&mut rest).await.unwrap();
        assert_eq!(&rest, b"hello");
    }
}
