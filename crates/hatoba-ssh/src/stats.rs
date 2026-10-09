//! Resource usage of a Linux server, sampled from `/proc` over the session (TERM-12).
//!
//! [`SshSession::open_stats`] runs `sh -s` on an exec channel and sends it `stats.sh` on stdin,
//! so nothing is installed on the server and the user's login shell only has to start `sh`.
//! The script prints a block of `/proc` readings each interval; [`Parser`] turns each block into
//! a [`Sample`], and two consecutive samples give the rates in [`ServerStats`].

use std::time::Duration;

use bytes::Bytes;
use russh::client::Msg;
use russh::{Channel, ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Sig};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TrySendError;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::error::SshError;
use crate::session::{SshSession, wait_request_reply};

/// The sampling script. It takes the interval in whole seconds as `$1`.
const SCRIPT: &str = include_str!("stats.sh");
/// Readings are small and replace each other: a reader that falls behind loses old ones.
const EVENT_CAPACITY: usize = 4;
/// A line longer than this is not one the script prints, and is dropped.
const MAX_LINE: usize = 4096;
/// How much of the script's stderr is kept to explain a failure.
const MAX_STDERR: usize = 2048;
/// `command not found` from the login shell: the server has no `sh`.
const EXIT_NOT_FOUND: u32 = 127;

/// One reading of a server's resource usage. Sizes are in bytes and rates in bytes per second.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ServerStats {
    /// Busy share of all CPUs since the previous reading, 0–100. `None` in the first reading.
    pub cpu_percent: Option<f64>,
    /// Number of CPUs.
    pub cpus: Option<u32>,
    /// Load averages over 1, 5 and 15 minutes.
    pub load: Option<[f64; 3]>,
    /// Physical memory.
    pub mem_total: Option<u64>,
    /// Memory in use: the total less what the kernel counts as available.
    pub mem_used: Option<u64>,
    /// Swap space; zero when the server has none.
    pub swap_total: Option<u64>,
    /// Swap in use.
    pub swap_used: Option<u64>,
    /// Receive rate since the previous reading, summed over [`net_interfaces`](Self::net_interfaces).
    pub net_rx_rate: Option<u64>,
    /// Send rate since the previous reading.
    pub net_tx_rate: Option<u64>,
    /// The interfaces counted: those of the default routes, or else all but loopback.
    pub net_interfaces: Vec<String>,
    /// Size of the root filesystem.
    pub disk_total: Option<u64>,
    /// Space used on the root filesystem.
    pub disk_used: Option<u64>,
    /// Space available to unprivileged users on the root filesystem (`df` leaves out the
    /// blocks reserved for root, so used and available can add up to less than the total).
    pub disk_available: Option<u64>,
    /// Time since the server booted, in seconds.
    pub uptime_secs: Option<u64>,
}

/// What sampling reports.
#[derive(Debug, Clone)]
pub enum StatsEvent {
    /// A reading, one per interval.
    Stats(Box<ServerStats>),
    /// The server does not run Linux, so there is nothing to read. Carries the system's name
    /// (such as `FreeBSD`), empty when unknown. Sent once, last.
    Unsupported(String),
    /// Sampling stopped on its own: the script failed or the connection ended. Sent once, last.
    Ended(SshError),
}

/// Sampling that runs until [`stop`](Self::stop) is called or the handle is dropped.
#[derive(Debug)]
pub struct StatsHandle {
    cancel: CancellationToken,
}

impl StatsHandle {
    /// Stops sampling and closes its channel. No further events are sent.
    pub fn stop(&self) {
        self.cancel.cancel();
    }

    /// `true` until sampling is stopped or ends on its own.
    pub fn is_running(&self) -> bool {
        !self.cancel.is_cancelled()
    }
}

