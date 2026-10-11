//! Integration tests against a real OpenSSH `sshd` (set `HATOBA_SSH_IT=1`):
//! authentication, host key verification, shells, exec and connection loss.
#![cfg(unix)]

mod common;

use std::time::{Duration, Instant};

use common::*;
use hatoba_ssh::{
    AuthMethod, ConnectConfig, GenerateKind, ShellEvent, ShellOptions, SshErrorKind, connect,
    generate_key, tcp_probe,
};
use zeroize::Zeroizing;

macro_rules! server {
    () => {
        match SshdServer::start() {
            Some(s) => s,
            None => return,
        }
    };
    ($kind:expr) => {
        match SshdServer::start_with($kind) {
            Some(s) => s,
            None => return,
        }
    };
}

async fn exec_text(session: &hatoba_ssh::SshSession, cmd: &str) -> String {
    let (status, out) = session.exec(cmd, None).await.unwrap();
    assert_eq!(status, 0, "{cmd}");
    String::from_utf8(out).unwrap()
}

// --------------------------------------------------------------------------- auth

#[tokio::test]
async fn key_auth_all_algorithms() {
    let server = server!();
    for (name, pass) in [
        ("ed25519", None),
        ("rsa", None),
        ("ecdsa256", None),
        ("ecdsa384", None),
        ("ecdsa521", None),
        ("ed25519_enc", Some(PASS)),
        ("rsa_enc", Some(PASS)),
        ("ecdsa256_enc", Some(PASS)),
    ] {
        let session = server.connect_key(name, pass).await;
        let who = exec_text(&session, "id -un").await;
        assert_eq!(who.trim(), server.key_user, "{name}");
        assert!(session.latency_ms() >= 1);
        session.disconnect().await;
    }
}

#[tokio::test]
async fn key_auth_with_converted_formats() {
    // PEM and PuTTY keys are converted to OpenSSH by parse_private_key and must work end to end.
    let server = server!();
    for (name, pass) in [
        ("rsa_pkcs1", None),
        ("rsa_pkcs1_enc", Some(PASS)),
        ("ec_sec1", None),
        ("ed25519_pkcs8", None),
        ("rsa_pkcs8_enc", Some(PASS)),
        ("ed25519_v2.ppk", None),
        ("ed25519_v3_enc.ppk", Some(PASS)),
        ("rsa_v3.ppk", None),
        ("rsa_v2_enc.ppk", Some(PASS)),
        ("ecdsa256_v3.ppk", None),
        ("ecdsa521_v2.ppk", None),
    ] {
        let session = server.connect_key(name, pass).await;
        assert_eq!(exec_text(&session, "echo ok").await.trim(), "ok", "{name}");
        session.disconnect().await;
    }
}

#[tokio::test]
async fn raw_pem_key_text_is_accepted_for_auth() {
    // AuthMethod::PrivateKey normally carries OpenSSH text but any supported format works.
    let server = server!();
    let auth = AuthMethod::PrivateKey {
        openssh: Zeroizing::new(fixture("rsa_pkcs1_enc")),
        passphrase: Some(Zeroizing::new(PASS.to_owned())),
    };
    let session = connect(server.config(auth), Verifier::accepting())
        .await
        .unwrap();
    assert_eq!(exec_text(&session, "echo pem").await.trim(), "pem");
}

#[tokio::test]
async fn unauthorized_key_is_auth_failed() {
    let server = server!();
    let key = generate_key(GenerateKind::Ed25519, "stranger", None).unwrap();
    let auth = AuthMethod::PrivateKey {
        openssh: key.openssh_private.clone(),
        passphrase: None,
    };
    let err = connect(server.config(auth), Verifier::accepting())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("rejected the private key"), "{err}");
}

