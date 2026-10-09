//! A scripted in-process SSH server and prompt callback for `AuthMethod::AgentThenAsk` and
//! `AuthMethod::Ask` (quick connect, HOST-12). Public keys are always rejected.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use hatoba_ssh::{AuthMethod, ConnectConfig, KeyboardInteractive, PromptRequest};
use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{Auth, Config, Handler, Response};
use russh::{MethodKind, MethodSet};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

/// The password the server accepts, by `password` or keyboard-interactive.
pub const ASK_PASSWORD: &str = "s3cret";

/// What the server offers and how it answers.
pub struct AskScript {
    pub methods: Vec<MethodKind>,
    /// keyboard-interactive accepts [`ASK_PASSWORD`]; otherwise it rejects every answer.
    pub ki_accepts: bool,
    /// Rejections before the server hangs up (russh's `max_auth_attempts`).
    pub max_auth_attempts: usize,
}

impl AskScript {
    pub fn offering(methods: &[MethodKind]) -> Self {
        Self {
            methods: methods.to_vec(),
            ki_accepts: true,
            max_auth_attempts: 10,
        }
    }
}

pub struct AskServer {
    pub port: u16,
    /// Every secret the server received, in order.
    pub received: Arc<Mutex<Vec<String>>>,
    /// Public keys offered to the server.
    pub keys_offered: Arc<AtomicUsize>,
    /// TCP connections accepted.
    pub connections: Arc<AtomicUsize>,
}

impl AskServer {
    pub fn received(&self) -> Vec<String> {
        self.received.lock().unwrap().clone()
    }

    pub fn connections(&self) -> usize {
        self.connections.load(Ordering::SeqCst)
    }

    pub fn keys_offered(&self) -> usize {
        self.keys_offered.load(Ordering::SeqCst)
    }
}

struct AskHandler {
    ki_accepts: bool,
    received: Arc<Mutex<Vec<String>>>,
    keys_offered: Arc<AtomicUsize>,
    round: usize,
}

impl Handler for AskHandler {
    type Error = russh::Error;

    async fn auth_publickey(&mut self, _user: &str, _key: &PublicKey) -> Result<Auth, Self::Error> {
        self.keys_offered.fetch_add(1, Ordering::SeqCst);
        Ok(Auth::reject())
    }

    async fn auth_password(&mut self, _user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.received.lock().unwrap().push(password.to_owned());
        Ok(if password == ASK_PASSWORD {
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
        Ok(if self.ki_accepts && answer == ASK_PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }
}

pub async fn start_ask_server(script: AskScript) -> AskServer {
    let key = PrivateKey::random(&mut russh::keys::key::safe_rng(), Algorithm::Ed25519).unwrap();
    let config = Arc::new(Config {
        keys: vec![key],
        methods: MethodSet::from(&script.methods[..]),
        max_auth_attempts: script.max_auth_attempts,
        auth_rejection_time: Duration::from_millis(20),
        auth_rejection_time_initial: Some(Duration::ZERO),
        ..Config::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = Arc::new(Mutex::new(Vec::new()));
    let keys_offered = Arc::new(AtomicUsize::new(0));
    let connections = Arc::new(AtomicUsize::new(0));
    let (shared, offered, count) = (received.clone(), keys_offered.clone(), connections.clone());
    let ki_accepts = script.ki_accepts;
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            count.fetch_add(1, Ordering::SeqCst);
            let handler = AskHandler {
                ki_accepts,
                received: shared.clone(),
                keys_offered: offered.clone(),
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
    AskServer {
        port,
        received,
        keys_offered,
        connections,
    }
}

/// Answers every prompt with `reply` (or cancels with `None`) and records what it was asked.
pub struct AskCallback {
    reply: Option<&'static str>,
    asked: Mutex<Vec<PromptRequest>>,
}

impl AskCallback {
    pub fn new(reply: Option<&'static str>) -> Arc<Self> {
        Arc::new(Self {
            reply,
            asked: Mutex::new(Vec::new()),
        })
    }

    pub fn asked(&self) -> Vec<PromptRequest> {
        self.asked.lock().unwrap().clone()
    }
}

#[async_trait]
impl KeyboardInteractive for AskCallback {
    async fn respond(&self, request: PromptRequest) -> Option<Vec<Zeroizing<String>>> {
        let count = request.prompts.len();
        self.asked.lock().unwrap().push(request);
        self.reply
            .map(|r| vec![Zeroizing::new(r.to_owned()); count])
    }
}

pub fn ask_config(port: u16, auth: AuthMethod, cb: &Arc<AskCallback>) -> ConnectConfig {
    let mut c = ConnectConfig::new("127.0.0.1", port, "alice", auth);
    c.keyboard_interactive = Some(cb.clone() as Arc<dyn KeyboardInteractive>);
    c
}
