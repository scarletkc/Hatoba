//! The shared client of provider, search and MCP requests, built again once its proxy changes.

use std::sync::{Mutex, MutexGuard, PoisonError};

/// The client (`hatoba_ai::provider::http_client_through`), built when first used, and the proxy
/// it goes through: `None` for the system proxy.
#[derive(Default)]
pub(crate) struct Http(Mutex<State>);

#[derive(Default)]
struct State {
    proxy: Option<reqwest::Proxy>,
    client: Option<reqwest::Client>,
}

impl Http {
    fn state(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The client, through the current proxy. Cloning is cheap, and every clone shares one
    /// connection pool.
    pub(crate) fn client(&self) -> reqwest::Client {
        let mut state = self.state();
        let State { proxy, client } = &mut *state;
        client
            .get_or_insert_with(|| hatoba_ai::provider::http_client_through(proxy.clone()))
            .clone()
    }

    /// The current proxy; `None` for the system proxy.
    pub(crate) fn proxy(&self) -> Option<reqwest::Proxy> {
        self.state().proxy.clone()
    }

    /// Requests from now on go through `proxy`. Those already sent keep their client.
    pub(crate) fn set_proxy(&self, proxy: Option<reqwest::Proxy>) {
        *self.state() = State {
            proxy,
            client: None,
        };
    }
}