impl Drop for StatsHandle {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

/// Reports a system without `/proc` at once: nothing is run on the server.
pub(crate) fn unsupported(system: &str) -> (StatsHandle, mpsc::Receiver<StatsEvent>) {
    let (tx, rx) = mpsc::channel(1);
    let _ = tx.try_send(StatsEvent::Unsupported(system.to_owned()));
    let cancel = CancellationToken::new();
    cancel.cancel();
    (StatsHandle { cancel }, rx)
}

pub(crate) async fn start(
    session: &SshSession,
    mut channel: Channel<Msg>,
    interval: Duration,
) -> Result<(StatsHandle, mpsc::Receiver<StatsEvent>), SshError> {
    let secs = interval.as_secs().clamp(1, 60);
    channel
        .exec(true, format!("sh -s -- {secs}"))
        .await
        .map_err(SshError::from)?;
    wait_request_reply(&mut channel, "exec").await?;
    let (reader, writer) = channel.split();
    writer
        .data_bytes(Bytes::from_static(SCRIPT.as_bytes()))
        .await
        .map_err(SshError::from)?;
    writer.eof().await.map_err(SshError::from)?;

    let (tx, rx) = mpsc::channel(EVENT_CAPACITY);
    let cancel = CancellationToken::new();
    tokio::spawn(pump(reader, writer, tx, cancel.clone(), session.clone()));
    Ok((StatsHandle { cancel }, rx))
}

/// Parses the script's output into readings until sampling stops.
async fn pump(
    mut reader: ChannelReadHalf,
    writer: ChannelWriteHalf<Msg>,
    tx: mpsc::Sender<StatsEvent>,
    cancel: CancellationToken,
    session: SshSession,
) {
    let mut parser = Parser::default();
    let mut parsed = Vec::new();
    let mut prev: Option<(Sample, Instant)> = None;
    let mut stderr = Vec::new();
    let mut exit_status = None;

    let ended = loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => break None,
            () = tx.closed() => break None,
            msg = reader.wait() => match msg {
                Some(ChannelMsg::Data { data }) => {
                    parser.feed(&data, &mut parsed);
                    for output in parsed.drain(..) {
                        match output {
                            Parsed::Sample(sample) => {
                                let now = Instant::now();
                                let last = prev.as_ref().map(|(s, at)| (s, now - *at));
                                let stats = Box::new(sample.stats(last));
                                prev = Some((*sample, now));
                                if let Err(TrySendError::Closed(_)) = tx.try_send(StatsEvent::Stats(stats)) {
                                    cancel.cancel();
                                }
                            }
                            Parsed::Unsupported(system) => {
                                let _ = tx.send(StatsEvent::Unsupported(system)).await;
                                cancel.cancel();
                            }
                        }
                    }
                }
                Some(ChannelMsg::ExtendedData { data, .. }) => {
                    stderr.extend_from_slice(&data);
                    let excess = stderr.len().saturating_sub(MAX_STDERR);
                    stderr.drain(..excess);
                }
                Some(ChannelMsg::ExitStatus { exit_status: s }) => exit_status = Some(s),
                Some(ChannelMsg::Close) | None => break Some(()),
                Some(_) => {}
            },
        }
    };

    if ended.is_none() || cancel.is_cancelled() {
        // Stopped here: the remote loop ends at its next write to the closed channel, or at
        // once on servers that deliver signals to exec channels.
        cancel.cancel();
        let _ = writer.signal(Sig::TERM).await;
        let _ = writer.eof().await;
        let _ = writer.close().await;
        return;
    }
    cancel.cancel();
    let event = if session.is_closed() {
        StatsEvent::Ended(session.close_error())
    } else if exit_status == Some(EXIT_NOT_FOUND) {
        StatsEvent::Unsupported(String::new())
    } else {
        StatsEvent::Ended(script_error(exit_status, &stderr))
    };
    let _ = tx.send(event).await;
}

