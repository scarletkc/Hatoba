//! Name resolution, TCP connect and the pausable connect timeout.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::time::Instant;

use crate::error::{SshError, SshErrorKind, classify_io};

/// Which stage of connection setup is currently running (used in timeout messages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Dns,
    Tcp,
    /// Resolving and connecting to the proxy.
    Proxy,
    /// The proxy is opening the connection to the server.
    ProxyTunnel,
    Handshake,
    Auth,
    Channel,
}

impl Phase {
    fn describe(self) -> &'static str {
        match self {
            Phase::Dns => "resolving the host name",
            Phase::Tcp => "establishing the TCP connection",
            Phase::Proxy => "connecting to the proxy",
            Phase::ProxyTunnel => "waiting for the proxy to connect to the server",
            Phase::Handshake => "the SSH handshake",
            Phase::Auth => "authentication",
            Phase::Channel => "opening the tunnel channel",
        }
    }

    /// The kind of a timeout in this phase: the proxy itself not answering is told apart from
    /// the server not answering.
    fn timeout_kind(self) -> SshErrorKind {
        match self {
            Phase::Proxy => SshErrorKind::ProxyUnreachable,
            _ => SshErrorKind::Timeout,
        }
    }
}

struct ClockState {
    start: Instant,
    paused_total: Duration,
    paused_since: Option<Instant>,
    pause_depth: u32,
    phase: Phase,
}

/// A deadline that does not tick while the user is being asked something
/// (host key prompt, 2FA code). One clock covers DNS + TCP + handshake + auth
/// of a single hop.
pub(crate) struct ConnectClock {
    state: Mutex<ClockState>,
}

/// Resumes the clock when dropped.
pub(crate) struct PauseGuard {
    clock: Arc<ConnectClock>,
}

impl Drop for PauseGuard {
    fn drop(&mut self) {
        let mut s = self.clock.state.lock().unwrap_or_else(|e| e.into_inner());
        s.pause_depth = s.pause_depth.saturating_sub(1);
        if s.pause_depth == 0
            && let Some(since) = s.paused_since.take()
        {
            s.paused_total += since.elapsed();
        }
    }
}

impl ConnectClock {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(ClockState {
                start: Instant::now(),
                paused_total: Duration::ZERO,
                paused_since: None,
                pause_depth: 0,
                phase: Phase::Dns,
            }),
        })
    }

    pub(crate) fn set_phase(&self, phase: Phase) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).phase = phase;
    }

    /// Stops the clock until the returned guard is dropped.
    pub(crate) fn pause(self: &Arc<Self>) -> PauseGuard {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.pause_depth == 0 {
            s.paused_since = Some(Instant::now());
        }
        s.pause_depth += 1;
        drop(s);
        PauseGuard {
            clock: Arc::clone(self),
        }
    }

    fn snapshot(&self) -> (Duration, bool, Phase) {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut paused = s.paused_total;
        if let Some(since) = s.paused_since {
            paused += since.elapsed();
        }
        (
            s.start.elapsed().saturating_sub(paused),
            s.pause_depth > 0,
            s.phase,
        )
    }

    /// Runs `fut`, failing with [`SshErrorKind::Timeout`] once `budget` of
    /// *unpaused* time has elapsed.
    pub(crate) async fn run<F: Future>(
        &self,
        budget: Duration,
        fut: F,
    ) -> Result<F::Output, SshError> {
        tokio::pin!(fut);
        loop {
            let (elapsed, paused, phase) = self.snapshot();
            let remaining = budget.saturating_sub(elapsed);
            if remaining.is_zero() && !paused {
                return Err(SshError::new(
                    phase.timeout_kind(),
                    format!(
                        "timed out after {} s while {}",
                        budget.as_secs_f32().round() as u64,
                        phase.describe()
                    ),
                ));
            }
            let tick = if paused {
                Duration::from_millis(100)
            } else {
                remaining
            };
            tokio::select! {
                out = &mut fut => return Ok(out),
                () = tokio::time::sleep(tick) => {}
            }
        }
    }
}

/// Resolves `host:port`, mapping every failure to [`SshErrorKind::Dns`].
pub(crate) async fn resolve(host: &str, port: u16) -> Result<Vec<SocketAddr>, SshError> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| SshError::new(SshErrorKind::Dns, format!("cannot resolve {host}: {e}")))?
        .collect();
    if addrs.is_empty() {
        return Err(SshError::new(
            SshErrorKind::Dns,
            format!("cannot resolve {host}: no addresses found"),
        ));
    }
    Ok(addrs)
}

/// Relative usefulness of a connect error when several addresses failed.
fn error_rank(kind: SshErrorKind) -> u8 {
    match kind {
        SshErrorKind::Refused => 3,
        SshErrorKind::Timeout => 2,
        SshErrorKind::Unreachable => 1,
        _ => 0,
    }
}

