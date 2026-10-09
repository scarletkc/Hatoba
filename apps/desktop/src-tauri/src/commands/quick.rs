//! Quick connect targets (HOST-11): validation and the device-local recent list.

use hatoba_core::vault::Vault;
use tauri::State;

use crate::dto::QuickTarget;
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// Recent targets kept on this device.
const RECENT_LIMIT: usize = 8;

/// Trims the target and checks it can be connected to. An IPv6 address may come in brackets.
pub fn validate(target: QuickTarget) -> AppResult<QuickTarget> {
    let address = target.address.trim();
    let address = address
        .strip_prefix('[')
        .and_then(|a| a.strip_suffix(']'))
        .unwrap_or(address);
    let bad = |c: char| c.is_whitespace() || c.is_control();
    if address.is_empty() {
        return Err(AppError::invalid("address", "the address is empty"));
    }
    if address.len() > 255 || address.contains(bad) || address.contains(['@', '/', '\\']) {
        return Err(AppError::invalid("address", "the address is not valid"));
    }
    // A user name may contain '@' (`ssh user@domain@host`), but never whitespace.
    let username = target.username.trim();
    if username.is_empty() {
        return Err(AppError::invalid("username", "the user name is empty"));
    }
    if username.len() > 255 || username.contains(bad) {
        return Err(AppError::invalid("username", "the user name is not valid"));
    }
    if target.port == 0 {
        return Err(AppError::invalid(
            "port",
            "the port must be between 1 and 65535",
        ));
    }
    Ok(QuickTarget {
        address: address.to_owned(),
        port: target.port,
        username: username.to_owned(),
    })
}

fn same(a: &QuickTarget, b: &QuickTarget) -> bool {
    a.address.eq_ignore_ascii_case(&b.address) && a.port == b.port && a.username == b.username
}

fn load(v: &Vault) -> AppResult<Vec<QuickTarget>> {
    Ok(v.recent_targets()?
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default())
}

fn store(v: &mut Vault, list: &[QuickTarget]) -> AppResult<()> {
    let json = serde_json::to_string(list).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(v.set_recent_targets(&json)?)
}

/// Moves `target` to the front of the list, keeping at most [`RECENT_LIMIT`] entries.
fn push_front(list: &mut Vec<QuickTarget>, target: &QuickTarget) {
    list.retain(|t| !same(t, target));
    list.insert(0, target.clone());
    list.truncate(RECENT_LIMIT);
}

/// Records a target that connected.
pub fn remember(v: &mut Vault, target: &QuickTarget) -> AppResult<()> {
    let mut list = load(v)?;
    push_front(&mut list, target);
    store(v, &list)
}

/// The recent quick-connect targets on this device, newest first.
#[tauri::command]
#[specta::specta]
pub fn recent_targets_list(state: State<'_, AppState>) -> AppResult<Vec<QuickTarget>> {
    state.with_unlocked(|v| load(v))
}

/// Forgets one recent target and returns what is left.
#[tauri::command]
#[specta::specta]
pub fn recent_target_remove(
    state: State<'_, AppState>,
    target: QuickTarget,
) -> AppResult<Vec<QuickTarget>> {
    state.with_unlocked(|v| {
        let mut list = load(v)?;
        list.retain(|t| !same(t, &target));
        store(v, &list)?;
        Ok(list)
    })
}

#[cfg(test)]
mod tests {
    use hatoba_core::{KdfParams, Vault};

    use super::*;

    fn target(address: &str, port: u16, username: &str) -> QuickTarget {
        QuickTarget {
            address: address.to_owned(),
            port,
            username: username.to_owned(),
        }
    }

    #[test]
    fn validation_trims_and_unwraps_ipv6() {
        assert_eq!(
            validate(target(" [2001:db8::1] ", 22, " root ")).unwrap(),
            target("2001:db8::1", 22, "root")
        );
        assert_eq!(
            validate(target("db", 2222, "me@corp.example")).unwrap(),
            target("db", 2222, "me@corp.example")
        );
        let field = |t| validate(t).unwrap_err().field.unwrap();
        assert_eq!(field(target("", 22, "root")), "address");
        assert_eq!(field(target("a b", 22, "root")), "address");
        assert_eq!(field(target("root@db", 22, "root")), "address");
        assert_eq!(field(target("db", 22, "  ")), "username");
        assert_eq!(field(target("db", 22, "ro ot")), "username");
        assert_eq!(field(target("db", 0, "root")), "port");
    }

    #[test]
    fn recent_list_moves_repeats_to_the_front_and_is_capped() {
        let mut list = Vec::new();
        for i in 0..10 {
            push_front(&mut list, &target(&format!("h{i}"), 22, "root"));
        }
        assert_eq!(list.len(), RECENT_LIMIT);
        assert_eq!(list[0].address, "h9");
        push_front(&mut list, &target("H5", 22, "root"));
        assert_eq!(list.len(), RECENT_LIMIT);
        assert_eq!(list[0].address, "H5");
        assert_eq!(
            list.iter()
                .filter(|t| t.address.eq_ignore_ascii_case("h5"))
                .count(),
            1
        );
        // Another port or user is another target.
        push_front(&mut list, &target("h9", 2222, "root"));
        push_front(&mut list, &target("h9", 22, "deploy"));
        assert_eq!(list.iter().filter(|t| t.address == "h9").count(), 3);
    }

    #[test]
    fn remembered_targets_survive_a_relock() {
        const PW: &str = "correct horse battery staple";
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
        remember(&mut vault, &target("db", 22, "root")).unwrap();
        remember(&mut vault, &target("web", 22, "deploy")).unwrap();
        vault.lock();
        assert!(load(&vault).is_err());
        vault.unlock(PW).unwrap();
        assert_eq!(
            load(&vault).unwrap(),
            vec![target("web", 22, "deploy"), target("db", 22, "root")]
        );
    }
}