/// Describes how the script ended, with the last line it wrote to stderr.
fn script_error(exit_status: Option<u32>, stderr: &[u8]) -> SshError {
    let stderr = String::from_utf8_lossy(stderr);
    let last = stderr.lines().map(str::trim).rfind(|l| !l.is_empty());
    let status = exit_status.map_or_else(
        || "the server closed the channel".to_owned(),
        |s| format!("exit status {s}"),
    );
    SshError::other(match last {
        Some(line) => format!("sampling stopped ({status}): {}", truncate(line, 200)),
        None => format!("sampling stopped ({status})"),
    })
}

fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((i, _)) => &s[..i],
        None => s,
    }
}

// ───────────────────────── Parsing ─────────────────────────

/// What a line of the script's output completes.
#[derive(Debug, PartialEq)]
enum Parsed {
    Sample(Box<Sample>),
    Unsupported(String),
}

/// The block of output a line belongs to, from the last `@name` marker.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Section {
    /// Before the first marker (such as what a login script printed) or after an unknown one.
    #[default]
    None,
    Cpu,
    Mem,
    Net,
    Load,
    Uptime,
    Disk,
}

/// CPU time from the `cpu` line of `/proc/stat`, in clock ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuTimes {
    /// user, nice, system, idle, iowait, irq, softirq and steal (guest time is part of user).
    total: u64,
    /// idle and iowait.
    idle: u64,
}

/// `/proc/meminfo` values, in kB.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct MemInfo {
    total: Option<u64>,
    free: Option<u64>,
    /// Absent before Linux 3.14.
    available: Option<u64>,
    buffers: Option<u64>,
    cached: Option<u64>,
    swap_total: Option<u64>,
    swap_free: Option<u64>,
}

/// One line of `df -kP`, in kB.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Disk {
    total: u64,
    used: u64,
    available: u64,
}

/// The raw readings of one block.
#[derive(Debug, Clone, Default, PartialEq)]
struct Sample {
    cpus: Option<u32>,
    cpu: Option<CpuTimes>,
    mem: MemInfo,
    /// Bytes received and sent since boot, summed over `interfaces`.
    net: Option<(u64, u64)>,
    interfaces: Vec<String>,
    load: Option<[f64; 3]>,
    /// Seconds since boot, with the kernel's centisecond precision.
    uptime: Option<f64>,
    disk: Option<Disk>,
}

/// Splits the script's output into lines and collects them into samples.
#[derive(Debug, Default)]
struct Parser {
    buf: Vec<u8>,
    section: Section,
    current: Sample,
    cpus: Option<u32>,
}

impl Parser {
    fn feed(&mut self, data: &[u8], out: &mut Vec<Parsed>) {
        self.buf.extend_from_slice(data);
        let mut start = 0;
        while let Some(end) = self.buf[start..].iter().position(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(&self.buf[start..start + end]).into_owned();
            start += end + 1;
            if let Some(parsed) = self.line(line.trim_end_matches('\r')) {
                out.push(parsed);
            }
        }
        self.buf.drain(..start);
        if self.buf.len() > MAX_LINE {
            self.buf.clear();
        }
    }

    fn line(&mut self, line: &str) -> Option<Parsed> {
        if let Some(marker) = line.strip_prefix('@') {
            let (name, rest) = marker.split_once(' ').unwrap_or((marker, ""));
            self.section = Section::None;
            match name {
                "unsupported" => return Some(Parsed::Unsupported(rest.trim().to_owned())),
                "cpus" => self.cpus = rest.trim().parse().ok(),
                "cpu" => self.section = Section::Cpu,
                "mem" => self.section = Section::Mem,
                "net" => self.section = Section::Net,
                "load" => self.section = Section::Load,
                "uptime" => self.section = Section::Uptime,
                "disk" => self.section = Section::Disk,
                "end" => {
                    let mut sample = Box::new(std::mem::take(&mut self.current));
                    sample.cpus = self.cpus;
                    return Some(Parsed::Sample(sample));
                }
                _ => {}
            }
            return None;
        }
        let sample = &mut self.current;
        match self.section {
            Section::None => {}
            Section::Cpu => sample.cpu = parse_cpu(line),
            Section::Mem => parse_meminfo(line, &mut sample.mem),
            Section::Net => {
                if let Some((name, rx, tx)) = parse_net_dev(line) {
                    let (sum_rx, sum_tx) = sample.net.unwrap_or_default();
                    sample.net = Some((sum_rx.saturating_add(rx), sum_tx.saturating_add(tx)));
                    sample.interfaces.push(name);
                }
            }
            Section::Load => sample.load = parse_load(line),
            Section::Uptime => sample.uptime = parse_uptime(line),
            // The header line does not parse, the filesystem's line does.
            Section::Disk => sample.disk = parse_df(line).or(sample.disk),
        }
        None
    }
}