#[tokio::test]
async fn bad_passphrase_or_missing_passphrase_is_key_parse() {
    let server = server!();
    let encrypted = fixture("ed25519_enc");
    for passphrase in [None, Some("wrong")] {
        let auth = AuthMethod::PrivateKey {
            openssh: Zeroizing::new(encrypted.clone()),
            passphrase: passphrase.map(|p| Zeroizing::new(p.to_owned())),
        };
        let err = connect(server.config(auth), Verifier::accepting())
            .await
            .unwrap_err();
        assert_eq!(err.kind, SshErrorKind::KeyParse, "{err}");
    }
    let auth = AuthMethod::PrivateKey {
        openssh: Zeroizing::new("not a key".to_owned()),
        passphrase: None,
    };
    let err = connect(server.config(auth), Verifier::accepting())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::KeyParse, "{err}");
}

#[tokio::test]
async fn password_auth() {
    let server = server!();
    let Some((user, password)) = server.password_user else {
        eprintln!("password user unavailable (not root?): skipping");
        return;
    };
    let mk = |pw: &str| {
        ConnectConfig::new(
            "127.0.0.1",
            server.port,
            user,
            AuthMethod::Password(Zeroizing::new(pw.to_owned())),
        )
    };
    let session = connect(mk(password), Verifier::accepting()).await.unwrap();
    assert_eq!(exec_text(&session, "id -un").await.trim(), user);
    session.disconnect().await;

    let err = connect(mk("wrong-password"), Verifier::accepting())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("rejected the password"), "{err}");
    assert!(
        !err.message.contains("wrong-password"),
        "no secrets in messages"
    );
}

#[tokio::test]
async fn auth_none_fails_against_real_server() {
    let server = server!();
    let err = connect(server.config(AuthMethod::None), Verifier::accepting())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
}

// --------------------------------------------------------------------------- host key

#[tokio::test]
async fn verifier_receives_host_key_details() {
    use base64::Engine as _;
    for kind in [HostKeyType::Ed25519, HostKeyType::Rsa, HostKeyType::Ecdsa] {
        let server = server!(kind);
        let verifier = Verifier::accepting();
        let session = connect(server.config(key_auth("ed25519", None)), verifier.clone())
            .await
            .unwrap();
        let calls = verifier.calls();
        assert_eq!(calls.len(), 1);
        let (host, port, info) = &calls[0];
        assert_eq!(host, "127.0.0.1");
        assert_eq!(*port, server.port);
        assert_eq!(info.key_type, server.host_key_type);
        assert_eq!(info.fingerprint, server.host_fingerprint);
        assert!(info.fingerprint.starts_with("SHA256:") && !info.fingerprint.ends_with('='));
        // The blob is what `known_hosts` / the .pub file contain after the type.
        let pub_line = std::fs::read_to_string(server.dir.path().join("host_key.pub")).unwrap();
        assert_eq!(info.public_key, pub_line.split_whitespace().nth(1).unwrap());
        assert!(
            base64::engine::general_purpose::STANDARD
                .decode(&info.public_key)
                .is_ok()
        );
        session.disconnect().await;
    }
}

#[tokio::test]
async fn rejected_host_key_aborts_the_connection() {
    let server = server!();
    let verifier = Verifier::rejecting();
    let err = connect(server.config(key_auth("ed25519", None)), verifier.clone())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::HostKeyRejected, "{err}");
    assert_eq!(verifier.calls().len(), 1);
}

#[tokio::test]
async fn slow_verifier_does_not_consume_the_connect_timeout() {
    let server = server!();
    let mut cfg = server.config(key_auth("ed25519", None));
    cfg.connect_timeout = Duration::from_millis(1500);
    // The user "thinks" for twice the connect timeout.
    let verifier = Verifier::slow(Duration::from_secs(3));
    let started = Instant::now();
    let session = connect(cfg, verifier)
        .await
        .expect("timeout must be paused during verify");
    assert!(started.elapsed() >= Duration::from_secs(3));
    assert_eq!(exec_text(&session, "echo ok").await.trim(), "ok");
}

// --------------------------------------------------------------------------- shell

