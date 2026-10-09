//! `AuthMethod::AgentThenAsk` (quick connect, HOST-12) with a running `ssh-agent` whose keys the
//! scripted server rejects. Needs `ssh-agent` and `ssh-add`; without them the test says so and
//! passes.
//!
//! One test function on purpose: it changes `SSH_AUTH_SOCK` for the whole process.
#![cfg(unix)]

mod common;

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use common::ask_server::*;
use common::*;
use hatoba_ssh::{AuthMethod, connect};
use russh::MethodKind;

/// Agent keys the server is offered, and rejects.
const AGENT_KEYS: [&str; 3] = ["ed25519", "ecdsa256", "rsa"];

struct Agent(Child);

impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn agent_identities_then_password() {
    let dir = tempfile::tempdir().unwrap();
    let sock = dir.path().join("agent.sock");
    let child = match Command::new("ssh-agent")
        .args(["-D", "-a"])
        .arg(&sock)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) => {
            eprintln!("skipping: ssh-agent is not available ({e})");
            return;
        }
    };
    let _agent = Agent(child);
    for _ in 0..100 {
        if sock.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(sock.exists(), "agent socket did not appear");
    for name in AGENT_KEYS {
        let file = dir.path().join(name);
        fs::write(&file, fixture(name)).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
        let out = Command::new("ssh-add")
            .env("SSH_AUTH_SOCK", &sock)
            .arg(&file)
            .stdin(Stdio::null())
            .output()
            .expect("ssh-add");
        assert!(
            out.status.success(),
            "ssh-add {name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // SAFETY: the only test in this binary, set before anything else reads the environment.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", &sock) };
    let methods = [MethodKind::PublicKey, MethodKind::Password];

    // Every identity is tried and rejected, then the password is asked on the same connection.
    let server = start_ask_server(AskScript::offering(&methods)).await;
    let cb = AskCallback::new(Some(ASK_PASSWORD));
    let session = connect(
        ask_config(server.port, AuthMethod::AgentThenAsk, &cb),
        Verifier::accepting(),
    )
    .await
    .expect("password after the agent identities");
    assert_eq!(server.keys_offered(), AGENT_KEYS.len());
    assert_eq!(server.connections(), 1);
    assert_eq!(cb.asked().len(), 1);
    assert_eq!(server.received(), vec![ASK_PASSWORD.to_owned()]);
    session.disconnect().await;

    // A server that hangs up after two rejections (like OpenSSH's MaxAuthTries) is connected
    // to again without the agent, and the password is asked once.
    let server = start_ask_server(AskScript {
        max_auth_attempts: 2,
        ..AskScript::offering(&methods)
    })
    .await;
    let cb = AskCallback::new(Some(ASK_PASSWORD));
    let verifier = Verifier::accepting();
    let session = connect(
        ask_config(server.port, AuthMethod::AgentThenAsk, &cb),
        verifier.clone(),
    )
    .await
    .expect("password on a second connection");
    assert_eq!(server.connections(), 2);
    assert_eq!(verifier.calls().len(), 2);
    assert_eq!(cb.asked().len(), 1);
    assert_eq!(server.received(), vec![ASK_PASSWORD.to_owned()]);
    session.disconnect().await;
}
