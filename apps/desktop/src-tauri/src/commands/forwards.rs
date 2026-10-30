//! Port forwarding rules: local (FWD-01), remote (FWD-03) and dynamic SOCKS (FWD-04), with
//! auto-start (FWD-02), which run on the host's connection ([`crate::connection`], FWD-05).

use hatoba_core::model::{ForwardKind, Item, PortForward};
use tauri::{AppHandle, State};
use zeroize::Zeroizing;

use super::hosts::MAX_NAME_CHARS;
use crate::dto::{ConnectionsEvent, ForwardInput, ForwardOptions, ForwardView};
use crate::error::{AppError, AppResult};
use crate::ssh::Overrides;
use crate::state::AppState;
use crate::{connection, sync};

fn view(id: &str, f: &PortForward) -> ForwardView {
    ForwardView {
        id: id.to_owned(),
        host_id: f.host_id.clone(),
        name: f.name.clone(),
        kind: f.kind.into(),
        bind_address: f.bind_address.clone(),
        bind_port: f.bind_port,
        dest_host: f.dest_host.clone().unwrap_or_default(),
        dest_port: f.dest_port.unwrap_or_default(),
        auto_start: f.auto_start,
    }
}

/// Every host's forwards, by host and then by bind and destination port.
#[tauri::command]
#[specta::specta]
pub fn forwards_list_all(state: State<'_, AppState>) -> AppResult<Vec<ForwardView>> {
    state.with_unlocked(|v| {
        let mut list: Vec<ForwardView> = v.forwards().iter().map(|(id, f)| view(id, f)).collect();
        list.sort_by(|a, b| {
            (&a.host_id, a.bind_port, a.dest_port).cmp(&(&b.host_id, b.bind_port, b.dest_port))
        });
        Ok(list)
    })
}

/// An address as it is listened on or connected to: an IPv6 literal typed as ssh writes it, in
/// brackets, loses them.
fn unbracketed(address: &str) -> &str {
    let address = address.trim();
    address
        .strip_prefix('[')
        .and_then(|a| a.strip_suffix(']'))
        .filter(|a| a.contains(':'))
        .unwrap_or(address)
}

#[tauri::command]
#[specta::specta]
pub fn forward_save(
    app: AppHandle,
    state: State<'_, AppState>,
    input: ForwardInput,
) -> AppResult<ForwardView> {
    let name = input.name.trim();
    if name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::invalid("name", "name is too long"));
    }
    let kind = ForwardKind::from(input.kind);
    // A remote forward listens where `ssh -R` does by default.
    let bind_address = match (unbracketed(&input.bind_address), kind) {
        ("", ForwardKind::Remote) => "localhost".to_owned(),
        ("", _) => "127.0.0.1".to_owned(),
        (a, _) => a.to_owned(),
    };
    if bind_address.chars().any(char::is_whitespace) {
        return Err(AppError::invalid("bind_address", "invalid bind address"));
    }
    let dest = match kind {
        ForwardKind::Dynamic => None,
        ForwardKind::Local | ForwardKind::Remote => {
            let dest_host = unbracketed(&input.dest_host).to_owned();
            if dest_host.is_empty() || dest_host.chars().any(char::is_whitespace) {
                return Err(AppError::invalid(
                    "dest_host",
                    "destination host is required",
                ));
            }
            if input.dest_port == 0 {
                return Err(AppError::invalid(
                    "dest_port",
                    "port must be between 1 and 65535",
                ));
            }
            Some((dest_host, input.dest_port))
        }
    };
    let saved = state.with_unlocked(|v| {
        v.get(&input.host_id)
            .and_then(Item::as_host)
            .ok_or_else(|| AppError::not_found("host"))?;
        let (dest_host, dest_port) = dest.unzip();
        let forward = PortForward {
            host_id: input.host_id.clone(),
            name: name.to_owned(),
            kind,
            bind_address,
            bind_port: input.bind_port,
            dest_host,
            dest_port,
            auto_start: input.auto_start,
            updated_at: 0,
        };
        let id = v.put(input.id.as_deref(), Item::Forward(forward))?;
        let f = v
            .get(&id)
            .and_then(Item::as_forward)
            .cloned()
            .ok_or_else(|| AppError::not_found("forward"))?;
        Ok(view(&id, &f))
    })?;
    sync::local_change(&app);
    connection::rules_changed(&app);
    Ok(saved)
}

/// Deletes the rule, and stops it where it runs.
#[tauri::command]
#[specta::specta]
pub fn forward_delete(app: AppHandle, state: State<'_, AppState>, id: String) -> AppResult<()> {
    let host_id = state.with_unlocked(|v| {
        let host_id = v
            .get(&id)
            .and_then(Item::as_forward)
            .map(|f| f.host_id.clone())
            .ok_or_else(|| AppError::not_found("forward"))?;
        v.delete(&id)?;
        Ok(host_id)
    })?;
    sync::local_change(&app);
    connection::stop(&app, &host_id, &id);
    Ok(())
}

/// Starts the saved forward on the host's connection (SSH-16), which signs in with `options`
/// when no tab or forward has it open, and returns the port it listens on (on the server for a
/// remote forward).
#[tauri::command]
#[specta::specta]
pub async fn forward_start(
    app: AppHandle,
    host_id: String,
    forward_id: String,
    options: ForwardOptions,
) -> AppResult<u16> {
    let overrides = Overrides {
        password: options
            .password
            .filter(|p| !p.is_empty())
            .map(Zeroizing::new),
        passphrase: options
            .passphrase
            .filter(|p| !p.is_empty())
            .map(Zeroizing::new),
        ask_password: false,
    };
    connection::start(&app, &host_id, &forward_id, &overrides).await
}

/// Stops the forward; the host's connection closes when no tab or forward is left on it.
#[tauri::command]
#[specta::specta]
pub fn forward_stop(app: AppHandle, host_id: String, forward_id: String) {
    connection::stop(&app, &host_id, &forward_id);
}

/// Every saved host's open connection, with the forwards on it.
#[tauri::command]
#[specta::specta]
pub fn connections_list(state: State<'_, AppState>) -> ConnectionsEvent {
    connection::list(&state)
}

#[cfg(test)]
mod tests {
    use super::unbracketed;

    #[test]
    fn an_ipv6_address_in_brackets_loses_them() {
        assert_eq!(unbracketed(" [::1] "), "::1");
        assert_eq!(unbracketed("[fd00::5]"), "fd00::5");
        assert_eq!(unbracketed("127.0.0.1"), "127.0.0.1");
        assert_eq!(unbracketed("[db.internal]"), "[db.internal]");
    }
}