#[tokio::test]
async fn shell_round_trip_resize_and_exit() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (shell, mut rx) = session
        .open_shell(ShellOptions {
            cols: 100,
            rows: 30,
            ..ShellOptions::default()
        })
        .await
        .unwrap();

    shell
        .write(b"echo hello-$((40+2))\n".to_vec())
        .await
        .unwrap();
    read_until(&mut rx, Duration::from_secs(10), |o| {
        contains(o, "hello-42")
    })
    .await;

    // The PTY has the size we asked for, and resizing updates it.
    shell.write(b"stty size\n".to_vec()).await.unwrap();
    read_until(&mut rx, Duration::from_secs(10), |o| {
        contains(o, "30 100\r\n")
    })
    .await;
    shell.resize(120, 40).await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    shell.write(b"stty size\n".to_vec()).await.unwrap();
    read_until(&mut rx, Duration::from_secs(10), |o| {
        contains(o, "40 120\r\n")
    })
    .await;

    // TERM and the requested LANG reach the shell.
    shell
        .write(b"echo T=$TERM L=$LANG\n".to_vec())
        .await
        .unwrap();
    read_until(&mut rx, Duration::from_secs(10), |o| {
        contains(o, "T=xterm-256color L=C.UTF-8")
    })
    .await;

    // UTF-8 (CJK) survives the PTY round trip.
    shell
        .write("echo 日本語-中文\n".as_bytes().to_vec())
        .await
        .unwrap();
    read_until(&mut rx, Duration::from_secs(10), |o| {
        String::from_utf8_lossy(o).matches("日本語-中文").count() >= 2
    })
    .await;

    shell.write(b"exit 3\n".to_vec()).await.unwrap();
    let mut closed = 0;
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv())
            .await
            .expect("event")
        {
            Some(ShellEvent::Data(_)) => {}
            Some(ShellEvent::Closed {
                exit_status,
                reason,
            }) => {
                closed += 1;
                assert_eq!(exit_status, Some(3), "{reason}");
            }
            Some(ShellEvent::Error(e)) => panic!("unexpected error {e}"),
            None => break,
        }
    }
    assert_eq!(closed, 1, "Closed is emitted exactly once");
    // Writing to a finished shell is an error, not a hang.
    assert!(shell.write(b"x".to_vec()).await.is_err());
    // The connection itself is still fine.
    assert_eq!(exec_text(&session, "echo alive").await.trim(), "alive");
}

#[tokio::test]
async fn shell_gets_host_env_and_ignores_refused_variables() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    // SSH-14: the test server accepts LANG and HATOBA_*; NOT_ACCEPTED is refused, which must not
    // keep the shell from starting.
    let opts = ShellOptions::default().with_env([
        ("HATOBA_GREETING".to_owned(), "hello world=1".to_owned()),
        ("NOT_ACCEPTED".to_owned(), "x".to_owned()),
        ("LANG".to_owned(), "C".to_owned()),
        ("TERM".to_owned(), "vt100".to_owned()),
    ]);
    let (shell, mut rx) = session.open_shell(opts).await.unwrap();
    shell
        .write(b"echo \"[G=$HATOBA_GREETING] [N=$NOT_ACCEPTED] [L=$LANG] [T=$TERM]\"\n".to_vec())
        .await
        .unwrap();
    read_until(&mut rx, Duration::from_secs(10), |o| {
        contains(o, "[G=hello world=1] [N=] [L=C] [T=vt100]")
    })
    .await;
    shell.close().await;
}

