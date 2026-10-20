use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use hatoba_ssh::{AuthMethod, ConnectConfig, HostKeyInfo, HostKeyVerifier, JumpHop, connect};
use russh::keys::PrivateKey;
use russh::server::{Auth, Config, Handler, Session};
use russh::{Channel, server};
use tokio::net::{TcpListener, TcpStream};

struct ServerHandler;

impl Handler for ServerHandler {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<server::Msg>,
        host: &str,
        port: u32,
        _originator: &str,
        _originator_port: u32,
        reply: server::ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        let mut stream = TcpStream::connect((host, port as u16)).await?;
        reply.accept().await;
        tokio::spawn(async move {
            let _ = tokio::io::copy_bidirectional(&mut channel.into_stream(), &mut stream).await;
        });
        Ok(())
    }
}

async fn start_server(rsa: bool, ed25519: bool) -> u16 {
    let mut keys = Vec::new();
    if rsa {
        keys.push(PrivateKey::from_openssh(include_str!("fixtures/keys/rsa")).unwrap());
    }
    if ed25519 {
        keys.push(PrivateKey::from_openssh(include_str!("fixtures/keys/ed25519")).unwrap());
    }
    let config = Arc::new(Config {
        keys,
        ..Config::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let config = config.clone();
            tokio::spawn(async move {
                if let Ok(running) = server::run_stream(config, stream, ServerHandler).await {
                    let _ = running.await;
                }
            });
        }
    });
    port
}

#[derive(Default)]
struct Verifier {
    known: HashMap<u16, Vec<String>>,
    presented: Mutex<Vec<(u16, String)>>,
    requested: Mutex<Vec<(String, u16)>>,
}

#[async_trait]
impl HostKeyVerifier for Verifier {
    async fn known_key_types(&self, host: &str, port: u16) -> Vec<String> {
        self.requested.lock().unwrap().push((host.to_owned(), port));
        self.known.get(&port).cloned().unwrap_or_default()
    }

    async fn verify(&self, _host: &str, port: u16, key: &HostKeyInfo) -> bool {
        self.presented
            .lock()
            .unwrap()
            .push((port, key.key_type.clone()));
        true
    }
}

#[tokio::test]
async fn stored_rsa_is_preferred_when_the_server_adds_ed25519() {
    let port = start_server(true, true).await;
    let verifier = Arc::new(Verifier {
        known: HashMap::from([(port, vec!["ssh-rsa".to_owned()])]),
        ..Verifier::default()
    });
    let cfg = ConnectConfig::new("127.0.0.1", port, "demo", AuthMethod::None);
    let session = connect(cfg, verifier.clone()).await.unwrap();
    assert_eq!(
        *verifier.presented.lock().unwrap(),
        [(port, "ssh-rsa".to_owned())]
    );
    assert_eq!(
        *verifier.requested.lock().unwrap(),
        [("127.0.0.1".to_owned(), port)]
    );
    session.disconnect().await;
}

#[tokio::test]
async fn an_unknown_host_keeps_the_default_algorithm_order() {
    let port = start_server(true, true).await;
    let verifier = Arc::new(Verifier::default());
    let cfg = ConnectConfig::new("127.0.0.1", port, "demo", AuthMethod::None);
    let session = connect(cfg, verifier.clone()).await.unwrap();
    assert_eq!(
        *verifier.presented.lock().unwrap(),
        [(port, "ssh-ed25519".to_owned())]
    );
    session.disconnect().await;
}

#[tokio::test]
async fn an_unavailable_stored_type_falls_back_to_an_offered_type() {
    let port = start_server(false, true).await;
    let verifier = Arc::new(Verifier {
        known: HashMap::from([(port, vec!["ssh-rsa".to_owned()])]),
        ..Verifier::default()
    });
    let cfg = ConnectConfig::new("127.0.0.1", port, "demo", AuthMethod::None);
    let session = connect(cfg, verifier.clone()).await.unwrap();
    assert_eq!(
        *verifier.presented.lock().unwrap(),
        [(port, "ssh-ed25519".to_owned())]
    );
    session.disconnect().await;
}

#[tokio::test]
async fn each_proxy_jump_hop_uses_its_own_known_key_types() {
    let jump = start_server(true, true).await;
    let target = start_server(true, true).await;
    let verifier = Arc::new(Verifier {
        known: HashMap::from([
            (jump, vec!["ssh-rsa".to_owned()]),
            (target, vec!["ssh-ed25519".to_owned()]),
        ]),
        ..Verifier::default()
    });
    let mut cfg = ConnectConfig::new("127.0.0.1", target, "demo", AuthMethod::None);
    cfg.jump.push(JumpHop {
        host: "127.0.0.1".to_owned(),
        port: jump,
        username: "demo".to_owned(),
        auth: AuthMethod::None,
        agent_sockets: Vec::new(),
    });
    let session = connect(cfg, verifier.clone()).await.unwrap();
    assert_eq!(
        *verifier.presented.lock().unwrap(),
        [
            (jump, "ssh-rsa".to_owned()),
            (target, "ssh-ed25519".to_owned())
        ]
    );
    assert_eq!(
        *verifier.requested.lock().unwrap(),
        [
            ("127.0.0.1".to_owned(), jump),
            ("127.0.0.1".to_owned(), target)
        ]
    );
    session.disconnect().await;
}
