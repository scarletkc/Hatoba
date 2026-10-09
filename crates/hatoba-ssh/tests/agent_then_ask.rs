//! `AuthMethod::AgentThenAsk` (quick connect, HOST-11) against a scripted in-process SSH server.
//!
//! One test function on purpose: it points `SSH_AUTH_SOCK` at a missing agent for the whole
//! process, so the agent step is skipped the way it is on a machine without an agent.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use common::*;
use hatoba_ssh::{
    AuthMethod, ConnectConfig, KeyboardInteractive, Prompt, PromptRequest, SshErrorKind, connect,
};
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{Auth, Config, Handler, Response};
use russh::{MethodKind, MethodSet};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

const PASSWORD: &str = "s3cret";

struct MockHandler {
    received: Arc<Mutex<Vec<String>>>,
    round: usize,
}

impl Handler for MockHandler {
    type Error = russh::Error;

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.received.lock().unwrap().push(password.to_owned());
        Ok(if password == PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let round = self.round;
        self.round += 1;
        if round == 0 {
            return Ok(Auth::Partial {
                name: "pam".into(),
                instructions: "".into(),
                prompts: vec![("Password: ".into(), false)].into(),
            });
        }
        let answer = response
            .and_then(|mut r| r.next())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default();
        self.received.lock().unwrap().push(answer.clone());
        Ok(if answer == PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }
}

/// A server offering `methods`; returns its port and the secrets it received.
async fn start_mock(methods: &[MethodKind]) -> (u16, Arc<Mutex<Vec<String>>>) {
    let key = PrivateKey::random(&mut russh::keys::key::safe_rng(), Algorithm::Ed25519).unwrap();
    let config = Arc::new(Config {
        keys: vec![key],
        methods: MethodSet::from(methods),
        auth_rejection_time: Duration::from_millis(20),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Config::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = Arc::new(Mutex::new(Vec::new()));
    let shared = received.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let handler = MockHandler {
                received: shared.clone(),
                round: 0,
            };
            let config = config.clone();
            tokio::spawn(async move {
                if let Ok(running) = russh::server::run_stream(config, stream, handler).await {
                    let _ = running.await;
                }
            });
        }
    });
    (port, received)
}

/// Answers with `reply` (or cancels with `None`) and records what it was asked.
struct Callback {
    reply: Option<&'static str>,
    asked: Mutex<Vec<PromptRequest>>,
}

impl Callback {
    fn new(reply: Option<&'static str>) -> Arc<Self> {
        Arc::new(Self {
            reply,
            asked: Mutex::new(Vec::new()),
        })
    }

    fn asked(&self) -> Vec<PromptRequest> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl KeyboardInteractive for Callback {
    async fn respond(&self, request: PromptRequest) -> Option<Vec<Zeroizing<String>>> {
        let count = request.prompts.len();
        self.asked.lock().unwrap().push(request);
        self.reply
            .map(|r| vec![Zeroizing::new(r.to_owned()); count])
    }
}

fn config(port: u16, cb: &Arc<Callback>) -> ConnectConfig {
    let mut c = ConnectConfig::new("127.0.0.1", port, "alice", AuthMethod::AgentThenAsk);
    c.keyboard_interactive = Some(cb.clone() as Arc<dyn KeyboardInteractive>);
    c
}

#[tokio::test]
async fn agent_then_ask() {
    let missing_agent = if cfg!(windows) {
        r"\\.\pipe\hatoba-test-no-agent"
    } else {
        "/nonexistent/hatoba-test-agent.sock"
    };
    // SAFETY: the only test in this binary, set before anything else reads the environment.
    unsafe { std::env::set_var("SSH_AUTH_SOCK", missing_agent) };

    // Password authentication only: the callback is asked for the login password, once.
    let (port, received) = start_mock(&[MethodKind::PublicKey, MethodKind::Password]).await;
    let cb = Callback::new(Some(PASSWORD));
    let session = connect(config(port, &cb), Verifier::accepting())
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
        ("127.0.0.1", port, "alice")
    );
    assert_eq!(
        asked[0].prompts,
        vec![Prompt {
            text: "Password: ".into(),
            echo: false
        }]
    );
    assert_eq!(*received.lock().unwrap(), vec![PASSWORD.to_owned()]);
    session.disconnect().await;

    // A wrong password fails the connection without asking again.
    let cb = Callback::new(Some("wrong"));
    let err = connect(config(port, &cb), Verifier::accepting())
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
    let cb = Callback::new(None);
    let err = connect(config(port, &cb), Verifier::accepting())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Cancelled, "{err}");

    // keyboard-interactive is preferred: the server's own prompt reaches the callback.
    let (port, received) = start_mock(&[
        MethodKind::PublicKey,
        MethodKind::KeyboardInteractive,
        MethodKind::Password,
    ])
    .await;
    let cb = Callback::new(Some(PASSWORD));
    let session = connect(config(port, &cb), Verifier::accepting())
        .await
        .expect("keyboard-interactive login");
    let asked = cb.asked();
    assert_eq!(asked.len(), 1);
    assert!(!asked[0].password);
    assert_eq!(asked[0].name, "pam");
    assert_eq!(*received.lock().unwrap(), vec![PASSWORD.to_owned()]);
    session.disconnect().await;

    // Public keys only: nothing to ask, the failure names what the server accepts.
    let (port, _) = start_mock(&[MethodKind::PublicKey]).await;
    let cb = Callback::new(Some(PASSWORD));
    let err = connect(config(port, &cb), Verifier::accepting())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("publickey"), "{err}");
    assert!(cb.asked().is_empty());
}