#[tokio::test]
async fn shell_output_is_batched_in_chunks_of_at_most_32k() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (shell, mut rx) = session.open_shell(ShellOptions::default()).await.unwrap();
    shell
        .write(b"stty -echo; seq 1 400000; echo DONE-$((1+1))\n".to_vec())
        .await
        .unwrap();

    let mut total = 0usize;
    let mut events = 0usize;
    let mut largest = 0usize;
    let mut tail = Vec::new();
    let started = Instant::now();
    loop {
        match tokio::time::timeout(Duration::from_secs(30), rx.recv())
            .await
            .expect("data")
        {
            Some(ShellEvent::Data(d)) => {
                events += 1;
                total += d.len();
                largest = largest.max(d.len());
                tail.extend_from_slice(&d);
                if tail.len() > 256 {
                    tail.drain(..tail.len() - 256);
                }
                if contains(&tail, "DONE-2") {
                    break;
                }
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    // 400000 lines ~ 2.6 MB of numbers (+ CRLF from the PTY).
    assert!(total > 2_500_000, "total {total}");
    assert!(largest <= 32 * 1024, "event of {largest} bytes");
    // Far fewer events than packets: batching works (>= 8 KB average).
    assert!(events < total / 4096, "{events} events for {total} bytes");
    eprintln!("{total} bytes in {events} events, {:?}", started.elapsed());
    shell.close().await;
}

#[tokio::test]
async fn single_keystroke_echo_is_not_delayed() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (shell, mut rx) = session.open_shell(ShellOptions::default()).await.unwrap();
    // Wait for the prompt to settle.
    tokio::time::sleep(Duration::from_millis(500)).await;
    while rx.try_recv().is_ok() {}

    let mut worst = Duration::ZERO;
    for _ in 0..5 {
        let t = Instant::now();
        shell.write(b"z".to_vec()).await.unwrap();
        read_until(&mut rx, Duration::from_secs(5), |o| contains(o, "z")).await;
        worst = worst.max(t.elapsed());
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Loopback; spec target is < 50 ms for the whole echo path.
    assert!(
        worst < Duration::from_millis(50),
        "worst echo latency {worst:?}"
    );
    shell.close().await;
}

#[tokio::test]
async fn closing_the_shell_yields_a_single_closed_event() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (shell, mut rx) = session.open_shell(ShellOptions::default()).await.unwrap();
    shell.write(b"echo up\n".to_vec()).await.unwrap();
    read_until(&mut rx, Duration::from_secs(10), |o| contains(o, "up")).await;
    shell.close().await;
    shell.close().await; // idempotent
    let mut closed = 0;
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
    {
        match ev {
            ShellEvent::Closed { .. } => closed += 1,
            ShellEvent::Data(_) => {}
            ShellEvent::Error(e) => panic!("{e}"),
        }
    }
    assert_eq!(closed, 1);
    assert_eq!(
        exec_text(&session, "echo still-connected").await.trim(),
        "still-connected"
    );
}

#[tokio::test]
async fn several_shells_and_commands_share_one_connection() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let mut shells = Vec::new();
    for i in 0..3 {
        let (shell, rx) = session.open_shell(ShellOptions::default()).await.unwrap();
        shell
            .write(format!("echo tab-{i}\n").into_bytes())
            .await
            .unwrap();
        shells.push((i, shell, rx));
    }
    let execs = futures::future::join_all((0..16).map(|i| {
        let s = session.clone();
        async move { exec_text(&s, &format!("echo n{i}")).await }
    }))
    .await;
    for (i, out) in execs.iter().enumerate() {
        assert_eq!(out.trim(), format!("n{i}"));
    }
    for (i, shell, mut rx) in shells {
        read_until(&mut rx, Duration::from_secs(10), |o| {
            contains(o, &format!("tab-{i}\r\n"))
        })
        .await;
        shell.close().await;
    }
}

#[tokio::test]
async fn disconnect_closes_shells_and_rejects_new_channels() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (_shell, mut rx) = session.open_shell(ShellOptions::default()).await.unwrap();
    assert!(!session.is_closed());
    session.disconnect().await;
    assert!(session.is_closed());

    let mut terminal = 0;
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
    {
        match ev {
            ShellEvent::Closed { .. } => terminal += 1,
            ShellEvent::Error(e) => panic!("a local disconnect is not an error: {e}"),
            ShellEvent::Data(_) => {}
        }
    }
    assert_eq!(terminal, 1);
    assert!(session.open_shell(ShellOptions::default()).await.is_err());
    assert!(session.exec("true", None).await.is_err());
    // `closed()` resolves immediately for a finished session.
    tokio::time::timeout(Duration::from_secs(1), session.closed())
        .await
        .unwrap();
}

