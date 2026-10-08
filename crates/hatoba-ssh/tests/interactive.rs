//! keyboard-interactive authentication (SSH-08) against a scripted in-process SSH server.
//! No sshd needed: the server side is russh's own server implementation.

mod common;

use std::borrow::Cow;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use common::*;
use hatoba_ssh::{
    AuthMethod, ConnectConfig, KeyboardInteractive, Prompt, PromptRequest, SshErrorKind, connect,
};
use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{Auth, Config, Handler, Response};
use russh::{MethodKind, MethodSet, SshId};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

/// What the scripted server asks for.
#[derive(Clone)]
enum Script {
    /// One "Password: " prompt; the right answer is accepted.
    Password(&'static str),
    /// "Password: " (kept re-prompting when wrong, like PAM) up to `tries` times.
    PasswordWithRetries(&'static str, usize),
    /// Password round, then a "Verification code: " round.
    PasswordThenOtp(&'static str, &'static str),
    /// A single round with two prompts at once.
    TwoPromptsAtOnce(&'static str, &'static str),
    /// An informational round without prompts, then a password round.
    InfoThenPassword(&'static str),
    /// Endless informational rounds.
    Endless,
    /// Public key (the given one) first, then a code round (AuthenticationMethods publickey,keyboard-interactive).
    PublicKeyThenOtp(PublicKey, &'static str),
}

struct MockHandler {
    script: Script,
    round: usize,
    answers: Arc<Mutex<Vec<Vec<String>>>>,
}

fn prompt(text: &'static str, echo: bool) -> Auth {
    Auth::Partial {
        name: Cow::Borrowed("test"),
        instructions: Cow::Borrowed(""),
        prompts: Cow::Owned(vec![(Cow::Borrowed(text), echo)]),
    }
}

impl Handler for MockHandler {
    type Error = russh::Error;

    async fn auth_publickey(&mut self, _user: &str, key: &PublicKey) -> Result<Auth, Self::Error> {
        if let Script::PublicKeyThenOtp(expected, _) = &self.script
            && expected.key_data() == key.key_data()
        {
            return Ok(Auth::Reject {
                proceed_with_methods: Some(MethodSet::from(&[MethodKind::KeyboardInteractive][..])),
                partial_success: true,
            });
        }
        Ok(Auth::reject())
    }

    async fn auth_keyboard_interactive<'a>(
        &'a mut self,
        _user: &str,
        _submethods: &str,
        response: Option<Response<'a>>,
    ) -> Result<Auth, Self::Error> {
        let answers: Option<Vec<String>> = response.map(|r| {
            r.map(|b| String::from_utf8_lossy(&b).into_owned())
                .collect()
        });
        if let Some(a) = &answers {
            self.answers.lock().unwrap().push(a.clone());
        }
        let round = self.round;
        self.round += 1;
        let first = |a: &Option<Vec<String>>| a.as_ref().and_then(|v| v.first().cloned());

        Ok(match (self.script.clone(), round) {
            (
                Script::Password(_) | Script::PasswordWithRetries(..) | Script::PasswordThenOtp(..),
                0,
            ) => prompt("Password: ", false),
            (Script::Password(pw), 1) => {
                if first(&answers).as_deref() == Some(pw) {
                    Auth::Accept
                } else {
                    Auth::reject()
                }
            }
            (Script::PasswordWithRetries(pw, tries), n) => {
                if first(&answers).as_deref() == Some(pw) {
                    Auth::Accept
                } else if n >= tries {
                    Auth::reject()
                } else {
                    prompt("Password: ", false)
                }
            }
            (Script::PasswordThenOtp(pw, _), 1) => {
                if first(&answers).as_deref() == Some(pw) {
                    prompt("Verification code: ", true)
                } else {
                    Auth::reject()
                }
            }
            (Script::PasswordThenOtp(_, otp), 2) => {
                if first(&answers).as_deref() == Some(otp) {
                    Auth::Accept
                } else {
                    Auth::reject()
                }
            }
            (Script::TwoPromptsAtOnce(..), 0) => Auth::Partial {
                name: Cow::Borrowed("two"),
                instructions: Cow::Borrowed("answer both"),
                prompts: Cow::Owned(vec![
                    (Cow::Borrowed("Password: "), false),
                    (Cow::Borrowed("Code: "), true),
                ]),
            },
            (Script::TwoPromptsAtOnce(pw, otp), 1) => match &answers {
                Some(a) if a.len() == 2 && a[0] == pw && a[1] == otp => Auth::Accept,
                _ => Auth::reject(),
            },
            (Script::InfoThenPassword(_), 0) => Auth::Partial {
                name: Cow::Borrowed("banner"),
                instructions: Cow::Borrowed("Welcome. Please log in."),
                prompts: Cow::Owned(vec![]),
            },
            (Script::InfoThenPassword(_), 1) => prompt("Password: ", false),
            (Script::InfoThenPassword(pw), 2) => {
                if first(&answers).as_deref() == Some(pw) {
                    Auth::Accept
                } else {
                    Auth::reject()
                }
            }
            (Script::Endless, _) => Auth::Partial {
                name: Cow::Borrowed("loop"),
                instructions: Cow::Borrowed("again"),
                prompts: Cow::Owned(vec![]),
            },
            (Script::PublicKeyThenOtp(..), 0) => prompt("Verification code: ", true),
            (Script::PublicKeyThenOtp(_, otp), 1) => {
                if first(&answers).as_deref() == Some(otp) {
                    Auth::Accept
                } else {
                    Auth::reject()
                }
            }
            _ => Auth::reject(),
        })
    }
}

/// The identification string the scripted server sends.
const SERVER_ID: &str = "SSH-2.0-HatobaMock_1.0 scripted";

struct Mock {
    port: u16,
    answers: Arc<Mutex<Vec<Vec<String>>>>,
}

async fn start_mock(script: Script) -> Mock {
    let key = PrivateKey::random(&mut russh::keys::key::safe_rng(), Algorithm::Ed25519).unwrap();
    let config = Arc::new(Config {
        keys: vec![key],
        methods: MethodSet::from(&[MethodKind::KeyboardInteractive, MethodKind::PublicKey][..]),
        auth_rejection_time: Duration::from_millis(20),
        auth_rejection_time_initial: Some(Duration::ZERO),
        server_id: SshId::Standard(Cow::Borrowed(SERVER_ID)),
        ..Config::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let answers = Arc::new(Mutex::new(Vec::new()));
    let shared = answers.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let handler = MockHandler {
                script: script.clone(),
                round: 0,
                answers: shared.clone(),
            };
            let config = config.clone();
            tokio::spawn(async move {
                if let Ok(running) = russh::server::run_stream(config, stream, handler).await {
                    let _ = running.await;
                }
            });
        }
    });
    Mock { port, answers }
}

/// Callback with scripted answers that records what it was asked.
struct Callback {
    reply: Mutex<Option<Vec<&'static str>>>,
    delay: Duration,
    asked: Mutex<Vec<PromptRequest>>,
}

impl Callback {
    fn new(reply: Option<Vec<&'static str>>) -> Arc<Self> {
        Arc::new(Self {
            reply: Mutex::new(reply),
            delay: Duration::ZERO,
            asked: Mutex::new(Vec::new()),
        })
    }
    fn slow(reply: Vec<&'static str>, delay: Duration) -> Arc<Self> {
        Arc::new(Self {
            reply: Mutex::new(Some(reply)),
            delay,
            asked: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl KeyboardInteractive for Callback {
    async fn respond(&self, request: PromptRequest) -> Option<Vec<Zeroizing<String>>> {
        self.asked.lock().unwrap().push(request);
        tokio::time::sleep(self.delay).await;
        self.reply.lock().unwrap().clone().map(|v| {
            v.into_iter()
                .map(|s| Zeroizing::new(s.to_owned()))
                .collect()
        })
    }
}

fn config(port: u16, auth: AuthMethod, cb: Option<Arc<Callback>>) -> ConnectConfig {
    let mut c = ConnectConfig::new("127.0.0.1", port, "alice", auth);
    c.keyboard_interactive = cb.map(|c| c as Arc<dyn KeyboardInteractive>);
    c
}

fn password(pw: &str) -> AuthMethod {
    AuthMethod::Password(Zeroizing::new(pw.to_owned()))
}

#[tokio::test]
async fn password_prompt_is_answered_automatically() {
    let mock = start_mock(Script::Password("s3cret")).await;
    let session = connect(
        config(mock.port, password("s3cret"), None),
        Verifier::accepting(),
    )
    .await
    .expect("auto-answered keyboard-interactive login");
    assert!(!session.is_closed());
    assert_eq!(
        *mock.answers.lock().unwrap(),
        vec![vec!["s3cret".to_owned()]]
    );
    // The version exchange's identification string, without its CR LF.
    assert_eq!(session.server_id(), Some(SERVER_ID));
    session.disconnect().await;
}

#[tokio::test]
async fn wrong_password_is_auth_failed_and_never_resent() {
    let mock = start_mock(Script::PasswordWithRetries("right", 3)).await;
    let err = connect(
        config(mock.port, password("wrong"), None),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("interactive"), "{err}");
    // The saved password was tried once; the re-prompt was not answered with it again.
    assert_eq!(
        *mock.answers.lock().unwrap(),
        vec![vec!["wrong".to_owned()]]
    );

    // With a callback the user can type the right password at the re-prompt.
    let mock = start_mock(Script::PasswordWithRetries("right", 3)).await;
    let cb = Callback::new(Some(vec!["right"]));
    connect(
        config(mock.port, password("wrong"), Some(cb.clone())),
        Verifier::accepting(),
    )
    .await
    .expect("second attempt typed by the user");
    let asked = cb.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].prompts,
        vec![Prompt {
            text: "Password: ".into(),
            echo: false
        }]
    );
}

#[tokio::test]
async fn rejected_password_reports_the_cause() {
    let mock = start_mock(Script::Password("right")).await;
    let err = connect(
        config(mock.port, password("wrong"), None),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("rejected the password"), "{err}");
    assert!(
        !err.message.contains("wrong"),
        "no secrets in messages: {err}"
    );
}

#[tokio::test]
async fn otp_prompt_goes_to_the_callback() {
    let mock = start_mock(Script::PasswordThenOtp("pw", "123456")).await;
    let cb = Callback::new(Some(vec!["123456"]));
    let session = connect(
        config(mock.port, password("pw"), Some(cb.clone())),
        Verifier::accepting(),
    )
    .await
    .unwrap();
    // The password round was automatic, only the code round reached the callback.
    {
        let asked = cb.asked.lock().unwrap();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].host, "127.0.0.1");
        assert_eq!(asked[0].name, "test");
        assert_eq!(
            asked[0].prompts,
            vec![Prompt {
                text: "Verification code: ".into(),
                echo: true
            }]
        );
    }
    assert_eq!(
        *mock.answers.lock().unwrap(),
        vec![vec!["pw".to_owned()], vec!["123456".to_owned()]]
    );
    session.disconnect().await;
}

#[tokio::test]
async fn wrong_code_is_auth_failed_and_missing_handler_is_explained() {
    let mock = start_mock(Script::PasswordThenOtp("pw", "123456")).await;
    let cb = Callback::new(Some(vec!["000000"]));
    let err = connect(
        config(mock.port, password("pw"), Some(cb)),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");

    let err = connect(
        config(mock.port, password("pw"), None),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("interactive input"), "{err}");
}

#[tokio::test]
async fn cancelling_the_prompt_cancels_the_connection() {
    let mock = start_mock(Script::PasswordThenOtp("pw", "123456")).await;
    let cb = Callback::new(None);
    let err = connect(
        config(mock.port, password("pw"), Some(cb)),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Cancelled, "{err}");
}

#[tokio::test]
async fn callback_must_answer_every_prompt() {
    let mock = start_mock(Script::TwoPromptsAtOnce("pw", "42")).await;
    let cb = Callback::new(Some(vec!["only-one"]));
    let err = connect(
        config(mock.port, password("pw"), Some(cb)),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Other, "{err}");
}

#[tokio::test]
async fn multi_prompt_rounds_are_delegated_whole() {
    let mock = start_mock(Script::TwoPromptsAtOnce("pw", "42")).await;
    let cb = Callback::new(Some(vec!["pw", "42"]));
    connect(
        config(mock.port, password("ignored"), Some(cb.clone())),
        Verifier::accepting(),
    )
    .await
    .unwrap();
    let asked = cb.asked.lock().unwrap();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].instructions, "answer both");
    assert_eq!(asked[0].prompts.len(), 2);
    assert!(!asked[0].prompts[0].echo && asked[0].prompts[1].echo);
}

#[tokio::test]
async fn informational_rounds_are_acknowledged() {
    let mock = start_mock(Script::InfoThenPassword("pw")).await;
    connect(
        config(mock.port, password("pw"), None),
        Verifier::accepting(),
    )
    .await
    .expect("empty info request is answered with no responses");
}

#[tokio::test]
async fn endless_rounds_are_cut_off() {
    let mock = start_mock(Script::Endless).await;
    let err = connect(
        config(mock.port, password("pw"), None),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
    assert!(err.message.contains("too many"), "{err}");
}

#[tokio::test]
async fn public_key_then_code_second_factor() {
    let parsed = hatoba_ssh::parse_private_key(&fixture("ed25519"), None).unwrap();
    let public = PublicKey::from_openssh(&parsed.public_openssh).unwrap();
    let mock = start_mock(Script::PublicKeyThenOtp(public, "654321")).await;
    let cb = Callback::new(Some(vec!["654321"]));
    let auth = AuthMethod::PrivateKey {
        openssh: parsed.openssh_private.clone(),
        passphrase: None,
    };
    let session = connect(
        config(mock.port, auth.clone(), Some(cb.clone())),
        Verifier::accepting(),
    )
    .await
    .expect("publickey + keyboard-interactive");
    assert_eq!(cb.asked.lock().unwrap().len(), 1);
    session.disconnect().await;

    // Another key is not even partially accepted.
    let other = key_auth("rsa", None);
    let err = connect(config(mock.port, other, Some(cb)), Verifier::accepting())
        .await
        .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::AuthFailed, "{err}");
}

#[tokio::test]
async fn waiting_for_the_user_does_not_count_against_the_timeout() {
    let mock = start_mock(Script::PasswordThenOtp("pw", "123456")).await;
    let cb = Callback::slow(vec!["123456"], Duration::from_millis(2500));
    let mut c = config(mock.port, password("pw"), Some(cb));
    c.connect_timeout = Duration::from_secs(1);
    let started = Instant::now();
    connect(c, Verifier::accepting())
        .await
        .expect("user think time is excluded");
    assert!(started.elapsed() >= Duration::from_millis(2500));
}
