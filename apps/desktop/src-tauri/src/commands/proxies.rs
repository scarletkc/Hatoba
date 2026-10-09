//! Saved SOCKS5 and HTTP proxies, and the default proxy of this device (SSH-13).

use hatoba_core::model::{HostProxy, Item, Proxy, ProxyKind as CoreKind};
use hatoba_core::vault::Vault;
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use crate::convert::proxy_view;
use crate::dto::{LocalPrefs, ProxyInput, ProxyKind, ProxyView};
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::sync;

/// The longest SOCKS5 username or password (RFC 1929); HTTP proxies get the same limit.
const MAX_CREDENTIAL_BYTES: usize = 255;

pub fn core_kind(kind: ProxyKind) -> CoreKind {
    match kind {
        ProxyKind::Socks5 => CoreKind::Socks5,
        ProxyKind::Http => CoreKind::Http,
    }
}

/// The id of this device's default proxy, from the device-local preferences.
pub fn device_default(vault: &Vault) -> Option<String> {
    let json = vault.local_prefs().ok().flatten()?;
    LocalPrefs::from_stored(&json).default_proxy_id
}

fn find_proxy(vault: &Vault, id: &str) -> AppResult<Proxy> {
    vault
        .get(id)
        .and_then(Item::as_proxy)
        .cloned()
        .ok_or_else(|| AppError::not_found("proxy"))
}

/// Validates the proxy form and builds the stored model. `None` as the password keeps the
/// saved one (as HOST-08 does for hosts); an empty one removes it.
pub fn proxy_from_input(input: &ProxyInput, existing: Option<&Proxy>) -> AppResult<Proxy> {
    let bad = |c: char| c.is_whitespace() || c.is_control();
    let name = input.name.trim();
    if name.is_empty() {
        return Err(AppError::invalid("name", "name is required"));
    }
    if name.chars().count() > 128 {
        return Err(AppError::invalid("name", "name is too long"));
    }
    let address = input.address.trim();
    let address = address
        .strip_prefix('[')
        .and_then(|a| a.strip_suffix(']'))
        .unwrap_or(address);
    if address.is_empty() {
        return Err(AppError::invalid("address", "address is required"));
    }
    if address.len() > 255 || address.contains(bad) || address.contains(['@', '/', '\\']) {
        return Err(AppError::invalid(
            "address",
            "address is not a valid host name or IP",
        ));
    }
    if input.port == 0 {
        return Err(AppError::invalid(
            "port",
            "port must be between 1 and 65535",
        ));
    }
    let username = input.username.trim();
    if username.len() > MAX_CREDENTIAL_BYTES || username.contains(char::is_control) {
        return Err(AppError::invalid("username", "username is not valid"));
    }
    if input.kind == ProxyKind::Http && username.contains(':') {
        return Err(AppError::invalid(
            "username",
            "an HTTP proxy username can't contain a colon",
        ));
    }
    let password = match (&input.password, existing) {
        _ if username.is_empty() => Zeroizing::new(String::new()),
        (Some(new), _) => Zeroizing::new(new.clone()),
        (None, Some(saved)) => saved.password.clone(),
        (None, None) => Zeroizing::new(String::new()),
    };
    if password.len() > MAX_CREDENTIAL_BYTES {
        return Err(AppError::invalid(
            "password",
            "password is longer than 255 bytes",
        ));
    }
    Ok(Proxy {
        name: name.to_owned(),
        kind: core_kind(input.kind),
        address: address.to_owned(),
        port: input.port,
        username: username.to_owned(),
        password,
        updated_at: 0,
    })
}

#[tauri::command]
#[specta::specta]
pub fn proxies_list(state: State<'_, AppState>) -> AppResult<Vec<ProxyView>> {
    state.with_unlocked(|v| {
        let mut proxies: Vec<ProxyView> = v
            .proxies()
            .iter()
            .map(|(id, p)| proxy_view(id, p, v))
            .collect();
        proxies.sort_by_cached_key(|p| p.name.to_lowercase());
        Ok(proxies)
    })
}

#[tauri::command]
#[specta::specta]
pub fn proxy_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: ProxyInput,
) -> AppResult<ProxyView> {
    let view = state.with_unlocked(|v| {
        let existing = match &input.id {
            Some(id) => Some(find_proxy(v, id)?),
            None => None,
        };
        let proxy = proxy_from_input(&input, existing.as_ref())?;
        let id = v.put(input.id.as_deref(), Item::Proxy(proxy))?;
        Ok(proxy_view(&id, &find_proxy(v, &id)?, v))
    })?;
    sync::local_change(&app);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub fn proxy_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.with_unlocked(|v| remove(v, &id))?;
    sync::local_change(&app);
    Ok(())
}