#[tokio::test]
async fn connection_loss_yields_a_single_error_event() {
    let server = server!();
    let proxy = Proxy::start(server.port).await;
    let cfg = ConnectConfig::new(
        "127.0.0.1",
        proxy.port,
        server.key_user.clone(),
        key_auth("ed25519", None),
    );
    let session = connect(cfg, Verifier::accepting()).await.unwrap();
    let (shell, mut rx) = session.open_shell(ShellOptions::default()).await.unwrap();
    shell.write(b"echo up\n".to_vec()).await.unwrap();
    read_until(&mut rx, Duration::from_secs(10), |o| contains(o, "up")).await;

    proxy.sever();

    let mut errors = Vec::new();
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(10), rx.recv())
        .await
        .expect("event")
    {
        match ev {
            ShellEvent::Error(e) => errors.push(e),
            ShellEvent::Data(_) => {}
            ShellEvent::Closed { reason, .. } => panic!("expected an error, got Closed({reason})"),
        }
    }
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert_eq!(errors[0].kind, SshErrorKind::Disconnected);
    assert!(session.is_closed());
    // Subsequent use reports the same kind of failure.
    let err = session.exec("true", None).await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Disconnected, "{err}");
}

#[tokio::test]
async fn keepalive_detects_a_silent_peer() {
    let server = server!();
    let proxy = Proxy::start(server.port).await;
    let mut cfg = ConnectConfig::new(
        "127.0.0.1",
        proxy.port,
        server.key_user.clone(),
        key_auth("ed25519", None),
    );
    cfg.keepalive_interval = Duration::from_millis(300);
    cfg.keepalive_max = 2;
    let session = connect(cfg, Verifier::accepting()).await.unwrap();
    let (_shell, mut rx) = session.open_shell(ShellOptions::default()).await.unwrap();

    proxy.blackhole();
    let started = Instant::now();
    let mut error = None;
    while let Some(ev) = tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .expect("detection")
    {
        if let ShellEvent::Error(e) = ev {
            error = Some(e);
        }
    }
    let error = error.expect("an Error event");
    assert_eq!(error.kind, SshErrorKind::Disconnected);
    assert!(
        error.message.contains("keepalive") || error.message.contains("inactivity"),
        "{error}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "detected after {:?}",
        started.elapsed()
    );
    assert!(session.is_closed());
}

// --------------------------------------------------------------------------- exec

#[tokio::test]
async fn exec_status_stdout_stderr_and_stdin() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;

    let out = session
        .exec_output("printf 'a\\nb'; echo err >&2; exit 7", None)
        .await
        .unwrap();
    assert_eq!(out.exit_status, Some(7));
    assert_eq!(out.stdout, b"a\nb");
    assert_eq!(out.stderr, b"err\n");

    let (status, stdout) = session.exec("false", None).await.unwrap();
    assert_eq!((status, stdout.as_slice()), (1, &b""[..]));

    // stdin is delivered and closed (cat terminates).
    let (status, echoed) = session
        .exec("cat", Some(b"hello stdin".to_vec()))
        .await
        .unwrap();
    assert_eq!(status, 0);
    assert_eq!(echoed, b"hello stdin");

    // No stdin given: the command sees EOF immediately.
    let (status, echoed) = session.exec("cat; echo done", None).await.unwrap();
    assert_eq!((status, echoed.as_slice()), (0, &b"done\n"[..]));

    // A large stdin round-trips (flow control in both directions).
    let big = pseudo_random(3 * 1024 * 1024, 7);
    let (status, hash) = session
        .exec("sha256sum | cut -d' ' -f1", Some(big.clone()))
        .await
        .unwrap();
    assert_eq!(status, 0);
    assert_eq!(String::from_utf8(hash).unwrap().trim(), sha256_hex(&big));
    let (_, echoed) = session.exec("cat", Some(big.clone())).await.unwrap();
    assert_eq!(echoed.len(), big.len());
    assert_eq!(sha256_hex(&echoed), sha256_hex(&big));

    // Command that exits without reading its stdin.
    let (status, _) = session
        .exec("true", Some(pseudo_random(1024 * 1024, 3)))
        .await
        .unwrap();
    assert_eq!(status, 0);

    // ssh-copy-id style: append a key via stdin.
    let target = server.dir.path().join("copied_keys");
    let cmd = format!("cat >> '{}'", target.display());
    session
        .exec(&cmd, Some(b"ssh-ed25519 AAAA test\n".to_vec()))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "ssh-ed25519 AAAA test\n"
    );
}

