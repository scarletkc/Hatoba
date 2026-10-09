//! `AuthMethod::AgentThenAsk` (quick connect, HOST-12) against a scripted in-process SSH server,
//! on a machine without an ssh-agent. `agent_then_ask_agent.rs` covers a running agent.
//!
//! One test function on purpose: it points `SSH_AUTH_SOCK` at a missing agent for the whole
//! process, so the agent step is skipped the way it is on a machine without an agent.

mod common;

use common::ask_server::*;
use common::*;
use hatoba_ssh::{AuthMethod, Prompt, SshErrorKind, connect};
use russh::MethodKind;

#[tokio::test]
async fn agent_then_ask() {
    let missing_agent = if cfg!(windows) {
        r"\\.\pipe\hatoba-test-no-agent"
    } else {
        "/nonexistent/hatoba-test-agent.sock"
    };
    // SAFETY: the only test in this binary, set before anything else reads the environment.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", missing_agent) };
    let config = |port, cb| ask_config(port, AuthMethod::AgentThenAsk, cb);

    // Password authentication only: the callback is asked for the login password, once.
    let server = start_ask_server(AskScript::offering(&[
        MethodKind::PublicKey,
        MethodKind::Password,
    ]))
    .await;
    let cb = AskCallback::new(Some(ASK_PASSWORD));
    let session = connect(config(server.port, &cb), Verifier::accepting())
        .await
        .expect("password asked for and accepted");
    let asked = cb.asked();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].password);
    assert_eq!(
        (
            asked[0].host.as_str(),
            asked[0].port,
            asked[0].username.as_str()
        ),
        ("127.0.0.1", server.port, "alice")
    );
    assert_eq!(
        asked[0].prompts,
        vec![Prompt {
            text: "Password: ".into(),
            echo: false
        }]
    );
    assert_eq!(server.received(), vec![ASK_PASSWORD.to_owned()]);
    session.disconnect().await;

    // A wrong password fails the connection without asking again.
    let cb = AskCallback::new(Some("wrong"));
    let err = connect(config(server.port, &cb), Verifier::accepting())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("rejected the password"), "{err}");
    assert!(
        !err.message.contains("wrong"),
        "no secrets in messages: {err}"
    );
    assert_eq!(cb.asked().len(), 1);

    // Cancelling the prompt cancels the connection.
    let cb = AskCallback::new(None);
    let err = connect(config(server.port, &cb), Verifier::accepting())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Cancelled, "{err}");

    // keyboard-interactive comes first: the server's own prompt reaches the callback.
    let server = start_ask_server(AskScript::offering(&[
        MethodKind::PublicKey,
        MethodKind::KeyboardInteractive,
        MethodKind::Password,
    ]))
    .await;
    let cb = AskCallback::new(Some(ASK_PASSWORD));
    let session = connect(config(server.port, &cb), Verifier::accepting())
        .await
        .expect("keyboard-interactive login");
    let asked = cb.asked();
    assert_eq!(asked.len(), 1);
    assert!(!asked[0].password);
    assert_eq!(asked[0].name, "pam");
    assert_eq!(server.received(), vec![ASK_PASSWORD.to_owned()]);
    session.disconnect().await;

    // When keyboard-interactive fails, the password is asked for if the server takes one.
    let server = start_ask_server(AskScript {
        ki_accepts: false,
        ..AskScript::offering(&[
            MethodKind::PublicKey,
            MethodKind::KeyboardInteractive,
            MethodKind::Password,
        ])
    })
    .await;
    let cb = AskCallback::new(Some(ASK_PASSWORD));
    let session = connect(config(server.port, &cb), Verifier::accepting())
        .await
        .expect("password after keyboard-interactive failed");
    let asked: Vec<bool> = cb.asked().iter().map(|r| r.password).collect();
    assert_eq!(asked, vec![false, true]);
    assert_eq!(server.connections(), 1);
    session.disconnect().await;

    // Public keys only: nothing to ask, the failure names what the server accepts.
    let server = start_ask_server(AskScript::offering(&[MethodKind::PublicKey])).await;
    let cb = AskCallback::new(Some(ASK_PASSWORD));
    let err = connect(config(server.port, &cb), Verifier::accepting())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("publickey"), "{err}");
    assert!(cb.asked().is_empty());
}