/// Deletes a proxy. Hosts that used it go back to the device default, and this device stops
/// using it as its default.
fn remove(v: &mut Vault, id: &str) -> AppResult<()> {
    find_proxy(v, id)?;
    for (host_id, mut host) in v.hosts() {
        if host.proxy.proxy_id() == Some(id) {
            host.proxy = HostProxy::DeviceDefault;
            v.put(Some(&host_id), Item::Host(host))?;
        }
    }
    if device_default(v).as_deref() == Some(id) {
        let stored = v.local_prefs()?;
        let mut prefs = stored
            .as_deref()
            .map(LocalPrefs::from_stored)
            .unwrap_or_default();
        prefs.default_proxy_id = None;
        let json = crate::commands::settings::merge_prefs(stored.as_deref(), &prefs)?;
        v.set_local_prefs(&json)?;
    }
    v.delete(id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use hatoba_core::KdfParams;
    use hatoba_core::model::Host;

    use super::*;

    fn input(kind: ProxyKind, username: &str, password: Option<&str>) -> ProxyInput {
        ProxyInput {
            id: None,
            name: " Office ".into(),
            kind,
            address: " [::1] ".into(),
            port: 1080,
            username: username.into(),
            password: password.map(str::to_owned),
        }
    }

    #[test]
    fn proxy_input_is_checked_and_the_password_kept() {
        let proxy = proxy_from_input(&input(ProxyKind::Socks5, "alice", Some("pw")), None).unwrap();
        assert_eq!((proxy.name.as_str(), proxy.address.as_str()), ("Office", "::1"));
        assert_eq!(proxy.password.as_str(), "pw");

        // HOST-08: no password in the input keeps the saved one; an empty one removes it.
        let kept = proxy_from_input(&input(ProxyKind::Socks5, "alice", None), Some(&proxy)).unwrap();
        assert_eq!(kept.password.as_str(), "pw");
        let cleared =
            proxy_from_input(&input(ProxyKind::Socks5, "alice", Some("")), Some(&proxy)).unwrap();
        assert!(cleared.password.is_empty());
        // Without a username there is nothing to sign in with.
        let anonymous = proxy_from_input(&input(ProxyKind::Socks5, "", None), Some(&proxy)).unwrap();
        assert!(anonymous.password.is_empty());

        let err = proxy_from_input(&input(ProxyKind::Http, "a:b", None), None).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("username"));
        let long = "x".repeat(256);
        let err = proxy_from_input(&input(ProxyKind::Socks5, "a", Some(&long)), None).unwrap_err();
        assert_eq!(err.field.as_deref(), Some("password"));
        let mut bad = input(ProxyKind::Socks5, "", None);
        bad.address = "proxy example".into();
        assert_eq!(
            proxy_from_input(&bad, None).unwrap_err().field.as_deref(),
            Some("address")
        );
        bad.address = "proxy".into();
        bad.port = 0;
        assert_eq!(
            proxy_from_input(&bad, None).unwrap_err().field.as_deref(),
            Some("port")
        );
    }

    #[test]
    fn deleting_a_proxy_resets_the_hosts_and_the_device_default() {
        let mut v = Vault::open_in_memory().unwrap();
        v.create_with_params("correct horse battery staple", KdfParams::for_tests())
            .unwrap();
        assert_eq!(device_default(&v), None);
        let proxy = proxy_from_input(&input(ProxyKind::Socks5, "", None), None).unwrap();
        let id = v.put(None, Item::Proxy(proxy.clone())).unwrap();
        let other = v.put(None, Item::Proxy(proxy)).unwrap();
        v.set_local_prefs(&format!(r#"{{"default_proxy_id":"{id}","language":"ja"}}"#))
            .unwrap();
        assert_eq!(device_default(&v).as_deref(), Some(id.as_str()));
        let host = |proxy_id: &str| Host {
            proxy: HostProxy::Proxy {
                proxy_id: proxy_id.to_owned(),
            },
            ..Host::default()
        };
        let uses = v.put(None, Item::Host(host(&id))).unwrap();
        let keeps = v.put(None, Item::Host(host(&other))).unwrap();

        remove(&mut v, &id).unwrap();
        assert!(v.get(&id).is_none());
        let proxy_of = |host_id: &str| v.get(host_id).and_then(Item::as_host).unwrap().proxy.clone();
        assert_eq!(proxy_of(&uses), HostProxy::DeviceDefault);
        assert_eq!(proxy_of(&keeps).proxy_id(), Some(other.as_str()));
        assert_eq!(device_default(&v), None);
        // The other prefs stay.
        let prefs = LocalPrefs::from_stored(&v.local_prefs().unwrap().unwrap());
        assert_eq!(prefs.language, crate::dto::Language::Ja);

        assert_eq!(remove(&mut v, &id).unwrap_err().code, crate::error::ErrorCode::NotFound);
    }
}