/// `cpu  4705 356 584 3699 23 23 0 0 0 0` (older kernels print fewer columns).
fn parse_cpu(line: &str) -> Option<CpuTimes> {
    let mut fields = line.split_whitespace();
    if fields.next()? != "cpu" {
        return None;
    }
    let ticks: Vec<u64> = fields.take(8).map_while(|f| f.parse().ok()).collect();
    if ticks.len() < 4 {
        return None;
    }
    Some(CpuTimes {
        total: ticks.iter().fold(0u64, |a, &b| a.saturating_add(b)),
        idle: ticks[3].saturating_add(ticks.get(4).copied().unwrap_or(0)),
    })
}

/// `MemTotal:       16314292 kB`
fn parse_meminfo(line: &str, mem: &mut MemInfo) {
    let Some((key, rest)) = line.split_once(':') else {
        return;
    };
    let value = rest.split_whitespace().next().and_then(|v| v.parse().ok());
    let slot = match key.trim() {
        "MemTotal" => &mut mem.total,
        "MemFree" => &mut mem.free,
        "MemAvailable" => &mut mem.available,
        "Buffers" => &mut mem.buffers,
        "Cached" => &mut mem.cached,
        "SwapTotal" => &mut mem.swap_total,
        "SwapFree" => &mut mem.swap_free,
        _ => return,
    };
    *slot = value;
}

/// `  eth0: 1234 5 0 0 0 0 0 0 5678 9 0 0 0 0 0 0`: the name and the received and sent bytes
/// (the first and ninth counters). Loopback is never counted.
fn parse_net_dev(line: &str) -> Option<(String, u64, u64)> {
    let (name, counters) = line.split_once(':')?;
    let name = name.trim();
    if name.is_empty() || name == "lo" {
        return None;
    }
    let counters: Vec<u64> = counters
        .split_whitespace()
        .map_while(|c| c.parse().ok())
        .collect();
    Some((name.to_owned(), *counters.first()?, *counters.get(8)?))
}

/// `0.52 0.40 0.31 1/123 4567`
fn parse_load(line: &str) -> Option<[f64; 3]> {
    let mut fields = line
        .split_whitespace()
        .map(|f| f.parse::<f64>().ok().filter(|l| l.is_finite()));
    Some([fields.next()??, fields.next()??, fields.next()??])
}

/// `12345.67 23456.78`: seconds since boot, then idle seconds summed over the CPUs.
fn parse_uptime(line: &str) -> Option<f64> {
    line.split_whitespace()
        .next()?
        .parse()
        .ok()
        .filter(|s: &f64| s.is_finite() && *s >= 0.0)
}

/// `/dev/sda1  30298176 8237812 20497612  29% /`, read from the end: a filesystem name may hold
/// spaces, the mount point `/` does not.
fn parse_df(line: &str) -> Option<Disk> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let n = fields.len();
    if n < 6 || !fields[n - 2].ends_with('%') {
        return None;
    }
    Some(Disk {
        total: fields[n - 5].parse().ok()?,
        used: fields[n - 4].parse().ok()?,
        available: fields[n - 3].parse().ok()?,
    })
}

