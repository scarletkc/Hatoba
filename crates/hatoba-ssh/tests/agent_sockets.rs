//! ssh-agent discovery (SSH-09) against a real `ssh-agent` listening where Bitwarden's desktop
//! app listens: `<home>/.bitwarden-ssh-agent.sock`, with `SSH_AUTH_SOCK` unset.
//!
//! This is the case a window opened from Finder or a desktop launcher is in: it never reads the
//! `~/.zshrc` that exports `SSH_AUTH_SOCK`, so before the discovery existed the agent looked
//! absent. The scripted server of `common` rejects public keys, which is enough to see an
//! identity being offered. No `sshd` is needed, so the test runs wherever `ssh-agent` and
//! `ssh-add` are.
//!
//! One test function on purpose: it changes `HOME` and `SSH_AUTH_SOCK` for the whole process.
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use common::ask_server::*;
use common::*;
use hatoba_ssh::{AgentKind, AuthMethod, SshErrorKind, agent_sockets, agent_status, connect};
use russh::MethodKind;

struct Agent {
    child: Child,
    sock: std::path::PathBuf,
}

impl Agent {
    /// Starts `ssh-agent` on `sock`, or returns `None` when the machine has no `ssh-agent`.
    fn start(sock: &std::path::Path) -> Option<Self> {
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

    fn add(&self, path: &std::path::Path) {
        let out = Command::new("ssh-add")
            .env("SSH_AUTH_SOCK", &self.sock)
            .arg(path)
            .stdin(Stdio::null())
            .output()
            .expect("ssh-add");
        assert!(
            out.status.success(),
            "ssh-add: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
async fn the_bitwarden_socket_is_found_without_ssh_auth_sock() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    fs::create_dir_all(&home).unwrap();
    let sock = home.join(".bitwarden-ssh-agent.sock");
    let Some(agent) = Agent::start(&sock) else {
        eprintln!("skipped: no ssh-agent on this machine");
        return;
    };
    // SAFETY: this is the only test in the binary, so nothing else reads the environment
    // concurrently. `HOME` is the temporary directory, which keeps the machine's own agents out.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::remove_var("SSH_AUTH_SOCK");
        std::env::remove_var("XDG_RUNTIME_DIR");
        std::env::remove_var("GNUPGHOME");
    }

    // The socket is offered, named after the agent that listens on it, and answers.
    let sockets = agent_sockets(None);
    assert_eq!(sockets.len(), 1, "{sockets:?}");
    assert_eq!(sockets[0].path, sock);
    assert_eq!(sockets[0].kind, AgentKind::Bitwarden);

    let status = agent_status(None).await.expect("the agent answers");
    assert_eq!(status.socket.path, sock);
    assert!(status.identities.is_empty(), "a fresh agent holds no keys");

    // A key added to it is listed with its type, fingerprint and comment.
    let key =
        hatoba_ssh::generate_key(hatoba_ssh::GenerateKind::Ed25519, "bitwarden-key", None).unwrap();
    let key_file = home.join("id_ed25519");
    fs::write(&key_file, key.openssh_private.as_bytes()).unwrap();
    fs::set_permissions(&key_file, fs::Permissions::from_mode(0o600)).unwrap();
    agent.add(&key_file);

    let status = agent_status(None).await.expect("the agent answers");
    assert_eq!(status.identities.len(), 1);
    assert_eq!(status.identities[0].algorithm, "ssh-ed25519");
    assert_eq!(status.identities[0].fingerprint, key.fingerprint);
    assert_eq!(status.identities[0].comment, "bitwarden-key");

    // The identity reaches the authentication path over the socket the device found.
    let server = start_ask_server(AskScript::offering(&[MethodKind::PublicKey])).await;
    let cb = AskCallback::new(None);
    let mut cfg = ask_config(server.port, AuthMethod::Agent, &cb);
    cfg.agent_socket = Some(sock.to_string_lossy().into_owned());
    let err = connect(cfg, Verifier::accepting()).await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(
        err.message.contains("none of the 1 ssh-agent identities"),
        "{err}"
    );
    assert_eq!(server.keys_offered(), 1);

    // A socket the user chose is used alone: the agent that is running is not taken instead.
    let chosen = dir.path().join("elsewhere.sock");
    let err = agent_status(Some(&chosen.to_string_lossy()))
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Other, "{err}");
    assert!(err.message.contains("elsewhere.sock"), "{err}");

    let mut cfg = ask_config(server.port, AuthMethod::Agent, &cb);
    cfg.agent_socket = Some(chosen.to_string_lossy().into_owned());
    let err = connect(cfg, Verifier::accepting()).await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Other, "{err}");
    assert!(err.message.contains("elsewhere.sock"), "{err}");

    // A window macOS started from Finder gets the launchd placeholder in `SSH_AUTH_SOCK`: it is
    // neither a choice nor an agent, so the search goes on and Bitwarden's socket is found.
    // SAFETY: as above; the only test in this binary.
    unsafe {
        std::env::set_var(
            "SSH_AUTH_SOCK",
            "/var/run/com.apple.launchd.hatoba-test/Listeners",
        )
    };
    let status = agent_status(None).await.expect("an agent holds the key");
    assert_eq!(status.socket.path, sock, "the placeholder hid Bitwarden's");
    assert_eq!(status.identities.len(), 1);

    // The identity found that way reaches the authentication path with no socket named.
    let mut cfg = ask_config(server.port, AuthMethod::Agent, &cb);
    cfg.agent_socket = None;
    let err = connect(cfg, Verifier::accepting()).await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(
        err.message.contains("none of the 1 ssh-agent identities"),
        "{err}"
    );

    // A socket this session sets is honoured alone: a path that stopped working fails with that
    // path named, and the agent that is running is not taken behind the user's back.
    let stale = home.join("gone.sock");
    // SAFETY: as above.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", &stale) };
    let sockets = agent_sockets(None);
    assert_eq!(sockets.len(), 1, "{sockets:?}");
    assert_eq!(sockets[0].path, stale);
    let err = agent_status(None).await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Other, "{err}");
    assert!(err.message.contains("gone.sock"), "{err}");

    // Which is the answer for a socket that accepts connections without being an agent as well.
    let fake = home.join(".not-an-agent.sock");
    let listener = tokio::net::UnixListener::bind(&fake).unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            drop(stream);
        }
    });
    // SAFETY: as above.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", &fake) };
    let err = agent_status(None).await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Other, "{err}");
    assert!(err.message.contains(".not-an-agent.sock"), "{err}");

    // `SSH_AUTH_SOCK` still wins when the shell exports it, as it does for the `ssh` command.
    let env_sock = home.join("agent.sock");
    fs::rename(&sock, &env_sock).unwrap();
    // SAFETY: as above.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", &env_sock) };
    let sockets = agent_sockets(None);
    assert_eq!(sockets[0].path, env_sock);
    assert_eq!(sockets[0].kind, AgentKind::System);
    let status = agent_status(None).await.expect("the agent answers");
    assert_eq!(status.identities.len(), 1);
}
