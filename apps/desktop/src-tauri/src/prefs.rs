//! Device-local preferences.

use hatoba_core::vault::Vault;

use crate::dto::LocalPrefs;
use crate::error::AppResult;

/// Readable while the vault is locked; returns defaults when none are stored yet.
pub(crate) fn read(vault: &Vault) -> AppResult<LocalPrefs> {
    Ok(vault
        .local_prefs()?
        .as_deref()
        .map(LocalPrefs::from_stored)
        .unwrap_or_default())
}
