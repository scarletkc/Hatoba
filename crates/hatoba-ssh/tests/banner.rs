//! The server's banners (`SSH_MSG_USERAUTH_BANNER`), against an in-process server that sends one
//! and then holds a login request, as Tailscale SSH's check mode holds the "none" login until the
//! user signs in at the link in it.

mod common;

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use common::*;
use hatoba_ssh::{
    AuthMethod, Banner, BannerWait, ConnectConfig, KeyboardInteractive, PromptRequest,
    SshErrorKind, connect,
};
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{Auth, Config, Handler, PublicKeyOffer};
use russh::{MethodKind, MethodSet, SshId};
use tokio::net::TcpListener;
use zeroize::Zeroizing;

const CHECK: &str = "# Tailscale SSH requires an additional check.\r\n\
                     # To authenticate, visit: https://login.tailscale.com/a/1234\r\n";

/// The request the server holds.
#[derive(Clone, Copy)]
enum Held {
    /// The "none" probe, which it then lets through.
    None,
    /// The public key request, after rejecting "none" at once.
    PublicKey,
}

struct HoldingHandler {
    held: Held,
    hold: Duration,
}

impl Handler for HoldingHandler {
    type Error = russh::Error;

    async fn authentication_banner(&mut self) -> Result<Option<String>, Self::Error> {
        Ok(Some(CHECK.to_owned()))
    }

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        match self.held {
            Held::None => {
                tokio::time::sleep(self.hold).await;
                Ok(Auth::Accept)
            }
            Held::PublicKey => Ok(Auth::reject()),
        }
    }

    async fn auth_publickey_offered(
        &mut self,
        _user: &str,
        _offer: PublicKeyOffer<'_>,
    ) -> Result<Auth, Self::Error> {
        tokio::time::sleep(self.hold).await;
        Ok(Auth::Accept)
    }
}

/// A server that sends [`CHECK`] and lets the `held` request through after `hold`.
async fn start_holding(held: Held, hold: Duration) -> u16 {
    let key = PrivateKey::random(&mut russh::keys::key::safe_rng(), Algorithm::Ed25519).unwrap();
    let config = Arc::new(Config {
        keys: vec![key],
        methods: MethodSet::from(&[MethodKind::None, MethodKind::PublicKey][..]),
        auth_rejection_time: Duration::from_millis(20),
        auth_rejection_time_initial: Some(Duration::ZERO),
        server_id: SshId::Standard(Cow::Borrowed("SSH-2.0-HatobaMock_1.0 holding")),
        ..Config::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let config = config.clone();
            tokio::spawn(async move {
                let handler = HoldingHandler { held, hold };
                if let Ok(running) = russh::server::run_stream(config, stream, handler).await {
                    let _ = running.await;
                }
            });
        }
    });
    port
}

/// Sets its flag when dropped.
struct Flag(Arc<AtomicBool>);

impl Drop for Flag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

/// Records the banners. `show` says whether someone sees them; `give_up` how long they wait on
/// one before giving up (`None`: until the server answers).
struct Shows {
    show: bool,
    give_up: Option<Duration>,
    seen: Mutex<Vec<Banner>>,
    ended: Arc<AtomicBool>,
}

impl Shows {
    fn new(show: bool, give_up: Option<Duration>) -> Arc<Self> {
        Arc::new(Self {
            show,
            give_up,
            seen: Mutex::new(Vec::new()),
            ended: Arc::new(AtomicBool::new(false)),
        })
    }
}

#[async_trait]
impl KeyboardInteractive for Shows {
    async fn respond(&self, _request: PromptRequest) -> Option<Vec<Zeroizing<String>>> {
        None
    }

    fn banner(&self, banner: Banner) -> Option<BannerWait> {
        self.seen.lock().unwrap().push(banner);
        if !self.show {
            return None;
        }
        let ended = Flag(Arc::clone(&self.ended));
        let give_up = self.give_up;
        Some(Box::pin(async move {
            let _ended = ended;
            match give_up {
                Some(after) => tokio::time::sleep(after).await,
                None => std::future::pending().await,
            }
        }))
    }
}

fn config(port: u16, shows: &Arc<Shows>, timeout: Duration) -> ConnectConfig {
    let mut c = ConnectConfig::new("127.0.0.1", port, "alice", AuthMethod::None);
    c.keyboard_interactive = Some(Arc::clone(shows) as Arc<dyn KeyboardInteractive>);
    c.connect_timeout = timeout;
    c
}

#[tokio::test]
async fn the_banner_is_shown_cleaned_and_its_wait_ends_with_the_answer() {
    let port = start_holding(Held::None, Duration::ZERO).await;
    let shows = Shows::new(true, None);
    let session = connect(
        config(port, &shows, Duration::from_secs(5)),
        Verifier::accepting(),
    )
    .await
    .expect("the server lets the login through");
    assert_eq!(
        *shows.seen.lock().unwrap(),
        vec![Banner {
            host: "127.0.0.1".into(),
            port,
            username: "alice".into(),
            text: "# Tailscale SSH requires an additional check.\n# To authenticate, visit: \
                   https://login.tailscale.com/a/1234"
                .into(),
        }]
    );
    assert!(shows.ended.load(Ordering::SeqCst), "the wait was dropped");
    session.disconnect().await;
}

#[tokio::test]
async fn a_login_held_after_a_banner_someone_sees_does_not_time_out() {
    let port = start_holding(Held::None, Duration::from_millis(2500)).await;
    let shows = Shows::new(true, None);
    let started = Instant::now();
    let session = connect(
        config(port, &shows, Duration::from_secs(1)),
        Verifier::accepting(),
    )
    .await
    .expect("waiting on the server after the banner is excluded");
    assert!(started.elapsed() >= Duration::from_millis(2500));
    session.disconnect().await;
}

#[tokio::test]
async fn a_banner_nobody_sees_leaves_the_timeout_running() {
    let port = start_holding(Held::None, Duration::from_secs(10)).await;
    let shows = Shows::new(false, None);
    let started = Instant::now();
    let err = connect(
        config(port, &shows, Duration::from_secs(1)),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Timeout, "{err}");
    assert!(err.message.contains("authentication"), "{err}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(shows.seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn giving_up_on_a_banner_cancels_the_login() {
    let port = start_holding(Held::None, Duration::from_secs(10)).await;
    let shows = Shows::new(true, Some(Duration::from_millis(200)));
    let started = Instant::now();
    let err = connect(
        config(port, &shows, Duration::from_secs(5)),
        Verifier::accepting(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Cancelled, "{err}");
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn a_banner_waits_only_on_the_request_it_came_with() {
    // The banner comes with the "none" probe, which the server rejects at once, and the server
    // then holds the public key request: neither the pause nor giving up on the banner reaches it.
    let port = start_holding(Held::PublicKey, Duration::from_secs(10)).await;
    let shows = Shows::new(true, Some(Duration::from_millis(300)));
    let mut c = config(port, &shows, Duration::from_secs(1));
    c.auth = key_auth("ed25519", None);
    let started = Instant::now();
    let err = connect(c, Verifier::accepting()).await.unwrap_err();
    assert_eq!(err.kind, SshErrorKind::Timeout, "{err}");
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(shows.ended.load(Ordering::SeqCst), "the wait was dropped");
}
