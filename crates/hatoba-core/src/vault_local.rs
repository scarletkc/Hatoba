//! Device-local state and preferences on [`Vault`] (plaintext and [`SealedLocal`]).

use zeroize::Zeroizing;

use crate::crypto::{self, AAD_AI_ALLOWANCES, AAD_CHATGPT_SESSIONS, AAD_RECENT_TARGETS, seal};
use crate::error::{Error, Result};
use crate::store::{StoreOps, meta};
use crate::vault::Vault;

/// Device-local values kept in the meta table, sealed under the vault key and never synced. They
/// are read and written only through [`Vault::sealed_local`] and [`Vault::set_sealed_local`],
/// and a join that changes the vault key seals each one in [`SealedLocal::ALL`] again, so a new
/// value needs nothing more than a variant here, listed in `ALL`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum SealedLocal {
    /// The shell's list of recent quick-connect targets (HOST-12), which names hosts.
    RecentTargets,
    /// The shell's AI approvals saved on this device (AI-19), which name hosts.
    AiAllowances,
    /// This device's ChatGPT sessions (AI-52), which hold tokens.
    ChatgptSessions,
}

impl SealedLocal {
    pub(crate) const ALL: [Self; 3] = [
        Self::RecentTargets,
        Self::AiAllowances,
        Self::ChatgptSessions,
    ];

    pub(crate) const fn meta_key(self) -> &'static str {
        match self {
            Self::RecentTargets => meta::RECENT_TARGETS,
            Self::AiAllowances => meta::AI_ALLOWANCES,
            Self::ChatgptSessions => meta::CHATGPT_SESSIONS,
        }
    }

    pub(crate) const fn aad(self) -> &'static [u8] {
        match self {
            Self::RecentTargets => AAD_RECENT_TARGETS.as_bytes(),
            Self::AiAllowances => AAD_AI_ALLOWANCES.as_bytes(),
            Self::ChatgptSessions => AAD_CHATGPT_SESSIONS.as_bytes(),
        }
    }
}

impl Vault {
    /// The bundled Worker version whose "update available" notice the user dismissed on this
    /// device (spec §6.7, Upgrades). The notice comes back with the next bundled version.
    #[must_use]
    pub fn worker_update_dismissed(&self) -> Option<String> {
        self.store
            .get_meta(meta::WORKER_UPDATE_DISMISSED)
            .ok()
            .flatten()
    }

    /// Records that the user dismissed the "update available" notice for `bundled`.
    ///
    /// # Errors
    /// Storage errors.
    pub fn set_worker_update_dismissed(&mut self, bundled: &str) -> Result<()> {
        self.store.set_meta(meta::WORKER_UPDATE_DISMISSED, bundled)
    }

    /// Records when a host was last connected to. Device-local; never synced.
    ///
    /// # Errors
    /// Storage errors.
    pub fn set_last_connected(&mut self, host_id: &str, at_ms: i64) -> Result<()> {
        self.store.set_last_connected(host_id, at_ms)
    }

    /// When a host was last connected to on this device.
    #[must_use]
    pub fn last_connected(&self, host_id: &str) -> Option<i64> {
        self.store.last_connected(host_id).ok().flatten()
    }

    /// Records the operating system a host's server reported, or clears it with `None`
    /// (HOST-11). Device-local; never synced.
    ///
    /// # Errors
    /// Storage errors.
    pub fn set_host_os(&mut self, host_id: &str, os: Option<&str>) -> Result<()> {
        self.store.set_host_os(host_id, os)
    }

    /// The operating system a host's server reported on its last connection from this device.
    #[must_use]
    pub fn host_os(&self, host_id: &str) -> Option<String> {
        self.store.host_os(host_id).ok().flatten()
    }

    /// Turns the resource usage in a host's terminals on or off (TERM-12). Device-local; never
    /// synced.
    ///
    /// # Errors
    /// Storage errors.
    pub fn set_host_show_stats(&mut self, host_id: &str, on: bool) -> Result<()> {
        self.store.set_host_show_stats(host_id, on)
    }

    /// Whether a host's terminals show the server's resource usage on this device. Off until
    /// turned on.
    #[must_use]
    pub fn host_show_stats(&self, host_id: &str) -> bool {
        self.store.host_show_stats(host_id).unwrap_or(false)
    }

    /// Leaves a host or a group out of the dashboard on this device, or brings it back
    /// (HOST-14). Device-local; never synced.
    ///
    /// # Errors
    /// Storage errors.
    pub fn set_dashboard_shown(&mut self, item_id: &str, shown: bool) -> Result<()> {
        self.store.set_dashboard_off(item_id, !shown)
    }