/// The file tools' read and write (AI-38…40) through sshd and the user's login shell.
#[tokio::test]
async fn remote_files_are_read_and_written_in_place() {
    use std::os::unix::fs::MetadataExt;

    use hatoba_ssh::remote_file::{self, Expect, RemoteFileError};

    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let area = server.area("remote_file");
    let path = area.join("it's $HOME.conf");
    std::fs::write(&path, b"listen 80;\r\n").unwrap();
    let inode = std::fs::metadata(&path).unwrap().ino();
    let name = path.to_str().unwrap();

    let file = remote_file::read(&session, name, 1024).await.unwrap();
    assert_eq!(file.path, name);
    let read = file.bytes;
    assert_eq!(read, b"listen 80;\r\n");
    assert!(matches!(
        remote_file::read(&session, name, 4).await,
        Err(RemoteFileError::TooLarge(12))
    ));

    remote_file::write(&session, name, b"listen 443;\r\n", Expect::Content(&read))
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"listen 443;\r\n");
    assert_eq!(std::fs::metadata(&path).unwrap().ino(), inode);
    // What the first read saw is gone, so a second write against it is refused.
    assert!(matches!(
        remote_file::write(&session, name, b"x", Expect::Content(&read)).await,
        Err(RemoteFileError::Changed)
    ));

    let new = area.join("new.txt");
    let new_name = new.to_str().unwrap();
    assert!(matches!(
        remote_file::read(&session, new_name, 1024).await,
        Err(RemoteFileError::NotFound)
    ));
    remote_file::write(
        &session,
        new_name,
        &pseudo_random(256 * 1024, 5),
        Expect::Missing,
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&new).unwrap(), pseudo_random(256 * 1024, 5));
}

// --------------------------------------------------------------------------- probe & errors on a live server

#[tokio::test]
async fn tcp_probe_against_live_and_dead_ports() {
    let server = server!();
    let ms = tcp_probe("127.0.0.1", server.port, Duration::from_secs(3)).await;
    assert!(ms.is_some_and(|m| (1..2000).contains(&m)), "{ms:?}");
    assert_eq!(
        tcp_probe("127.0.0.1", free_port(), Duration::from_secs(3)).await,
        None
    );
    assert_eq!(
        tcp_probe("nonexistent.invalid", 22, Duration::from_secs(3)).await,
        None
    );
}

#[tokio::test]
async fn wrong_port_is_refused_not_auth_failed() {
    let server = server!();
    let port = free_port();
    let err = connect(
        ConnectConfig::new(
            "127.0.0.1",
            port,
            server.key_user.clone(),
            key_auth("ed25519", None),
        ),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Refused, "{err}");
}

// --------------------------------------------------------------------------- throughput & backpressure