/// Connects to the first reachable address. Returns the stream and the time
/// the winning `connect` took (the TCP RTT).
///
/// Attempts that are not the last one are capped so a black-holed first
/// address (typically IPv6) cannot eat the whole connect budget.
pub(crate) async fn tcp_connect(
    host: &str,
    port: u16,
    clock: &ConnectClock,
) -> Result<(TcpStream, Duration), SshError> {
    clock.set_phase(Phase::Dns);
    let addrs = resolve(host, port).await?;
    clock.set_phase(Phase::Tcp);
    connect_first(&addrs).await
}

/// Connects to the first of `addrs` that accepts, as [`tcp_connect`] does after resolving.
pub(crate) async fn connect_first(addrs: &[SocketAddr]) -> Result<(TcpStream, Duration), SshError> {
    let Some(last) = addrs.len().checked_sub(1) else {
        return Err(SshError::other("no address to connect to"));
    };
    let mut best: Option<SshError> = None;
    for (i, addr) in addrs.iter().enumerate() {
        let started = Instant::now();
        let attempt = TcpStream::connect(addr);
        let result = if i == last {
            attempt.await.map_err(|e| (classify_io(&e), e.to_string()))
        } else {
            match tokio::time::timeout(Duration::from_secs(4), attempt).await {
                Ok(r) => r.map_err(|e| (classify_io(&e), e.to_string())),
                Err(_) => Err((
                    SshErrorKind::Timeout,
                    "connect attempt timed out".to_string(),
                )),
            }
        };
        match result {
            Ok(stream) => {
                let rtt = started.elapsed();
                if let Err(e) = stream.set_nodelay(true) {
                    tracing::debug!("set_nodelay failed: {e}");
                }
                return Ok((stream, rtt));
            }
            Err((kind, msg)) => {
                let err = SshError::new(kind, format!("connect to {addr}: {msg}"));
                if best
                    .as_ref()
                    .is_none_or(|b| error_rank(kind) > error_rank(b.kind))
                {
                    best = Some(err);
                }
            }
        }
    }
    Err(best.unwrap_or_else(|| SshError::other("no address to connect to")))
}

/// Milliseconds rounded up so a sub-millisecond loopback RTT shows as 1.
pub(crate) fn ceil_ms(d: Duration) -> u32 {
    let us = d.as_micros();
    u32::try_from(us.div_ceil(1000)).unwrap_or(u32::MAX).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn clock_times_out_when_not_paused() {
        let clock = ConnectClock::new();
        clock.set_phase(Phase::Handshake);
        let started = std::time::Instant::now();
        let err = clock
            .run(Duration::from_millis(200), std::future::pending::<()>())
            .await
            .unwrap_err();
        assert_eq!(err.kind, SshErrorKind::Timeout);
        assert!(err.message.contains("handshake"), "{err}");
        assert!(started.elapsed() >= Duration::from_millis(190));
    }

    #[tokio::test]
    async fn clock_does_not_tick_while_paused() {
        let clock = ConnectClock::new();
        let pauser = Arc::clone(&clock);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let _guard = pauser.pause();
            tokio::time::sleep(Duration::from_millis(600)).await;
        });
        // ~700 ms of wall time, ~100 ms of it unpaused, budget 300 ms.
        let out = clock
            .run(Duration::from_millis(300), async {
                tokio::time::sleep(Duration::from_millis(700)).await;
                7
            })
            .await;
        assert_eq!(out.unwrap(), 7);
    }

    #[tokio::test]
    async fn nested_pauses_resume_only_after_the_last_guard() {
        let clock = ConnectClock::new();
        let a = clock.pause();
        let b = clock.pause();
        drop(a);
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(clock.snapshot().1, "still paused by the second guard");
        drop(b);
        assert!(!clock.snapshot().1);
        assert!(
            clock.snapshot().0 < Duration::from_millis(30),
            "paused time is excluded"
        );
    }

    #[test]
    fn millisecond_rounding() {
        assert_eq!(ceil_ms(Duration::from_micros(10)), 1);
        assert_eq!(ceil_ms(Duration::ZERO), 1);
        assert_eq!(ceil_ms(Duration::from_micros(1500)), 2);
        assert_eq!(ceil_ms(Duration::from_millis(40)), 40);
    }

    #[test]
    fn best_connect_error_prefers_refused() {
        assert!(error_rank(SshErrorKind::Refused) > error_rank(SshErrorKind::Timeout));
        assert!(error_rank(SshErrorKind::Timeout) > error_rank(SshErrorKind::Unreachable));
        assert!(error_rank(SshErrorKind::Unreachable) > error_rank(SshErrorKind::Io));
    }
}
