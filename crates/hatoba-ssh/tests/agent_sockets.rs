//! Agent authentication over the sockets a hop is given (SSH-09), against real `ssh-agent`s.
//!
//! The desktop app gives a hop the `IdentityAgent` of `~/.ssh/config`, or the login shell's
//! `SSH_AUTH_SOCK` followed by the inherited one. The scripted server of `common` rejects public
//! keys, which is enough to see which agent's identities were offered. No `sshd` is needed, so
//! the tests run wherever `ssh-agent` and `ssh-add` are.
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use common::ask_server::*;
use common::*;
use hatoba_ssh::{AuthMethod, SshError, SshErrorKind, connect};
use russh::MethodKind;

struct Agent {
    child: Child,
    sock: PathBuf,
}

impl Agent {
    /// Starts `ssh-agent` on `sock`, or returns `None` when the machine has no `ssh-agent`.
    fn start(sock: &Path) -> Option<Self> {
        let child = Command::new("ssh-agent")
            .args(["-D", "-a"])
            .arg(sock)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        for _ in 0..100 {
            if sock.exists() {
                return Some(Self {
                    child,
                    sock: sock.to_owned(),
                });
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the agent socket did not appear");
    }

    /// Adds a new Ed25519 key with `comment` to the agent.
    fn add_key(&self, dir: &Path, comment: &str) {
        let key =
            hatoba_ssh::generate_key(hatoba_ssh::GenerateKind::Ed25519, comment, None).unwrap();
        let key_file = dir.join(comment);
        fs::write(&key_file, key.openssh_private.as_bytes()).unwrap();
        fs::set_permissions(&key_file, fs::Permissions::from_mode(0o600)).unwrap();
        let out = Command::new("ssh-add")
            .env("SSH_AUTH_SOCK", &self.sock)
            .arg(&key_file)
            .stdin(Stdio::null())
            .output()
            .expect("ssh-add");
        assert!(
            out.status.success(),
            "ssh-add: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn path(&self) -> String {
        self.sock.to_string_lossy().into_owned()
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Signs in with the agent over `sockets` to a server that rejects every key.
async fn sign_in(sockets: Vec<String>) -> (SshError, usize) {
    let server = start_ask_server(AskScript::offering(&[MethodKind::PublicKey])).await;
    let cb = AskCallback::new(None);
    let mut cfg = ask_config(server.port, AuthMethod::Agent, &cb);
    cfg.agent_sockets = sockets;
    let err = connect(cfg, Verifier::accepting()).await.unwrap_err();
    (err, server.keys_offered())
}

#[tokio::test]
async fn the_agent_that_holds_keys_signs_in() {
    let dir = tempfile::tempdir().unwrap();
    let (Some(empty), Some(full)) = (
        Agent::start(&dir.path().join("empty.sock")),
        Agent::start(&dir.path().join("full.sock")),
    ) else {
        eprintln!("skipped: no ssh-agent on this machine");
        return;
    };
    full.add_key(dir.path(), "full-key");

    // A given socket is used as it is.
    let (err, offered) = sign_in(vec![full.path()]).await;
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(
        err.message.contains("none of the 1 ssh-agent identities"),
        "{err}"
    );
    assert_eq!(offered, 1);

    // An agent without keys, such as one the login shell started for itself, gives way to the
    // next socket when that one holds keys.
    let (err, offered) = sign_in(vec![empty.path(), full.path()]).await;
    assert!(
        err.message.contains("none of the 1 ssh-agent identities"),
        "{err}"
    );
    assert_eq!(offered, 1);

    // When no agent holds keys, the first that answered is the one reported.
    let (err, offered) = sign_in(vec![empty.path()]).await;
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("holds no identities"), "{err}");
    assert_eq!(offered, 0);
}

#[tokio::test]
async fn a_socket_that_does_not_answer_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let Some(agent) = Agent::start(&dir.path().join("agent.sock")) else {
        eprintln!("skipped: no ssh-agent on this machine");
        return;
    };
    agent.add_key(dir.path(), "agent-key");

    // A socket that accepts connections without being an agent is passed over.
    let fake = dir.path().join("fake.sock");
    let listener = tokio::net::UnixListener::bind(&fake).unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });
    let fake = fake.to_string_lossy().into_owned();
    let (err, offered) = sign_in(vec![fake.clone(), agent.path()]).await;
    assert!(
        err.message.contains("none of the 1 ssh-agent identities"),
        "{err}"
    );
    assert_eq!(offered, 1);

    // With nothing answering, the error names each socket, so a stale `IdentityAgent` shows.
    let gone = dir.path().join("gone.sock").to_string_lossy().into_owned();
    let (err, offered) = sign_in(vec![gone.clone()]).await;
    assert_eq!(err.kind, SshErrorKind::Other, "{err}");
    assert!(
        err.message.contains("ssh-agent is not available at"),
        "{err}"
    );
    assert!(err.message.contains(&gone), "{err}");
    assert_eq!(offered, 0);

    let (err, _) = sign_in(vec![gone.clone(), fake.clone()]).await;
    assert_eq!(err.kind, SshErrorKind::Other, "{err}");
    assert!(
        err.message.contains(&gone) && err.message.contains(&fake),
        "{err}"
    );
}