/// §11: a 50 MB `cat` must not freeze anything, and nothing may be lost.
#[tokio::test]
async fn fifty_megabytes_of_output_arrive_complete() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (shell, mut rx) = session.open_shell(ShellOptions::default()).await.unwrap();
    shell.write(b"stty -echo -onlcr\n".to_vec()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    while rx.try_recv().is_ok() {}

    const SIZE: usize = 50 * 1000 * 1000;
    shell
        .write(
            format!("head -c {SIZE} /dev/zero | tr '\\0' 'x'; echo; echo END-$((20+22))\n")
                .into_bytes(),
        )
        .await
        .unwrap();
    let started = Instant::now();
    let mut total = 0usize;
    let mut tail = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(60), rx.recv())
            .await
            .expect("data")
        {
            Some(ShellEvent::Data(d)) => {
                total += d.len();
                assert!(d.len() <= 32 * 1024);
                tail.extend_from_slice(&d);
                if tail.len() > 256 {
                    tail.drain(..tail.len() - 256);
                }
                if contains(&tail, "END-42") {
                    break;
                }
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    let secs = started.elapsed().as_secs_f64();
    eprintln!(
        "{total} bytes in {secs:.2}s = {:.0} MB/s",
        total as f64 / 1e6 / secs
    );
    assert!((SIZE..SIZE + 64).contains(&total), "total {total}");
    shell.close().await;
}

/// §10.3 backpressure: a stalled consumer must stop the reader instead of buffering without bound.
#[tokio::test]
async fn a_stalled_consumer_applies_backpressure() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let (shell, mut rx) = session.open_shell(ShellOptions::default()).await.unwrap();
    shell.write(b"stty -echo -onlcr\n".to_vec()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    while rx.try_recv().is_ok() {}

    const SIZE: usize = 60 * 1000 * 1000;
    shell
        .write(
            format!("head -c {SIZE} /dev/zero | tr '\\0' 'y'; echo; echo END-$((20+22))\n")
                .into_bytes(),
        )
        .await
        .unwrap();

    // Do not read for a while: the bounded channel (128 events x <= 32 KB) fills up and stays full.
    tokio::time::sleep(Duration::from_millis(2000)).await;
    assert_eq!(rx.len(), 128, "channel should be full");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(rx.len(), 128, "and stay bounded");

    // Everything still arrives once the consumer wakes up.
    let mut total = 0usize;
    let mut tail = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(60), rx.recv())
            .await
            .expect("data")
        {
            Some(ShellEvent::Data(d)) => {
                total += d.len();
                tail.extend_from_slice(&d);
                if tail.len() > 256 {
                    tail.drain(..tail.len() - 256);
                }
                if contains(&tail, "END-42") {
                    break;
                }
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!((SIZE..SIZE + 64).contains(&total), "total {total}");
    // The session stayed healthy while stalled.
    assert_eq!(exec_text(&session, "echo fine").await.trim(), "fine");
    shell.close().await;
}

#[tokio::test]
async fn sftp_throughput_is_reasonable() {
    let server = server!();
    let session = server.connect_key("ed25519", None).await;
    let sftp = session.sftp().await.unwrap();
    let area = server.area("speed");
    let local = tempfile::tempdir().unwrap();
    let src = local.path().join("big");
    std::fs::write(&src, pseudo_random(64 * 1024 * 1024, 9)).unwrap();
    let remote = area.join("big");

    let t = Instant::now();
    sftp.upload(&src, remote.to_str().unwrap(), |_| {}, Default::default())
        .await
        .unwrap();
    let up = 64.0 / t.elapsed().as_secs_f64();
    let t = Instant::now();
    let back = local.path().join("back");
    sftp.download(remote.to_str().unwrap(), &back, |_| {}, Default::default())
        .await
        .unwrap();
    let down = 64.0 / t.elapsed().as_secs_f64();
    eprintln!("sftp upload {up:.0} MB/s, download {down:.0} MB/s");
    assert_eq!(std::fs::metadata(&back).unwrap().len(), 64 * 1024 * 1024);
    // Loopback; a single-digit MB/s result would mean the pipelining is broken.
    assert!(
        up > 10.0 && down > 10.0,
        "up {up:.1} MB/s, down {down:.1} MB/s"
    );
}
