//! AI approvals saved on this device (AI-19): command prefixes and tools allowed on a host, and
//! domains `fetch_url` may fetch. The frontend decides with them which calls ask; this module only
//! keeps them, sealed in the vault's meta table and never synced.

use hatoba_core::vault::Vault;
use tauri::State;

use crate::dto::{AiAllowance, AiAllowanceKind};
use crate::error::{AppError, AppResult};
use crate::state::AppState;

/// The tools a [`AiAllowanceKind::Tool`] approval can name.
const TOOLS: [&str; 3] = ["edit_file", "write_file", "send_input"];
/// Longest value or host reference kept, in characters.
const MAX_CHARS: usize = 1000;

/// A text field of an approval, trimmed; `None` when empty.
fn field(value: Option<&str>, name: &str) -> AppResult<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if value.chars().count() > MAX_CHARS || value.chars().any(char::is_control) {
        return Err(AppError::invalid(name, format!("the {name} is not valid")));
    }
    Ok(Some(value.to_owned()))
}

/// Checks an approval and puts it in the form it is stored and compared in.
fn normalize(a: &AiAllowance) -> AppResult<AiAllowance> {
    let host_id = field(a.host_id.as_deref(), "host_id")?;
    let target = field(a.target.as_deref(), "target")?;
    let value =
        field(Some(&a.value), "value")?.ok_or_else(|| AppError::invalid("value", "it is empty"))?;
    let (host_id, target, value) = match a.kind {
        AiAllowanceKind::Command | AiAllowanceKind::Tool => {
            // A host, named one way.
            if host_id.is_some() == target.is_some() {
                return Err(AppError::invalid(
                    "host_id",
                    "name either a saved host or a quick-connect target",
                ));
            }
            if a.kind == AiAllowanceKind::Tool && !TOOLS.contains(&value.as_str()) {
                return Err(AppError::invalid("value", "not a tool that can be allowed"));
            }
            (host_id, target, value)
        }
        AiAllowanceKind::Domain => {
            if host_id.is_some() || target.is_some() {
                return Err(AppError::invalid("host_id", "a domain is not for one host"));
            }
            let value = value.to_ascii_lowercase();
            let bad = |c: char| c.is_whitespace() || "/\\@?#".contains(c);
            if value.contains(bad) {
                return Err(AppError::invalid("value", "not a domain"));
            }
            (None, None, value)
        }
    };
    Ok(AiAllowance {
        kind: a.kind,
        host_id,
        target,
        value,
    })
}

fn load(v: &Vault) -> AppResult<Vec<AiAllowance>> {
    Ok(v.ai_allowances()?
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default())
}

fn store(v: &mut Vault, list: &[AiAllowance]) -> AppResult<()> {
    let json = serde_json::to_string(list).map_err(|e| AppError::internal(e.to_string()))?;
    Ok(v.set_ai_allowances(&json)?)
}

/// Adds an approval unless the list has it already.
fn add(v: &mut Vault, allowance: &AiAllowance) -> AppResult<Vec<AiAllowance>> {
    let allowance = normalize(allowance)?;
    let mut list = load(v)?;
    if !list.contains(&allowance) {
        list.push(allowance);
        store(v, &list)?;
    }
    Ok(list)
}

fn remove(v: &mut Vault, allowance: &AiAllowance) -> AppResult<Vec<AiAllowance>> {
    let allowance = normalize(allowance)?;
    let mut list = load(v)?;
    list.retain(|a| a != &allowance);
    store(v, &list)?;
    Ok(list)
}

/// The AI approvals saved on this device, oldest first.
#[tauri::command]
#[specta::specta]
pub fn ai_allowances_list(state: State<'_, AppState>) -> AppResult<Vec<AiAllowance>> {
    state.with_unlocked(|v| load(v))
}

/// Saves an approval on this device and returns them all.
#[tauri::command]
#[specta::specta]
pub fn ai_allowance_add(
    state: State<'_, AppState>,
    allowance: AiAllowance,
) -> AppResult<Vec<AiAllowance>> {
    state.with_unlocked(|v| add(v, &allowance))
}

/// Removes a saved approval and returns what is left.
#[tauri::command]
#[specta::specta]
pub fn ai_allowance_remove(
    state: State<'_, AppState>,
    allowance: AiAllowance,
) -> AppResult<Vec<AiAllowance>> {
    state.with_unlocked(|v| remove(v, &allowance))
}

#[cfg(test)]
mod tests {
    use hatoba_core::{KdfParams, Vault, crypto};

    use super::*;

    fn allowance(
        kind: AiAllowanceKind,
        host_id: Option<&str>,
        target: Option<&str>,
        value: &str,
    ) -> AiAllowance {
        AiAllowance {
            kind,
            host_id: host_id.map(str::to_owned),
            target: target.map(str::to_owned),
            value: value.to_owned(),
        }
    }

    #[test]
    fn approvals_are_checked_and_normalized() {
        use AiAllowanceKind::{Command, Domain, Tool};
        assert_eq!(
            normalize(&allowance(Command, Some(" h1 "), None, " git pull ")).unwrap(),
            allowance(Command, Some("h1"), None, "git pull")
        );
        assert_eq!(
            normalize(&allowance(Domain, None, Some(""), "Docs.Example.COM")).unwrap(),
            allowance(Domain, None, None, "docs.example.com")
        );
        let field = |a: AiAllowance| normalize(&a).unwrap_err().field.unwrap();
        // A command or tool approval names one host, one way.
        assert_eq!(field(allowance(Command, None, None, "ls")), "host_id");
        assert_eq!(
            field(allowance(Tool, Some("h1"), Some("root@db:22"), "edit_file")),
            "host_id"
        );
        assert_eq!(
            field(allowance(Tool, Some("h1"), None, "run_command")),
            "value"
        );
        assert_eq!(field(allowance(Command, Some("h1"), None, "  ")), "value");
        assert_eq!(
            field(allowance(Command, Some("h1"), None, "ls\nrm")),
            "value"
        );
        assert_eq!(
            field(allowance(Domain, Some("h1"), None, "a.com")),
            "host_id"
        );
        assert_eq!(field(allowance(Domain, None, None, "a.com/x")), "value");
    }

    #[test]
    fn approvals_are_kept_once_and_survive_a_relock() {
        use AiAllowanceKind::{Command, Domain, Tool};
        let pw = crypto::hex_encode(&crypto::random_bytes::<16>().unwrap());
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(&pw, KdfParams::for_tests())
            .unwrap();
        let git = allowance(Command, None, Some("root@db:22"), "git pull");
        let edit = allowance(Tool, Some("h1"), None, "edit_file");
        add(&mut vault, &git).unwrap();
        add(&mut vault, &edit).unwrap();
        // The same approval again, as typed differently, changes nothing.
        let list = add(
            &mut vault,
            &allowance(Command, None, Some("root@db:22"), "git pull "),
        )
        .unwrap();
        assert_eq!(list, vec![git.clone(), edit.clone()]);

        vault.lock();
        assert!(load(&vault).is_err());
        vault.unlock(&pw).unwrap();
        assert_eq!(load(&vault).unwrap(), vec![git.clone(), edit.clone()]);

        assert_eq!(remove(&mut vault, &git).unwrap(), vec![edit.clone()]);
        let docs = allowance(Domain, None, None, "docs.example.com");
        assert_eq!(add(&mut vault, &docs).unwrap(), vec![edit, docs]);
    }
}
