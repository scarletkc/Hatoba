//! Host listing and lookup for `hatoba` commands (HOST-15).

use hatoba_cli::protocol::{AuthKind, HostInfo};
use hatoba_core::model::{Host, HostAuth, Item};
use hatoba_core::vault::Vault;

/// The saved hosts, by name, without their credentials.
pub(crate) fn host_list(vault: &Vault) -> Vec<HostInfo> {
    let mut hosts: Vec<HostInfo> = vault
        .hosts()
        .into_iter()
        .map(|(id, h)| HostInfo {
            auth: match &h.auth {
                HostAuth::Password { password } if !password.is_empty() => AuthKind::Password,
                HostAuth::Password { .. } | HostAuth::Ask => AuthKind::Ask,
                HostAuth::Key { .. } => AuthKind::Key,
                HostAuth::Agent => AuthKind::Agent,
            },
            group: h
                .group_id
                .as_deref()
                .and_then(|g| vault.get(g))
                .and_then(Item::as_group)
                .map(|g| g.name.clone()),
            jump: h
                .jump_host_id
                .as_deref()
                .and_then(|j| vault.get(j))
                .and_then(Item::as_host)
                .map(|j| j.name.clone()),
            id,
            name: h.name,
            address: h.address,
            port: h.port,
            username: h.username,
            tags: h.tags,
            ai_notes: h.ai_notes,
        })
        .collect();
    hosts.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    hosts
}

/// The saved host a command names: by its id, or by its name as `hatoba <name>` finds it
/// (HOST-13), which several hosts may share. Only SSH hosts run commands and copy files, and
/// `hatoba memory` names only them; naming a Telnet host says so (TELNET-01).
pub(crate) fn find_host(vault: &Vault, wanted: &str) -> Result<(String, Host), String> {
    if let Some(host) = vault.get(wanted).and_then(Item::as_host) {
        return Ok((wanted.to_owned(), host.clone()));
    }
    let key = wanted.trim().to_lowercase();
    let mut named: Vec<(String, Host)> = vault
        .hosts()
        .into_iter()
        .filter(|(_, h)| h.name.trim().to_lowercase() == key)
        .collect();
    match named.len() {
        1 => Ok(named.remove(0)),
        0 => Err(match telnet_name(vault, wanted, &key) {
            Some(name) => format!(
                "{wanted} is a Telnet host, and exec, cp and memory work only on SSH hosts. Run \
                 hatoba {name} to open it in Hatoba."
            ),
            None => {
                format!("no saved host is named {wanted}. Run hatoba hosts to see their names.")
            }
        }),
        n => {
            named.sort_by(|a, b| a.0.cmp(&b.0));
            let ids: Vec<&str> = named.iter().map(|(id, _)| id.as_str()).collect();
            Err(format!(
                "{n} saved hosts are named {wanted}. Name one by its id instead: {}.",
                ids.join(", ")
            ))
        }
    }
}

/// The name of the Telnet host `wanted` names by its id or its name, which `hatoba <name>` opens.
fn telnet_name(vault: &Vault, wanted: &str, key: &str) -> Option<String> {
    if let Some(host) = vault.get(wanted).and_then(Item::as_telnet_host) {
        return Some(host.name.trim().to_owned());
    }
    vault
        .telnet_hosts()
        .into_iter()
        .map(|(_, h)| h.name.trim().to_owned())
        .find(|name| name.to_lowercase() == key)
}
