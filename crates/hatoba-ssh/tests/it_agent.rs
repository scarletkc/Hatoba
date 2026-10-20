//! ssh-agent authentication (SSH-09) against a real `ssh-agent` and `sshd` (set `HATOBA_SSH_IT=1`).
//!
//! One test function on purpose: it changes `SSH_AUTH_SOCK` for the whole process.
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use common::*;
use hatoba_ssh::{AuthMethod, SshErrorKind, connect};

struct Agent {
    child: Child,
    sock: std::path::PathBuf,
}

impl Agent {
    fn start(dir: &std::path::Path) -> Self {
        let sock = dir.join("agent.sock");
        let child = Command::new("ssh-agent")
            .args(["-D", "-a"])
            .arg(&sock)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("ssh-agent");
        for _ in 0..100 {
            if sock.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(sock.exists(), "agent socket did not appear");
        Self { child, sock }
    }

    fn add(&self, dir: &std::path::Path, fixture_name: &str, passphrase: Option<&str>) {
        let file = dir.join(format!("agent_{fixture_name}"));
        fs::write(&file, fixture(fixture_name)).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let mut cmd = Command::new("ssh-add");
        cmd.env("SSH_AUTH_SOCK", &self.sock)
            .arg(&file)
            .stdin(Stdio::null());
        if let Some(passphrase) = passphrase {
            // Feed the passphrase through SSH_ASKPASS.
            let askpass = dir.join("askpass.sh");
            fs::write(&askpass, format!("#!/bin/sh\necho '{passphrase}'\n")).unwrap();
            fs::set_permissions(&askpass, fs::Permissions::from_mode(0o700)).unwrap();
            cmd.env("SSH_ASKPASS", &askpass)
                .env("SSH_ASKPASS_REQUIRE", "force")
                .env("DISPLAY", ":0");
        }
        let out = cmd.output().expect("ssh-add");
        assert!(
            out.status.success(),
            "ssh-add {fixture_name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn clear(&self) {
        let status = Command::new("ssh-add")
            .env("SSH_AUTH_SOCK", &self.sock)
            .arg("-D")
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn agent_authentication() {
    let Some(server) = SshdServer::start() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let agent = Agent::start(dir.path());
    // SAFETY: this is the only test in the binary, so nothing else touches the environment
    // concurrently.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", &agent.sock) };

    let login = |label: &str| {
        let label = label.to_owned();
        let cfg = server.config(AuthMethod::Agent);
        async move {
            let r = connect(cfg, Verifier::accepting()).await;
            eprintln!(
                "agent login ({label}): {}",
                if r.is_ok() {
                    "ok".into()
                } else {
                    format!("{}", r.as_ref().err().unwrap())
                }
            );
            r
        }
    };

    // Empty agent.
    let err = login("empty").await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("no identities"), "{err}");

    // Only an identity the server does not know.
    let stranger = hatoba_ssh::generate_key(hatoba_ssh::GenerateKind::Ed25519, "x", None).unwrap();
    let stranger_file = dir.path().join("stranger");
    fs::write(&stranger_file, stranger.openssh_private.as_bytes()).unwrap();
    fs::set_permissions(&stranger_file, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(
        Command::new("ssh-add")
            .env("SSH_AUTH_SOCK", &agent.sock)
            .arg(&stranger_file)
            .stderr(Stdio::null())
            .status()
            .unwrap()
            .success()
    );
    let err = login("stranger only").await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("ssh-agent identities"), "{err}");

    // The stranger first, then a good ed25519 key: every identity is tried in turn.
    agent.add(dir.path(), "ed25519", None);
    let session = login("stranger + ed25519")
        .await
        .expect("second identity is accepted");
    assert_eq!(
        session.exec("id -un", None).await.unwrap().1,
        format!("{}\n", server.key_user).as_bytes()
    );
    session.disconnect().await;

    // RSA through the agent (rsa-sha2 signatures).
    agent.clear();
    agent.add(dir.path(), "rsa", None);
    let session = login("rsa").await.expect("rsa via agent");
    assert_eq!(
        session.exec("echo rsa-ok", None).await.unwrap().1,
        b"rsa-ok\n"
    );
    session.disconnect().await;

    // ECDSA with a passphrase-protected source key added via askpass.
    agent.clear();
    agent.add(dir.path(), "ecdsa256_enc", Some(PASS));
    login("ecdsa")
        .await
        .expect("ecdsa via agent")
        .disconnect()
        .await;

    // Agent not reachable: a precise error, not an auth failure.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", dir.path().join("nonexistent.sock")) };
    let err = login("missing socket").await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Other, "{err}");
    assert!(err.message.contains("ssh-agent"), "{err}");
    unsafe { std::env::remove_var("SSH_AUTH_SOCK") };
    let err = login("unset").await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Other, "{err}");
    assert!(err.message.contains("ssh-agent"), "{err}");
}