const KB: u64 = 1024;

impl Sample {
    /// The reading this sample gives, with the CPU share and rates since `prev` (the previous
    /// sample and the time since it arrived).
    fn stats(&self, prev: Option<(&Sample, Duration)>) -> ServerStats {
        let mem = &self.mem;
        let available = mem.available.or_else(|| {
            Some(
                mem.free?
                    .saturating_add(mem.buffers.unwrap_or(0))
                    .saturating_add(mem.cached.unwrap_or(0)),
            )
        });
        let mut stats = ServerStats {
            cpus: self.cpus,
            load: self.load,
            mem_total: mem.total.map(|t| t * KB),
            mem_used: mem
                .total
                .zip(available)
                .map(|(t, a)| t.saturating_sub(a) * KB),
            swap_total: mem.swap_total.map(|t| t * KB),
            swap_used: mem
                .swap_total
                .zip(mem.swap_free)
                .map(|(t, f)| t.saturating_sub(f) * KB),
            net_interfaces: self.interfaces.clone(),
            disk_total: self.disk.map(|d| d.total * KB),
            disk_used: self.disk.map(|d| d.used * KB),
            disk_available: self.disk.map(|d| d.available * KB),
            // Whole seconds: the fraction is cut off, not rounded.
            uptime_secs: self.uptime.map(|s| s as u64),
            ..ServerStats::default()
        };
        let Some((prev, elapsed)) = prev else {
            return stats;
        };

        if let (Some(now), Some(before)) = (self.cpu, prev.cpu) {
            let total = now.total.checked_sub(before.total).filter(|&t| t > 0);
            let idle = now.idle.checked_sub(before.idle);
            if let (Some(total), Some(idle)) = (total, idle) {
                let busy = total.saturating_sub(idle) as f64 / total as f64;
                stats.cpu_percent = Some((busy * 100.0).clamp(0.0, 100.0));
            }
        }

        // The server's own clock, so a delayed block does not show as a burst; the local one
        // when the server's did not move forward.
        let secs = match (self.uptime, prev.uptime) {
            (Some(now), Some(before)) if now > before => now - before,
            _ => elapsed.as_secs_f64(),
        };
        if let (Some((rx, tx)), Some((prev_rx, prev_tx))) = (self.net, prev.net) {
            // Counters that went back (an interface came back up) or a changed set of
            // interfaces give no rate for this reading.
            if secs > 0.0 && self.interfaces == prev.interfaces {
                let rate = |now: u64, before: u64| {
                    now.checked_sub(before)
                        .map(|d| (d as f64 / secs).round() as u64)
                };
                stats.net_rx_rate = rate(rx, prev_rx);
                stats.net_tx_rate = rate(tx, prev_tx);
            }
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two blocks as `stats.sh` printed them on Ubuntu, after a login script's greeting.
    const OUTPUT: &str = "\
Welcome!
@cpus 4
@cpu
cpu  1000 0 500 8000 500 0 0 0 0 0
@mem
MemTotal:       16000000 kB
MemFree:         2000000 kB
MemAvailable:    6000000 kB
Buffers:          100000 kB
Cached:          3000000 kB
SwapTotal:       4194304 kB
SwapFree:        4094304 kB
@net
  eth0: 1000000    800    0    0    0     0          0         0   500000     600    0    0    0     0       0          0
@load
0.52 0.40 0.31 2/345 6789
@uptime
1000.00 3900.00
@disk
Filesystem     1024-blocks     Used Available Capacity Mounted on
/dev/sda1         30000000 10000000  18500000      36% /
@end
@cpu
cpu  1300 0 600 8500 600 0 0 0 0 0
@mem
MemTotal:       16000000 kB
MemFree:         1900000 kB
MemAvailable:    5900000 kB
Buffers:          100000 kB
Cached:          3000000 kB
SwapTotal:       4194304 kB
SwapFree:        4094304 kB
@net
  eth0: 1400000    900    0    0    0     0          0         0   600000     700    0    0    0     0       0          0
@load
0.60 0.42 0.32 1/346 6790
@uptime
1002.00 3907.00
@disk
Filesystem     1024-blocks     Used Available Capacity Mounted on
/dev/sda1         30000000 10000004  18499996      36% /
@end
";

    fn parse_all(chunks: &[&[u8]]) -> Vec<Parsed> {
        let mut parser = Parser::default();
        let mut out = Vec::new();
        for chunk in chunks {
            parser.feed(chunk, &mut out);
        }
        out
    }

    fn samples(output: &str) -> Vec<Sample> {
        parse_all(&[output.as_bytes()])
            .into_iter()
            .map(|p| match p {
                Parsed::Sample(s) => *s,
                Parsed::Unsupported(s) => panic!("unsupported {s}"),
            })
            .collect()
    }

    #[test]
    fn the_first_reading_has_sizes_but_no_rates() {
        let samples = samples(OUTPUT);
        assert_eq!(samples.len(), 2);
        let stats = samples[0].stats(None);
        assert_eq!(stats.cpus, Some(4));
        assert_eq!(stats.cpu_percent, None);
        assert_eq!(stats.load, Some([0.52, 0.40, 0.31]));
        assert_eq!(stats.mem_total, Some(16_000_000 * KB));
        assert_eq!(stats.mem_used, Some(10_000_000 * KB));
        assert_eq!(stats.swap_total, Some(4_194_304 * KB));
        assert_eq!(stats.swap_used, Some(100_000 * KB));
        assert_eq!((stats.net_rx_rate, stats.net_tx_rate), (None, None));
        assert_eq!(stats.net_interfaces, ["eth0"]);
        assert_eq!(stats.disk_total, Some(30_000_000 * KB));
        assert_eq!(stats.disk_used, Some(10_000_000 * KB));
        assert_eq!(stats.disk_available, Some(18_500_000 * KB));
        assert_eq!(stats.uptime_secs, Some(1000));
    }

    #[test]
    fn rates_come_from_two_readings_and_the_servers_clock() {
        let samples = samples(OUTPUT);
        // The block arrived 5 s after the last one locally, but 2 s passed on the server.
        let stats = samples[1].stats(Some((&samples[0], Duration::from_secs(5))));
        // 1000 ticks passed, 600 of them idle or waiting for I/O.
        assert_eq!(stats.cpu_percent, Some(40.0));
        assert_eq!(stats.net_rx_rate, Some(200_000));
        assert_eq!(stats.net_tx_rate, Some(50_000));
        assert_eq!(stats.mem_used, Some(10_100_000 * KB));
        assert_eq!(stats.uptime_secs, Some(1002));
    }

    #[test]
    fn lines_split_across_packets_and_crlf_are_joined() {
        let crlf = OUTPUT.replace('\n', "\r\n");
        let bytes = crlf.as_bytes();
        let chunks: Vec<&[u8]> = bytes.chunks(7).collect();
        let parsed = parse_all(&chunks);
        assert_eq!(parsed, parse_all(&[OUTPUT.as_bytes()]));
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn other_systems_are_reported_by_name() {
        assert_eq!(
            parse_all(&[b"@unsupported FreeBSD\n"]),
            [Parsed::Unsupported("FreeBSD".to_owned())]
        );
        assert_eq!(
            parse_all(&[b"@unsupported \n"]),
            [Parsed::Unsupported(String::new())]
        );
    }

    #[test]
    fn older_kernels_without_mem_available_count_cache_as_available() {
        let mut mem = MemInfo::default();
        for line in [
            "MemTotal:  1000 kB",
            "MemFree:    200 kB",
            "Buffers:     50 kB",
            "Cached:     250 kB",
        ] {
            parse_meminfo(line, &mut mem);
        }
        let sample = Sample {
            mem,
            ..Sample::default()
        };
        assert_eq!(sample.stats(None).mem_used, Some(500 * KB));
    }

    #[test]
    fn counters_that_go_back_or_change_interfaces_give_no_rate() {
        let before = Sample {
            net: Some((5000, 5000)),
            interfaces: vec!["eth0".into()],
            cpu: Some(CpuTimes {
                total: 100,
                idle: 50,
            }),
            ..Sample::default()
        };
        let reset = Sample {
            net: Some((100, 6000)),
            ..before.clone()
        };
        let stats = reset.stats(Some((&before, Duration::from_secs(2))));
        assert_eq!(stats.net_rx_rate, None);
        assert_eq!(stats.net_tx_rate, Some(500));
        // No CPU time passed: no share rather than a division by zero.
        assert_eq!(stats.cpu_percent, None);

        let moved = Sample {
            net: Some((9000, 9000)),
            interfaces: vec!["wg0".into()],
            ..before.clone()
        };
        let stats = moved.stats(Some((&before, Duration::from_secs(2))));
        assert_eq!((stats.net_rx_rate, stats.net_tx_rate), (None, None));
    }

    #[test]
    fn net_dev_lines_with_glued_counters_and_loopback() {
        // Kernels before 3.x glued a long counter to the colon.
        assert_eq!(
            parse_net_dev("  eth0:123456789 10 0 0 0 0 0 0 987 11 0 0 0 0 0 0"),
            Some(("eth0".to_owned(), 123_456_789, 987))
        );
        assert_eq!(
            parse_net_dev("    lo: 5 1 0 0 0 0 0 0 5 1 0 0 0 0 0 0"),
            None
        );
        assert_eq!(parse_net_dev("eth0: 1 2 3"), None);
        let samples = samples(
            "@net\n  eth0: 10 0 0 0 0 0 0 0 20 0 0 0 0 0 0 0\n  wg0: 1 0 0 0 0 0 0 0 2 0 0 0 0 0 0 0\n@end\n",
        );
        assert_eq!(samples[0].net, Some((11, 22)));
        assert_eq!(samples[0].interfaces, ["eth0", "wg0"]);
    }

    #[test]
    fn short_cpu_lines_and_odd_df_output() {
        assert_eq!(
            parse_cpu("cpu 10 0 10 80"),
            Some(CpuTimes {
                total: 100,
                idle: 80
            })
        );
        assert_eq!(parse_cpu("cpu0 10 0 10 80"), None);
        assert_eq!(parse_cpu("cpu 10 0"), None);
        assert_eq!(
            parse_df("map auto_home 100 40 50 45% /"),
            Some(Disk {
                total: 100,
                used: 40,
                available: 50
            })
        );
        assert_eq!(
            parse_df("Filesystem 1024-blocks Used Available Capacity Mounted on"),
            None
        );
        assert_eq!(parse_load("0.1 0.2"), None);
        assert_eq!(parse_load("0.1 0.2 nan 1/2 3"), None);
        assert_eq!(parse_uptime("-1 2"), None);
    }

    #[test]
    fn overlong_garbage_is_dropped() {
        let mut parser = Parser::default();
        let mut out = Vec::new();
        parser.feed(&vec![b'x'; MAX_LINE + 1], &mut out);
        parser.feed(b"\n@end\n", &mut out);
        assert_eq!(out.len(), 1);
        assert!(parser.buf.is_empty());
    }

    #[test]
    fn script_errors_quote_the_last_line_of_stderr() {
        let err = script_error(Some(2), b"one\nsh: 3: awk: not found\n\n");
        assert_eq!(
            err.message,
            "sampling stopped (exit status 2): sh: 3: awk: not found"
        );
        assert_eq!(
            script_error(None, b"").message,
            "sampling stopped (the server closed the channel)"
        );
    }
}