    /// Whether the dashboard shows a host or a group on this device. On until turned off.
    #[must_use]
    pub fn dashboard_shown(&self, item_id: &str) -> bool {
        !self.store.dashboard_off(item_id).unwrap_or(false)
    }

    /// The opaque device-local UI preferences (plaintext JSON owned by the shell). Works while
    /// locked.
    ///
    /// # Errors
    /// Storage errors.
    pub fn local_prefs(&self) -> Result<Option<String>> {
        self.store.get_meta(meta::LOCAL_PREFS)
    }

    /// Stores the device-local UI preferences. Works while locked. Must not contain secrets.
    ///
    /// # Errors
    /// Storage errors.
    pub fn set_local_prefs(&mut self, json: &str) -> Result<()> {
        self.store.set_meta(meta::LOCAL_PREFS, json)
    }

    /// The device-local state of the star prompt (plaintext JSON owned by the shell). Works
    /// while locked.
    ///
    /// # Errors
    /// Storage errors.
    pub fn star_prompt(&self) -> Result<Option<String>> {
        self.store.get_meta(meta::STAR_PROMPT)
    }

    /// Stores the star prompt state. Works while locked. Must not contain secrets.
    ///
    /// # Errors
    /// Storage errors.
    pub fn set_star_prompt(&mut self, json: &str) -> Result<()> {
        self.store.set_meta(meta::STAR_PROMPT, json)
    }

    /// The device-local state of the MCP servers (plaintext JSON owned by the shell): which are
    /// enabled on this device and what they always allow (spec §13.9). Works while locked.
    ///
    /// # Errors
    /// Storage errors.
    pub fn mcp_device_state(&self) -> Result<Option<String>> {
        self.store.get_meta(meta::MCP_DEVICE_STATE)
    }

    /// Stores the device-local MCP server state. Works while locked. Must not contain secrets.
    ///
    /// # Errors
    /// Storage errors.
    pub fn set_mcp_device_state(&mut self, json: &str) -> Result<()> {
        self.store.set_meta(meta::MCP_DEVICE_STATE, json)
    }

    /// The device-local list of recent quick-connect targets (opaque JSON owned by the shell,
    /// HOST-12). Never synced, and sealed under the vault key because it names hosts. A value
    /// this vault key cannot open reads as `None`.
    ///
    /// # Errors
    /// [`Error::Locked`], storage errors.
    pub fn recent_targets(&self) -> Result<Option<String>> {
        self.sealed_local(SealedLocal::RecentTargets)
    }

    /// Stores the recent quick-connect targets, sealed under the vault key.
    ///
    /// # Errors
    /// [`Error::Locked`], storage errors.
    pub fn set_recent_targets(&mut self, json: &str) -> Result<()> {
        self.set_sealed_local(SealedLocal::RecentTargets, json)
    }

    /// The AI approvals saved on this device (opaque JSON owned by the shell, AI-19). Never
    /// synced, and sealed under the vault key because they name hosts. A value this vault key
    /// cannot open reads as `None`.
    ///
    /// # Errors
    /// [`Error::Locked`], storage errors.
    pub fn ai_allowances(&self) -> Result<Option<String>> {
        self.sealed_local(SealedLocal::AiAllowances)
    }

    /// Stores the AI approvals saved on this device, sealed under the vault key.
    ///
    /// # Errors
    /// [`Error::Locked`], storage errors.
    pub fn set_ai_allowances(&mut self, json: &str) -> Result<()> {
        self.set_sealed_local(SealedLocal::AiAllowances, json)
    }

    /// This device's ChatGPT sessions and `ext_agent_host_id` (opaque JSON owned by the shell,
    /// AI-52). Never synced, and sealed under the vault key because it holds tokens. A value this
    /// vault key cannot open reads as `None`.
    ///
    /// # Errors
    /// [`Error::Locked`], storage errors.
    pub fn chatgpt_sessions(&self) -> Result<Option<Zeroizing<String>>> {
        Ok(self
            .sealed_local(SealedLocal::ChatgptSessions)?
            .map(Zeroizing::new))
    }

    /// Stores this device's ChatGPT sessions, sealed under the vault key.
    ///
    /// # Errors
    /// [`Error::Locked`], storage errors.
    pub fn set_chatgpt_sessions(&mut self, json: &str) -> Result<()> {
        self.set_sealed_local(SealedLocal::ChatgptSessions, json)
    }

