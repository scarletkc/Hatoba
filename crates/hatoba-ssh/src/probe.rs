//! Cheap reachability probe for the host status dot (HOST-10).

use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::time::Instant;

use crate::net::{ceil_ms, resolve};
use crate::proxy::{self, ProxyConfig};

/// Opens a TCP connection to `host:port` and immediately closes it; no SSH
/// handshake and no authentication. Returns the connect time in milliseconds,
/// or `None` if the host did not accept a connection within `timeout`
/// (DNS failure, refusal, unreachable or timeout all map to `None`).
pub async fn tcp_probe(host: &str, port: u16, timeout: Duration) -> Option<u32> {
    let attempt = async {
        let addrs = resolve(host, port).await.ok()?;
        for addr in addrs {
            let started = Instant::now();
            if TcpStream::connect(addr).await.is_ok() {
                return Some(ceil_ms(started.elapsed()));
            }
        }
        None
    };
    tokio::time::timeout(timeout, attempt).await.ok().flatten()
}

/// [`tcp_probe`] through `proxy`, when there is one (SSH-13). Some proxies confirm a connection
/// before they have reached the server, so through a proxy the host counts as online only once
/// the server's SSH identification string starts to arrive, and the time runs until then.
pub async fn tcp_probe_via(
    proxy: Option<&ProxyConfig>,
    host: &str,
    port: u16,
    timeout: Duration,
) -> Option<u32> {
    let Some(proxy) = proxy else {
        return tcp_probe(host, port, timeout).await;
    };
    let attempt = async {
        let started = Instant::now();
        let mut stream = proxy::reach(proxy).await.ok()?;
        proxy::handshake(&mut stream, proxy, host, port)
            .await
            .ok()?;
        let mut first = [0u8; 1];
        (stream.read(&mut first).await.ok()? == 1).then(|| ceil_ms(started.elapsed()))
    };
    tokio::time::timeout(timeout, attempt).await.ok().flatten()
}
