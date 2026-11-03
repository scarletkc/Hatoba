//! The proxy of Hatoba's own connections (SSH-13): sync, the AI assistant and the update check.
//! This device's preferences name a saved proxy (`LocalPrefs::app_proxy_id`); without one, or
//! once that proxy is deleted, they use the system proxy.

use std::sync::{Mutex, MutexGuard, PoisonError};

use hatoba_core::model::{Item, Proxy, ProxyKind};
use hatoba_core::vault::Vault;
use reqwest::{NoProxy, Url};
use tauri::{AppHandle, Manager};
use zeroize::Zeroizing;

use crate::dto::LocalPrefs;
use crate::state::AppState;
use crate::sync;

/// The saved proxy that Hatoba's own connections go through now; `None` for the system proxy.
#[derive(Default)]
pub struct AppProxy(Mutex<Option<Proxy>>);

impl AppProxy {
    fn applied(&self) -> MutexGuard<'_, Option<Proxy>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The proxy of the update check's requests; `None` for the system proxy.
    pub fn reqwest(&self) -> Option<reqwest::Proxy> {
        self.applied().as_ref().and_then(reqwest_proxy)
    }
}

/// Points sync, the AI assistant and the update check at the proxy this device's preferences
/// name, when they use another one, and connects sync again through it. Does nothing while the
/// vault is locked.
pub fn refresh(app: &AppHandle) {
    let st = app.state::<AppState>();
    let Ok(proxy) = st.with_unlocked(|v| Ok(named(v))) else {
        return;
    };
    {
        let mut applied = st.app_proxy.applied();
        if *applied == proxy {
            return;
        }
        applied.clone_from(&proxy);
    }
    let through = proxy.as_ref().and_then(reqwest_proxy);
    hatoba_core::sync::set_proxy(through.clone());
    st.ai.set_proxy(through);
    sync::reconnect(app);
}

/// Drops the saved proxy and its password when the vault locks (SEC-01): until [`refresh`] runs
/// after the unlock, sync, the AI assistant and the update check use the system proxy.
pub fn forget(app: &AppHandle) {
    let st = app.state::<AppState>();
    if st.app_proxy.applied().take().is_none() {
        return;
    }
    hatoba_core::sync::set_proxy(None);
    st.ai.set_proxy(None);
}

/// The saved proxy this device's preferences name, if it still exists.
fn named(v: &Vault) -> Option<Proxy> {
    let json = v.local_prefs().ok().flatten()?;
    let id = LocalPrefs::from_stored(&json).app_proxy_id?;
    v.get(&id).and_then(Item::as_proxy).cloned()
}

/// reqwest's proxy for `proxy`. Host names go to it unresolved, as SSH-13 has it, and loopback
/// targets, such as a model server on this computer, bypass it.
fn reqwest_proxy(proxy: &Proxy) -> Option<reqwest::Proxy> {
    let scheme = match proxy.kind {
        ProxyKind::Socks5 => "socks5h",
        ProxyKind::Http => "http",
    };
    let host = if proxy.address.contains(':') {
        format!("[{}]", proxy.address)
    } else {
        proxy.address.clone()
    };
    let built = Url::parse(&format!("{scheme}://{host}:{}", proxy.port))
        .ok()
        .and_then(|mut url| {
            if !proxy.username.is_empty() {
                // reqwest decodes the user info, and `Url` leaves `%` as it is.
                url.set_username(&proxy.username.replace('%', "%25")).ok()?;
                let password = Zeroizing::new(proxy.password.replace('%', "%25"));
                url.set_password(Some(&password)).ok()?;
            }
            reqwest::Proxy::all(url.as_str()).ok()
        });
    let Some(built) = built else {
        // Checked when it was saved (`proxy_from_input`), so this does not happen.
        tracing::warn!("the saved proxy does not make a proxy URL; using the system proxy");
        return None;
    };
    Some(built.no_proxy(NoProxy::from_string("localhost,127.0.0.0/8,::1")))
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::*;

    fn proxy(kind: ProxyKind, port: u16, username: &str, password: &str) -> Proxy {
        Proxy {
            name: "Office".into(),
            kind,
            address: "127.0.0.1".into(),
            port,
            username: username.into(),
            password: Zeroizing::new(password.into()),
            updated_at: 0,
        }
    }

    fn client(proxy: &Proxy) -> reqwest::Client {
        let _ = rustls::crypto::ring::default_provider().install_default();
        reqwest::Client::builder()
            .proxy(reqwest_proxy(proxy).unwrap())
            .build()
            .unwrap()
    }

    /// What the first connection to `listener` sends, up to `len` bytes.
    async fn first_bytes(listener: &TcpListener, len: usize) -> Vec<u8> {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0; len];
        let n = socket.read(&mut buf).await.unwrap();
        buf.truncate(n);
        buf
    }

    #[tokio::test]
    async fn an_http_proxy_gets_the_request_and_the_sign_in() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = client(&proxy(ProxyKind::Http, port, "alice", "p@ss:w/%41rd%"));
        let request = tokio::spawn(async move {
            let _ = client.get("http://hatoba.test/page").send().await;
        });
        let head = String::from_utf8(first_bytes(&listener, 4096).await).unwrap();
        request.abort();
        assert!(
            head.starts_with("GET http://hatoba.test/page HTTP/1.1\r\n"),
            "{head}"
        );
        // base64("alice:p@ss:w/%41rd%")
        assert!(
            head.to_lowercase()
                .contains("proxy-authorization: basic ywxpy2u6cebzczp3lyu0mxjkjq=="),
            "{head}"
        );
    }

    #[tokio::test]
    async fn a_socks5_proxy_gets_the_sign_in_and_the_host_name() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let client = client(&proxy(ProxyKind::Socks5, port, "alice", "p@ss"));
        let request = tokio::spawn(async move {
            let _ = client.get("http://hatoba.test/").send().await;
        });
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut greeting = [0; 2];
        socket.read_exact(&mut greeting).await.unwrap();
        let mut methods = vec![0; usize::from(greeting[1])];
        socket.read_exact(&mut methods).await.unwrap();
        assert_eq!(greeting[0], 5);
        assert!(
            methods.contains(&2),
            "offers username and password: {methods:?}"
        );
        socket.write_all(&[5, 2]).await.unwrap();
        let mut auth = vec![0; 64];
        let n = socket.read(&mut auth).await.unwrap();
        assert_eq!(&auth[..n], b"\x01\x05alice\x04p@ss");
        socket.write_all(&[1, 0]).await.unwrap();
        let mut connect = vec![0; 64];
        let n = socket.read(&mut connect).await.unwrap();
        request.abort();
        // CONNECT to a domain name (address type 3), which the proxy resolves.
        assert_eq!(&connect[..5], b"\x05\x01\x00\x03\x0b");
        assert_eq!(&connect[5..16], b"hatoba.test");
        assert_eq!(n, 18);
    }

    #[tokio::test]
    async fn loopback_targets_bypass_the_proxy() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::any())
            .respond_with(wiremock::ResponseTemplate::new(204))
            .mount(&server)
            .await;
        // Nothing listens on port 9 (discard) here, so a proxied request would fail.
        let client = client(&proxy(ProxyKind::Http, 9, "", ""));
        let resp = client.get(server.uri()).send().await.unwrap();
        assert_eq!(resp.status().as_u16(), 204);
    }
}