    /// A [`SealedLocal`] value; one this vault key cannot open reads as `None`.
    fn sealed_local(&self, which: SealedLocal) -> Result<Option<String>> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        let Some(sealed) = self.store.get_meta(which.meta_key())? else {
            return Ok(None);
        };
        Ok(crypto::open_json(&unlocked.vault_key, which.aad(), &sealed)
            .ok()
            .and_then(|plain| String::from_utf8(plain.to_vec()).ok()))
    }

    /// Stores a [`SealedLocal`] value, sealed under the vault key.
    fn set_sealed_local(&mut self, which: SealedLocal, json: &str) -> Result<()> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        let sealed = seal(&unlocked.vault_key, which.aad(), json.as_bytes())?;
        self.store.set_meta(which.meta_key(), &sealed.to_json())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::random_key;
    use crate::test_support::{TEST_PASSWORD as PW, host_item as host, unlocked_vault};

    #[test]
    fn local_state_and_prefs_work_while_locked() {
        let (mut vault, _clock) = unlocked_vault();
        let id = vault.put(None, host("h")).unwrap();
        vault.set_last_connected(&id, 123).unwrap();
        vault.lock();
        assert_eq!(vault.last_connected(&id), Some(123));
        assert_eq!(vault.local_prefs().unwrap(), None);
        vault.set_local_prefs(r#"{"window":{"w":1200}}"#).unwrap();
        assert_eq!(
            vault.local_prefs().unwrap().as_deref(),
            Some(r#"{"window":{"w":1200}}"#)
        );
        assert_eq!(vault.star_prompt().unwrap(), None);
        vault.set_star_prompt(r#"{"done":true}"#).unwrap();
        assert_eq!(
            vault.star_prompt().unwrap().as_deref(),
            Some(r#"{"done":true}"#)
        );
        assert_eq!(vault.mcp_device_state().unwrap(), None);
        vault.set_mcp_device_state(r#"{"servers":{}}"#).unwrap();
        assert_eq!(
            vault.mcp_device_state().unwrap().as_deref(),
            Some(r#"{"servers":{}}"#)
        );
        assert_eq!(vault.worker_update_dismissed(), None);
        vault.set_worker_update_dismissed("0.3.0").unwrap();
        assert_eq!(vault.worker_update_dismissed().as_deref(), Some("0.3.0"));
        // last_connected is not an item and never dirties anything.
        assert_eq!(vault.pending_count(), 2);
    }

    #[test]
    fn recent_targets_are_sealed_and_need_the_vault_unlocked() {
        let (mut vault, _clock) = unlocked_vault();
        assert_eq!(vault.recent_targets().unwrap(), None);
        let json = r#"[{"address":"db.internal.example","port":22,"username":"deploy"}]"#;
        vault.set_recent_targets(json).unwrap();
        assert_eq!(vault.recent_targets().unwrap().as_deref(), Some(json));
        let raw = vault.store.get_meta(meta::RECENT_TARGETS).unwrap().unwrap();
        assert!(!raw.contains("db.internal.example"), "{raw}");
        // Device-local: nothing to push.
        assert_eq!(vault.pending_count(), 1);

        vault.lock();
        assert!(matches!(vault.recent_targets(), Err(Error::Locked)));
        assert!(matches!(vault.set_recent_targets("[]"), Err(Error::Locked)));
        vault.unlock(PW).unwrap();
        assert_eq!(vault.recent_targets().unwrap().as_deref(), Some(json));

        // Sealed under another key (a vault replaced since): reads as nothing.
        vault
            .store
            .set_meta(
                meta::RECENT_TARGETS,
                &seal(&[7u8; 32], AAD_RECENT_TARGETS.as_bytes(), b"[]")
                    .unwrap()
                    .to_json(),
            )
            .unwrap();
        assert_eq!(vault.recent_targets().unwrap(), None);
    }

    #[test]
    fn ai_allowances_are_sealed_and_need_the_vault_unlocked() {
        let (mut vault, _clock) = unlocked_vault();
        assert_eq!(vault.ai_allowances().unwrap(), None);
        let json = r#"[{"kind":"command","target":"deploy@db.internal.example:22","value":"git"}]"#;
        vault.set_ai_allowances(json).unwrap();
        assert_eq!(vault.ai_allowances().unwrap().as_deref(), Some(json));
        let raw = vault.store.get_meta(meta::AI_ALLOWANCES).unwrap().unwrap();
        assert!(!raw.contains("db.internal.example"), "{raw}");
        // Device-local: nothing to push.
        assert_eq!(vault.pending_count(), 1);

        vault.lock();
        assert!(matches!(vault.ai_allowances(), Err(Error::Locked)));
        assert!(matches!(vault.set_ai_allowances("[]"), Err(Error::Locked)));
        vault.unlock(PW).unwrap();
        assert_eq!(vault.ai_allowances().unwrap().as_deref(), Some(json));

        // Sealed under another key (a vault replaced since): reads as nothing.
        vault
            .store
            .set_meta(
                meta::AI_ALLOWANCES,
                &seal(&random_key().unwrap(), AAD_AI_ALLOWANCES.as_bytes(), b"[]")
                    .unwrap()
                    .to_json(),
            )
            .unwrap();
        assert_eq!(vault.ai_allowances().unwrap(), None);
    }

    #[test]
    fn chatgpt_sessions_are_sealed_and_need_the_vault_unlocked() {
        let (mut vault, _clock) = unlocked_vault();
        assert_eq!(vault.chatgpt_sessions().unwrap(), None);
        let json = r#"{"host_id":"urn:uuid:1","sessions":{"p":{"access_token":"tok-secret"}}}"#;
        vault.set_chatgpt_sessions(json).unwrap();
        let read = vault.chatgpt_sessions().unwrap().unwrap();
        assert_eq!(read.as_str(), json);
        let raw = vault
            .store
            .get_meta(meta::CHATGPT_SESSIONS)
            .unwrap()
            .unwrap();
        assert!(!raw.contains("tok-secret"), "{raw}");
        // Device-local: nothing to push.
        assert_eq!(vault.pending_count(), 1);

        vault.lock();
        assert!(matches!(vault.chatgpt_sessions(), Err(Error::Locked)));
        assert!(matches!(
            vault.set_chatgpt_sessions("{}"),
            Err(Error::Locked)
        ));
        vault.unlock(PW).unwrap();
        assert_eq!(vault.chatgpt_sessions().unwrap().unwrap().as_str(), json);
    }

    #[test]
    fn host_os_is_device_local() {
        let (mut vault, _clock) = unlocked_vault();
        let id = vault.put(None, host("h")).unwrap();
        let row = vault.store.item_row(&id).unwrap();
        let pending = vault.pending_count();
        assert_eq!(vault.host_os(&id), None);
        vault.set_last_connected(&id, 123).unwrap();
        vault.set_host_os(&id, Some("ubuntu")).unwrap();
        assert_eq!(vault.host_os(&id).as_deref(), Some("ubuntu"));
        // The host item, its `updated_at` included, is untouched, so nothing is left to sync.
        assert_eq!(vault.store.item_row(&id).unwrap(), row);
        assert_eq!(vault.pending_count(), pending);
        vault.lock();
        assert_eq!(vault.host_os(&id).as_deref(), Some("ubuntu"));
        vault.set_host_os(&id, None).unwrap();
        assert_eq!(vault.host_os(&id), None);
        assert_eq!(vault.last_connected(&id), Some(123));
    }

    #[test]
    fn show_stats_is_device_local_and_off_by_default() {
        let (mut vault, _clock) = unlocked_vault();
        let id = vault.put(None, host("h")).unwrap();
        let row = vault.store.item_row(&id).unwrap();
        let pending = vault.pending_count();
        assert!(!vault.host_show_stats(&id));
        vault.set_host_show_stats(&id, true).unwrap();
        assert!(vault.host_show_stats(&id));
        // The host item is untouched, so nothing is left to sync.
        assert_eq!(vault.store.item_row(&id).unwrap(), row);
        assert_eq!(vault.pending_count(), pending);
        vault.set_host_show_stats(&id, false).unwrap();
        assert!(!vault.host_show_stats(&id));
    }

    #[test]
    fn dashboard_switch_is_device_local_and_on_by_default() {
        let (mut vault, _clock) = unlocked_vault();
        let id = vault.put(None, host("h")).unwrap();
        let row = vault.store.item_row(&id).unwrap();
        let pending = vault.pending_count();
        assert!(vault.dashboard_shown(&id));
        vault.set_dashboard_shown(&id, false).unwrap();
        assert!(!vault.dashboard_shown(&id));
        assert_eq!(vault.store.item_row(&id).unwrap(), row);
        assert_eq!(vault.pending_count(), pending);
        vault.set_dashboard_shown(&id, true).unwrap();
        assert!(vault.dashboard_shown(&id));
    }
}
