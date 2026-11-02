mod common;

use std::borrow::Cow;
use std::sync::Arc;

use common::Verifier;
use hatoba_ssh::{AuthMethod, ConnectConfig, connect};
use russh::keys::PrivateKey;
use russh::server::{self, Auth, Config, Handler};
use russh::{Preferred, kex};
use tokio::net::TcpListener;

struct ServerHandler;

impl Handler for ServerHandler {
    type Error = russh::Error;

    async fn auth_none(&mut self, _user: &str) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }
}

async fn start_server(kex: &'static [kex::Name]) -> u16 {
    let config = Arc::new(Config {
        keys: vec![PrivateKey::from_openssh(include_str!("fixtures/keys/ed25519")).unwrap()],
        preferred: Preferred {
            kex: Cow::Borrowed(kex),
            ..Preferred::default()
        },
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

/// The exchanges older network switches offer: NIST-curve ECDH and SHA-1 Diffie-Hellman.
#[tokio::test]
async fn connects_to_a_server_with_only_nist_ecdh_and_sha1_exchanges() {
    let port = start_server(&[
        kex::ECDH_SHA2_NISTP256,
        kex::ECDH_SHA2_NISTP384,
        kex::DH_GEX_SHA1,
        kex::DH_G14_SHA1,
        kex::DH_G1_SHA1,
    ])
    .await;
    let cfg = ConnectConfig::new("127.0.0.1", port, "demo", AuthMethod::None);
    let session = connect(cfg, Verifier::accepting()).await.unwrap();
    session.disconnect().await;
}

#[tokio::test]
async fn connects_with_each_nist_curve_alone() {
    for curve in [
        &[kex::ECDH_SHA2_NISTP256],
        &[kex::ECDH_SHA2_NISTP384],
        &[kex::ECDH_SHA2_NISTP521],
    ] {
        let port = start_server(curve).await;
        let cfg = ConnectConfig::new("127.0.0.1", port, "demo", AuthMethod::None);
        let session = connect(cfg, Verifier::accepting())
            .await
            .unwrap_or_else(|e| panic!("{curve:?}: {e:?}"));
        session.disconnect().await;
    }
}
