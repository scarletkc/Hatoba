//! Device-local preference helpers.

use hatoba_core::vault::Vault;

use crate::dto::LocalPrefs;
use crate::error::AppResult;

/// Reads this device's preferences from `vault`, returning defaults when none are stored yet.
pub fn device_prefs(vault: &Vault) -> AppResult<LocalPrefs> {
    Ok(vault
        .local_prefs()?
        .as_deref()
        .map(LocalPrefs::from_stored)
        .unwrap_or_default())
}
