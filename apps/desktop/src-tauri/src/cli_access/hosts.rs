//! `hatoba hosts add`, `edit` and `delete` (HOST-15): other programs save, change and delete the
//! saved SSH hosts through the functions the host editor and the Hosts page use
//! (`commands::hosts`), so the command checks and does what they do. No password leaves Hatoba:
//! `hosts --json` gives none, and what Settings lists of a change says `--password-stdin` in its
//! place.

use hatoba_cli::protocol::{AuthKind, EnvVar, HostChanges, HostInfo};
use hatoba_core::model::{Host, HostAuth, HostProxy, Item};
use hatoba_core::vault::Vault;

use crate::commands::hosts::{check_item_size, host_from_input, save_group, save_host};
use crate::convert::proxy_choice;
use crate::dto::{self, GroupInput, HostEnvVar, HostInput, ProxyMode};

use super::find_host;

/// A saved host as `hatoba hosts` lists it, without its credentials.
pub(super) fn info(vault: &Vault, id: String, h: Host) -> HostInfo {
    HostInfo {
        auth: match &h.auth {
            HostAuth::Password { password } if !password.is_empty() => AuthKind::Password,
            HostAuth::Password { .. } | HostAuth::Ask => AuthKind::Ask,
            HostAuth::Key { .. } => AuthKind::Key,
            HostAuth::Agent => AuthKind::Agent,
        },
        key: match &h.auth {
            HostAuth::Key { key_id } => vault
                .get(key_id)
                .and_then(Item::as_key)
                .map(|k| k.name.clone()),
            _ => None,
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
        proxy: match &h.proxy {
            HostProxy::DeviceDefault => "default".to_owned(),
            HostProxy::Direct => "direct".to_owned(),
            // A proxy deleted on another device is left as the id the host still names.
            HostProxy::Proxy { proxy_id } => vault
                .get(proxy_id)
                .and_then(Item::as_proxy)
                .map_or_else(|| proxy_id.clone(), |p| p.name.clone()),
        },
        env: h
            .env
            .into_iter()
            .map(|v| EnvVar {
                name: v.name,
                value: v.value,
            })
            .collect(),
        favorite: h.favorite,
        id,
        name: h.name,
        address: h.address,
        port: h.port,
        username: h.username,
        tags: h.tags,
        note: h.note,
        ai_notes: h.ai_notes,
    }
}

/// What `hosts add` or `edit` saves: the host editor's form for the host, and the group to make
/// first when the changes name one that no group has.
#[derive(Debug)]
pub(super) struct Plan {
    pub(super) input: HostInput,
    new_group: Option<String>,
    /// The host signs in with a password that the changes don't hold, so the window asks for it.
    pub(super) needs_password: bool,
}

impl Plan {
    /// The name the host is saved under, as the editor gives a host without one.
    pub(super) fn name(&self) -> String {
        match self.input.name.trim() {
            "" => self.input.address.trim().to_owned(),
            name => name.to_owned(),
        }
    }
}

/// The form for `edited`, or for a new host as **New Host** starts it with `local_user` as its
/// user, with `changes` made to it. Names that name nothing, or several things, fail here.
pub(super) fn plan(
    vault: &Vault,
    edited: Option<(&str, &Host)>,
    changes: &HostChanges,
    local_user: &str,
) -> Result<Plan, String> {
    let mut input = match edited {
        Some((id, host)) => form(id, host),
        None => HostInput {
            id: None,
            name: String::new(),
            address: String::new(),
            port: 22,
            username: local_user.to_owned(),
            auth_kind: dto::AuthKind::Password,
            password: None,
            key_id: None,
            group_id: None,
            tags: Vec::new(),
            favorite: false,
            jump_host_id: None,
            proxy_mode: ProxyMode::DeviceDefault,
            proxy_id: None,
            note: String::new(),
            ai_notes: String::new(),
            env: Vec::new(),
        },
    };
    let c = changes;
    if let Some(name) = &c.name {
        input.name.clone_from(name);
    }
    if let Some(address) = &c.address {
        input.address.clone_from(address);
    }
    if let Some(port) = c.port {
        input.port = port;
    }
    if let Some(username) = &c.username {
        input.username.clone_from(username);
    }
    if let Some(auth) = c.auth {
        input.auth_kind = match auth {
            AuthKind::Password => dto::AuthKind::Password,
            AuthKind::Key => dto::AuthKind::Key,
            AuthKind::Agent => dto::AuthKind::Agent,
            AuthKind::Ask => dto::AuthKind::Ask,
        };
    }
    if let Some(key) = &c.key {
        let keys = vault.keys();
        input.key_id = Some(named(&keys, |k| &k.name, key, "key", KEYS)?);
    }
    input.password.clone_from(&c.password);
    if let Some(jump) = &c.jump {
        input.jump_host_id = Some(find_host(vault, jump)?.0);
    }
    if c.no_jump {
        input.jump_host_id = None;
    }
    if let Some(proxy) = &c.proxy {
        (input.proxy_mode, input.proxy_id) = match proxy.as_str() {
            "default" => (ProxyMode::DeviceDefault, None),
            "direct" => (ProxyMode::Direct, None),
            _ => {
                let proxies = vault.proxies();
                let id = named(&proxies, |p| &p.name, proxy, "proxy", PROXIES)?;
                (ProxyMode::Proxy, Some(id))
            }
        };
    }
    let mut new_group = None;
    if let Some(group) = &c.group {
        input.group_id = group_named(vault, group)?;
        if input.group_id.is_none() {
            new_group = Some(group.trim().to_owned());
        }
    }
    if c.no_group {
        input.group_id = None;
    }
    input
        .tags
        .retain(|tag| !c.untags.iter().any(|u| u.trim() == tag.as_str()));
    input.tags.extend(c.tags.iter().cloned());
    input
        .env
        .retain(|v| !c.unset_env.iter().any(|name| name.trim() == v.name.trim()));
    for set in &c.env {
        match input
            .env
            .iter_mut()
            .find(|v| v.name.trim() == set.name.trim())
        {
            Some(v) => v.value.clone_from(&set.value),
            None => input.env.push(HostEnvVar {
                name: set.name.clone(),
                value: set.value.clone(),
            }),
        }
    }
    if let Some(note) = &c.note {
        input.note.clone_from(note);
    }
    if let Some(notes) = &c.ai_notes {
        input.ai_notes.clone_from(notes);
    }
    if let Some(favorite) = c.favorite {
        input.favorite = favorite;
    }
    // As in the editor: behind a jump host the proxy is ignored, so one deleted on another device
    // goes back to the device default instead of blocking the save.
    if input.jump_host_id.is_some()
        && input.proxy_mode == ProxyMode::Proxy
        && input
            .proxy_id
            .as_deref()
            .is_none_or(|p| vault.get(p).and_then(Item::as_proxy).is_none())
    {
        (input.proxy_mode, input.proxy_id) = (ProxyMode::DeviceDefault, None);
    }
    let needs_password = c.auth == Some(AuthKind::Password) && c.password.is_none();
    Ok(Plan {
        input,
        new_group,
        needs_password,
    })
}

/// The form the host editor opens `host` with, which saves it again as it is: a password it
/// doesn't hold keeps the saved one (HOST-08).
fn form(id: &str, host: &Host) -> HostInput {
    let (auth_kind, key_id) = match &host.auth {
        HostAuth::Password { .. } => (dto::AuthKind::Password, None),
        HostAuth::Key { key_id } => (dto::AuthKind::Key, Some(key_id.clone())),
        HostAuth::Agent => (dto::AuthKind::Agent, None),
        HostAuth::Ask => (dto::AuthKind::Ask, None),
    };
    let (proxy_mode, proxy_id) = proxy_choice(&host.proxy);
    HostInput {
        id: Some(id.to_owned()),
        name: host.name.clone(),
        address: host.address.clone(),
        port: host.port,
        username: host.username.clone(),
        auth_kind,
        password: None,
        key_id,
        group_id: host.group_id.clone(),
        tags: host.tags.clone(),
        favorite: host.favorite,
        jump_host_id: host.jump_host_id.clone(),
        proxy_mode,
        proxy_id,
        note: host.note.clone(),
        ai_notes: host.ai_notes.clone(),
        env: host
            .env
            .iter()
            .map(|v| HostEnvVar {
                name: v.name.clone(),
                value: v.value.clone(),
            })
            .collect(),
    }
}

/// Where a message about a key sends the reader.
const KEYS: &str = "The Keys page in Hatoba lists them.";
const PROXIES: &str = "Settings > Proxies in Hatoba lists them.";

/// The id of the item that `wanted` names among `items`: its id, or its name as `find_host`
/// matches a host's, which several may share. `what` is the kind of item, such as `key`.
fn named<T>(
    items: &[(String, T)],
    name: impl Fn(&T) -> &str,
    wanted: &str,
    what: &str,
    listed: &str,
) -> Result<String, String> {
    if let Some((id, _)) = items.iter().find(|(id, _)| id == wanted) {
        return Ok(id.clone());
    }
    let key = wanted.trim().to_lowercase();
    let mut ids: Vec<&str> = items
        .iter()
        .filter(|(_, item)| name(item).trim().to_lowercase() == key)
        .map(|(id, _)| id.as_str())
        .collect();
    ids.sort_unstable();
    match ids[..] {
        [id] => Ok(id.to_owned()),
        [] => Err(format!("no {what} in Hatoba is named {wanted}. {listed}")),
        _ => Err(format!(
            "{} {what}s in Hatoba are named {wanted}. Name one by its id instead: {}.",
            ids.len(),
            ids.join(", ")
        )),
    }
}

/// The group that `wanted` names by its id or its name, or `None` when no group has that name.
fn group_named(vault: &Vault, wanted: &str) -> Result<Option<String>, String> {
    if wanted.trim().is_empty() {
        return Err(
            "--group needs a group's name; --no-group takes the host out of its group".into(),
        );
    }
    let groups = vault.groups();
    let key = wanted.trim().to_lowercase();
    if !groups
        .iter()
        .any(|(id, g)| id == wanted || g.name.trim().to_lowercase() == key)
    {
        return Ok(None);
    }
    named(&groups, |g| &g.name, wanted, "group", "").map(Some)
}

/// Whether the host editor would save the plan, with its message when it wouldn't.
pub(super) fn check(vault: &Vault, plan: &Plan) -> Result<(), String> {
    let existing = plan
        .input
        .id
        .as_deref()
        .and_then(|id| vault.get(id))
        .and_then(Item::as_host);
    let host = host_from_input(&plan.input, existing, vault).map_err(|e| e.detail)?;
    check_item_size(vault, &Item::Host(host)).map_err(|e| e.detail)
}

/// Saves the plan, after the group it makes, and returns the host's id. When the host can't be
/// saved, the group goes too, so nothing changes.
pub(super) fn save(vault: &mut Vault, mut plan: Plan) -> Result<String, String> {
    let group = match &plan.new_group {
        Some(name) => {
            // A new group goes last, as from the group dialog.
            let sort = i32::try_from(vault.groups().len()).unwrap_or(i32::MAX);
            let input = GroupInput {
                id: None,
                name: name.clone(),
                parent_id: None,
                sort,
            };
            let id = save_group(vault, &input).map_err(|e| e.detail)?;
            plan.input.group_id = Some(id.clone());
            Some(id)
        }
        None => None,
    };
    save_host(vault, &plan.input).map_err(|e| {
        if let Some(id) = &group
            && let Err(undo) = vault.delete_group(id)
        {
            tracing::warn!("hatoba hosts could not remove the group it made: {undo}");
        }
        e.detail
    })
}

/// What `hosts delete` prints.
pub(super) fn deleted(name: &str, forwards: usize) -> String {
    match forwards {
        0 => format!("Deleted the host \"{name}\"."),
        1 => format!("Deleted the host \"{name}\" and its forwarding rule."),
        n => format!("Deleted the host \"{name}\" and its {n} forwarding rules."),
    }
}

/// What Settings lists of a change: the command and its options, as they were given, with
/// `--password-stdin` for a password.
pub(super) fn describe(word: &str, c: &HostChanges) -> String {
    let mut out = vec![format!("hosts {word}")];
    if word == "add" {
        let address = c.address.as_deref().unwrap_or_default();
        let address = if address.contains(':') {
            format!("[{address}]")
        } else {
            address.to_owned()
        };
        let user = c
            .username
            .as_deref()
            .map(|u| format!("{u}@"))
            .unwrap_or_default();
        let port = c.port.map(|p| format!(":{p}")).unwrap_or_default();
        out.push(shown(&format!("{user}{address}{port}")));
    }
    let mut option = |name: &str, value: Option<&str>| match value {
        Some(value) => out.push(format!("{name} {}", shown(value))),
        None => out.push(name.to_owned()),
    };
    if let Some(v) = &c.name {
        option("--name", Some(v));
    }
    if word != "add" {
        if let Some(v) = &c.address {
            option("--address", Some(v));
        }
        if let Some(v) = c.port {
            option("--port", Some(&v.to_string()));
        }
        if let Some(v) = &c.username {
            option("--user", Some(v));
        }
    }
    let implied = match c.auth {
        Some(AuthKind::Key) => c.key.is_some(),
        Some(AuthKind::Password) => c.password.is_some(),
        _ => false,
    };
    if let Some(auth) = c.auth.filter(|_| !implied) {
        let word = match auth {
            AuthKind::Password => "password",
            AuthKind::Key => "key",
            AuthKind::Agent => "agent",
            AuthKind::Ask => "ask",
        };
        option("--auth", Some(word));
    }
    if let Some(v) = &c.key {
        option("--key", Some(v));
    }
    if c.password.is_some() {
        option("--password-stdin", None);
    }
    if let Some(v) = &c.jump {
        option("--jump", Some(v));
    }
    if c.no_jump {
        option("--no-jump", None);
    }
    if let Some(v) = &c.proxy {
        option("--proxy", Some(v));
    }
    if let Some(v) = &c.group {
        option("--group", Some(v));
    }
    if c.no_group {
        option("--no-group", None);
    }
    for v in &c.tags {
        option("--tag", Some(v));
    }
    for v in &c.untags {
        option("--untag", Some(v));
    }
    for v in &c.env {
        option("--env", Some(&format!("{}={}", v.name, v.value)));
    }
    for v in &c.unset_env {
        option("--unset-env", Some(v));
    }
    if let Some(v) = &c.note {
        option("--note", Some(v));
    }
    if let Some(v) = &c.ai_notes {
        option("--ai-notes", Some(v));
    }
    match c.favorite {
        Some(true) => option("--favorite", None),
        Some(false) => option("--no-favorite", None),
        None => {}
    }
    out.join(" ")
}

/// The longest value Settings shows of an option, in characters.
const SHOWN_CHARS: usize = 40;

/// A value as Settings lists it: in quotes when it is empty or holds a space, and cut short.
fn shown(value: &str) -> String {
    let mut text: String = value.chars().take(SHOWN_CHARS).collect();
    if value.chars().count() > SHOWN_CHARS {
        text.push('…');
    }
    if text.is_empty() || text.contains(char::is_whitespace) || text.contains('"') {
        format!("\"{}\"", text.replace('"', "\\\""))
    } else {
        text
    }
}
