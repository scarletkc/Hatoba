//! Shared helpers for the integration tests: a throw-away OpenSSH `sshd`, a
//! recording host key verifier and small stream utilities.
//!
//! The sshd tests only run when `HATOBA_SSH_IT=1` is set (CI runners may lack
//! sshd); without it `SshdServer::start()` prints a note and returns `None`.
#![allow(dead_code)]

pub mod ask_server;

use std::fs;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use hatoba_ssh::{
    AuthMethod, ConnectConfig, HostKeyInfo, HostKeyVerifier, ShellEvent, SshSession, connect,
    parse_private_key,
};
use tokio::sync::mpsc::Receiver;
use zeroize::Zeroizing;

pub const PASS: &str = "hatoba-test";
pub const TEST_USER: &str = "hatobatest";
pub const TEST_PASSWORD: &str = "hatoba-pw-123";

pub fn fixture(name: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/keys")
        .join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// `AuthMethod::PrivateKey` built from a fixture (parsed with the crate's own parser,
/// exactly like the app will store it).
pub fn key_auth(fixture_name: &str, passphrase: Option<&str>) -> AuthMethod {
    let parsed = parse_private_key(&fixture(fixture_name), passphrase)
        .unwrap_or_else(|e| panic!("{fixture_name}: {e}"));
    AuthMethod::PrivateKey {
        openssh: parsed.openssh_private.clone(),
        passphrase: parsed
            .encrypted
            .then(|| Zeroizing::new(passphrase.unwrap().to_owned())),
    }
}

/// Host key verifier that records every call.
pub struct Verifier {
    pub accept: bool,
    /// Simulated user think time before answering.
    pub delay: Duration,
    pub calls: Mutex<Vec<(String, u16, HostKeyInfo)>>,
}

impl Verifier {
    pub fn accepting() -> Arc<Self> {
        Arc::new(Self {
            accept: true,
            delay: Duration::ZERO,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub fn rejecting() -> Arc<Self> {
        Arc::new(Self {
            accept: false,
            delay: Duration::ZERO,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub fn slow(delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            accept: true,
            delay,
            calls: Mutex::new(Vec::new()),
        })
    }

    pub fn calls(&self) -> Vec<(String, u16, HostKeyInfo)> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl HostKeyVerifier for Verifier {
    async fn verify(&self, host: &str, port: u16, key: &HostKeyInfo) -> bool {
        self.calls
            .lock()
            .unwrap()
            .push((host.to_owned(), port, key.clone()));
        if !self.delay.is_zero() {
            tokio::time::sleep(self.delay).await;
        }
        self.accept
    }
}

pub fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().port()
}

fn sshd_path() -> Option<PathBuf> {
    ["/usr/sbin/sshd", "/usr/local/sbin/sshd", "/usr/bin/sshd"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "0")
        .unwrap_or(false)
}

fn current_user() -> String {
    let out = Command::new("id").arg("-un").output().expect("id -un");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Creates the password test user once per process (root only).
fn ensure_password_user() -> bool {
    static ONCE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ONCE.get_or_init(|| {
        if !is_root() {
            return false;
        }
        let exists = Command::new("id")
            .arg(TEST_USER)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !exists {
            let ok = Command::new("useradd")
                .args(["-m", "-s", "/bin/bash", TEST_USER])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                return false;
            }
        }
        let mut child = match Command::new("chpasswd").stdin(Stdio::piped()).spawn() {
            Ok(c) => c,
            Err(_) => return false,
        };
        child
            .stdin
            .take()
            .unwrap()
            .write_all(format!("{TEST_USER}:{TEST_PASSWORD}\n").as_bytes())
            .unwrap();
        child.wait().map(|s| s.success()).unwrap_or(false)
    })
}

/// Which host key type the test server presents.
#[derive(Clone, Copy)]
pub enum HostKeyType {
    Ed25519,
    Rsa,
    Ecdsa,
}

pub struct SshdServer {
    child: Child,
    pub port: u16,
    pub dir: tempfile::TempDir,
    /// User that the fixture keys log in as (the user running the tests).
    pub key_user: String,
    /// Dedicated user with a known password (only when running as root).
    pub password_user: Option<(&'static str, &'static str)>,
    /// `SHA256:...` of the server host key as printed by `ssh-keygen -l`.
    pub host_fingerprint: String,
    pub host_key_type: &'static str,
}

impl SshdServer {
    /// Starts sshd, or returns `None` (after printing why) if the integration tests are disabled.
    pub fn start() -> Option<Self> {
        Self::start_with(HostKeyType::Ed25519)
    }

    pub fn start_with(host_key: HostKeyType) -> Option<Self> {
        if std::env::var("HATOBA_SSH_IT").as_deref() != Ok("1") {
            eprintln!("HATOBA_SSH_IT=1 not set: skipping sshd integration test");
            return None;
        }
        let sshd =
            sshd_path().expect("HATOBA_SSH_IT=1 but sshd was not found (install openssh-server)");
        if !Path::new("/run/sshd").exists() {
            // Privilege separation directory, normally created by the service unit.
            fs::create_dir_all("/run/sshd").expect("cannot create /run/sshd (run as root)");
        }

        let dir = tempfile::Builder::new()
            .prefix("hatoba-sshd-")
            .tempdir()
            .unwrap();
        let (kt, key_name) = match host_key {
            HostKeyType::Ed25519 => ("ed25519", "host_key"),
            HostKeyType::Rsa => ("rsa", "host_key"),
            HostKeyType::Ecdsa => ("ecdsa", "host_key"),
        };
        let host_key_path = dir.path().join(key_name);
        let status = Command::new("ssh-keygen")
            .args(["-q", "-t", kt, "-N", "", "-f"])
            .arg(&host_key_path)
            .status()
            .expect("ssh-keygen");
        assert!(status.success());
        let fp_out = Command::new("ssh-keygen")
            .args(["-l", "-E", "sha256", "-f"])
            .arg(dir.path().join("host_key.pub"))
            .output()
            .unwrap();
        let host_fingerprint = String::from_utf8_lossy(&fp_out.stdout)
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_owned();

        // Every fixture key may log in.
        let mut pubs: Vec<_> =
            fs::read_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keys"))
                .unwrap()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|x| x == "pub"))
                .collect();
        pubs.sort();
        let authorized: String = pubs
            .iter()
            .map(|p| {
                let l = fs::read_to_string(p).unwrap();
                format!("{}\n", l.trim())
            })
            .collect();
        fs::write(dir.path().join("authorized_keys"), authorized).unwrap();

        let password_user = ensure_password_user().then_some((TEST_USER, TEST_PASSWORD));
        let config_path = dir.path().join("sshd_config");
        // `free_port` releases the port before sshd binds it, so a parallel test can take it in
        // between. sshd then exits with "Cannot bind any address" and another port is tried.
        let mut attempt = 0;
        let (child, port) = loop {
            attempt += 1;
            let port = free_port();
            let config = format!(
                "Port {port}\n\
                 ListenAddress 127.0.0.1\n\
                 HostKey {dir}/host_key\n\
                 PidFile {dir}/sshd.pid\n\
                 AuthorizedKeysFile {dir}/authorized_keys\n\
                 StrictModes no\n\
                 UsePAM no\n\
                 PasswordAuthentication yes\n\
                 KbdInteractiveAuthentication no\n\
                 PubkeyAuthentication yes\n\
                 PermitRootLogin yes\n\
                 AllowTcpForwarding yes\n\
                 AcceptEnv LANG HATOBA_*\n\
                 Subsystem sftp internal-sftp\n\
                 MaxAuthTries 10\n\
                 MaxSessions 100\n\
                 LoginGraceTime 30\n\
                 UseDNS no\n\
                 PrintMotd no\n\
                 LogLevel ERROR\n",
                dir = dir.path().display()
            );
            fs::write(&config_path, config).unwrap();

            let log = fs::File::create(dir.path().join("sshd.log")).unwrap();
            let mut child = Command::new(&sshd)
                .args(["-D", "-e", "-f"])
                .arg(&config_path)
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .expect("failed to spawn sshd");
            match wait_for_listen(&mut child, dir.path()) {
                Ok(()) => break (child, port),
                Err((_, log)) if attempt < 5 && log.contains("Cannot bind any address") => {}
                Err((status, log)) => panic!("sshd exited early ({status}): {log}"),
            }
        };

        Some(Self {
            child,
            port,
            dir,
            key_user: current_user(),
            password_user,
            host_fingerprint,
            host_key_type: match host_key {
                HostKeyType::Ed25519 => "ssh-ed25519",
                HostKeyType::Rsa => "ssh-rsa",
                HostKeyType::Ecdsa => "ecdsa-sha2-nistp256",
            },
        })
    }

    pub fn log(&self) -> String {
        fs::read_to_string(self.dir.path().join("sshd.log")).unwrap_or_default()
    }

    /// Kills sshd and all its connection processes' parent; existing connections drop.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    pub fn config(&self, auth: AuthMethod) -> ConnectConfig {
        ConnectConfig::new("127.0.0.1", self.port, self.key_user.clone(), auth)
    }

    pub async fn connect_key(&self, fixture_name: &str, passphrase: Option<&str>) -> SshSession {
        connect(
            self.config(key_auth(fixture_name, passphrase)),
            Verifier::accepting(),
        )
        .await
        .unwrap_or_else(|e| panic!("connect with {fixture_name}: {e}"))
    }

    /// Scratch directory for SFTP tests (created inside the server dir).
    pub fn area(&self, name: &str) -> PathBuf {
        let p = self.dir.path().join(name);
        fs::create_dir_all(&p).unwrap();
        p
    }
}

/// Waits until the sshd in `child` has bound its listen socket, or returns its exit status and
/// log if it exits first. sshd writes its pid file only once the socket is bound (`sshd.c`, after
/// `server_listen`). A TCP probe cannot tell: it can reach another socket that holds the port for
/// a moment, or connect to itself while nothing listens there.
fn wait_for_listen(child: &mut Child, dir: &Path) -> Result<(), (ExitStatus, String)> {
    let pid = child.id().to_string();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if fs::read_to_string(dir.join("sshd.pid")).is_ok_and(|p| p.trim() == pid) {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            let log = fs::read_to_string(dir.join("sshd.log")).unwrap_or_default();
            return Err((status, log));
        }
        assert!(Instant::now() < deadline, "sshd did not start in time");
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl Drop for SshdServer {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Reads shell events until `done(&collected)` holds; panics on timeout.
pub async fn read_until(
    rx: &mut Receiver<ShellEvent>,
    timeout: Duration,
    done: impl Fn(&[u8]) -> bool,
) -> Vec<u8> {
    let mut out = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if done(&out) {
            return out;
        }
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(ShellEvent::Data(d))) => out.extend_from_slice(&d),
            Ok(Some(other)) => panic!(
                "unexpected shell event {other:?}; collected so far: {:?}",
                String::from_utf8_lossy(&out)
            ),
            Ok(None) => panic!(
                "shell channel ended; collected: {:?}",
                String::from_utf8_lossy(&out)
            ),
            Err(_) => panic!("timed out; collected: {:?}", String::from_utf8_lossy(&out)),
        }
    }
}

pub fn contains(haystack: &[u8], needle: &str) -> bool {
    String::from_utf8_lossy(haystack).contains(needle)
}

// ---------------------------------------------------------------------------
// A TCP proxy whose connections can be cut or silenced, to simulate network faults.
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener as TokioListener;
use tokio_util::sync::CancellationToken;

pub struct Proxy {
    pub port: u16,
    blackhole: Arc<AtomicBool>,
    sever: CancellationToken,
    accept: tokio::task::JoinHandle<()>,
}

impl Proxy {
    /// Forwards `127.0.0.1:<port>` to `upstream_port`.
    pub async fn start(upstream_port: u16) -> Self {
        let listener = TokioListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let blackhole = Arc::new(AtomicBool::new(false));
        let sever = CancellationToken::new();
        let (bh, sv) = (blackhole.clone(), sever.clone());
        let accept = tokio::spawn(async move {
            loop {
                let Ok((client, _)) = listener.accept().await else {
                    break;
                };
                let Ok(upstream) =
                    tokio::net::TcpStream::connect(("127.0.0.1", upstream_port)).await
                else {
                    continue;
                };
                let (bh, sv) = (bh.clone(), sv.clone());
                tokio::spawn(async move {
                    let (cr, cw) = client.into_split();
                    let (ur, uw) = upstream.into_split();
                    tokio::join!(pump(cr, uw, bh.clone(), sv.clone()), pump(ur, cw, bh, sv));
                });
            }
        });
        Self {
            port,
            blackhole,
            sever,
            accept,
        }
    }

    /// Keeps connections open but silently discards all traffic in both directions.
    pub fn blackhole(&self) {
        self.blackhole.store(true, Ordering::SeqCst);
    }

    /// Closes every connection (peers see EOF).
    pub fn sever(&self) {
        self.sever.cancel();
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.accept.abort();
        self.sever.cancel();
    }
}

async fn pump(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    blackhole: Arc<AtomicBool>,
    sever: CancellationToken,
) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        tokio::select! {
            () = sever.cancelled() => break,
            n = from.read(&mut buf) => match n {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if blackhole.load(Ordering::SeqCst) {
                        continue;
                    }
                    if to.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
            },
        }
    }
}

/// Deterministic pseudo-random bytes (xorshift), so file contents are reproducible.
pub fn pseudo_random(len: usize, seed: u64) -> Vec<u8> {
    let mut x = seed | 1;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(&x.to_le_bytes());
    }
    out.truncate(len);
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
