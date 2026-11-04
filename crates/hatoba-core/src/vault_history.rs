//! The command history behind command suggestions (spec TERM-14, TERM-15, §5.2).
//!
//! Each saved host, SSH or Telnet, and each local shell keeps a history of its own, named by its
//! owner: the host's item id, or [`LOCAL_SHELL_OWNER_PREFIX`] and the shell's id. A history is one
//! row of the device-local `command_history` table: a JSON array of its commands, newest first,
//! sealed under the vault key with an AAD that names the owner, because a command can hold a
//! secret typed into it. Histories are not items, so they never sync.
//!
//! A host's history goes with the host: the store deletes it with the host's tombstone, whether
//! the deletion was made on this device or pulled by sync
//! ([`StoreOps::upsert_item_row`](crate::store::StoreOps::upsert_item_row)), and a command typed in
//! a tab whose host is no longer saved is not kept.

use zeroize::{Zeroize, Zeroizing};

use crate::crypto::{self, AAD_COMMAND_HISTORY_PREFIX, seal};
use crate::error::{Error, Result};
use crate::model::Item;
use crate::store::{CommandHistoryRow, StoreOps};
use crate::vault::Vault;

/// The most commands a history keeps; adding one more drops the oldest.
pub const COMMAND_HISTORY_MAX: usize = 1000;
/// The longest command a history keeps, in characters.
pub const COMMAND_MAX_CHARS: usize = 4096;
/// The start of a local shell's owner, which the shell's id follows.
pub const LOCAL_SHELL_OWNER_PREFIX: &str = "local:";

fn aad(owner: &str) -> String {
    format!("{AAD_COMMAND_HISTORY_PREFIX}{owner}")
}

/// Opens a history; `None` when `vault_key` cannot open it here or it holds no list of commands.
fn open(vault_key: &[u8; 32], row: &CommandHistoryRow) -> Option<Zeroizing<Vec<String>>> {
    let plain = crypto::open_json(vault_key, aad(&row.owner).as_bytes(), &row.commands).ok()?;
    serde_json::from_slice(&plain).ok().map(Zeroizing::new)
}

/// Whether a history keeps `command`: one that is blank, starts with a space (as the shells'
/// `ignorespace` leaves it out of their own history) or is longer than [`COMMAND_MAX_CHARS`] is
/// left out.
fn keeps(command: &str) -> bool {
    !command.trim().is_empty()
        && !command.starts_with(' ')
        && command.chars().count() <= COMMAND_MAX_CHARS
}

/// Seals every command history again under `new` (the vault joined another vault, flow C). A
/// history `old` cannot open is deleted.
pub(crate) fn reseal_command_history(
    tx: &impl StoreOps,
    old: &[u8; 32],
    new: &[u8; 32],
) -> Result<()> {
    for row in tx.command_history_rows()? {
        let aad = aad(&row.owner);
        match crypto::open_json(old, aad.as_bytes(), &row.commands) {
            Ok(plain) => tx.put_command_history(&CommandHistoryRow {
                commands: seal(new, aad.as_bytes(), &plain)?.to_json(),
                ..row
            })?,
            Err(_) => tx.delete_command_history(&row.owner)?,
        }
    }
    Ok(())
}

impl Vault {
    /// The commands of a host's or local shell's history, newest first. A history this device
    /// has none of, or one this vault key cannot open, is empty.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn command_history(&self, owner: &str) -> Result<Zeroizing<Vec<String>>> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        Ok(self
            .store
            .command_history(owner)?
            .and_then(|row| open(&unlocked.vault_key, &row))
            .unwrap_or_else(|| Zeroizing::new(Vec::new())))
    }

    /// Puts a command at the top of a host's or local shell's history: one already there moves
    /// up, and past [`COMMAND_HISTORY_MAX`] the oldest goes. A command that is blank, starts with
    /// a space or is longer than [`COMMAND_MAX_CHARS`] is not kept, and neither is one for an
    /// owner that is neither a saved host nor a local shell, such as a tab whose host was deleted.
    /// Returns whether it kept the command.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn command_history_add(&mut self, owner: &str, command: &str) -> Result<bool> {
        if !self.is_unlocked() {
            return Err(Error::Locked);
        }
        let saved = owner.starts_with(LOCAL_SHELL_OWNER_PREFIX)
            || matches!(self.get(owner), Some(Item::Host(_) | Item::TelnetHost(_)));
        if !saved || !keeps(command) {
            return Ok(false);
        }
        let mut commands = self.command_history(owner)?;
        if let Some(at) = commands.iter().position(|c| c == command) {
            commands.remove(at).zeroize();
        }
        commands.insert(0, command.to_owned());
        if commands.len() > COMMAND_HISTORY_MAX {
            for mut old in commands.drain(COMMAND_HISTORY_MAX..) {
                old.zeroize();
            }
        }
        self.store_command_history(owner, &commands)?;
        Ok(true)
    }

    /// Removes a command from a host's or local shell's history. Returns whether the history had
    /// it.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn command_history_remove(&mut self, owner: &str, command: &str) -> Result<bool> {
        let mut commands = self.command_history(owner)?;
        let Some(at) = commands.iter().position(|c| c == command) else {
            return Ok(false);
        };
        commands.remove(at).zeroize();
        if commands.is_empty() {
            self.store.delete_command_history(owner)?;
        } else {
            self.store_command_history(owner, &commands)?;
        }
        Ok(true)
    }

    /// Deletes the command history of every host and local shell on this device.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn command_history_clear(&mut self) -> Result<()> {
        if !self.is_unlocked() {
            return Err(Error::Locked);
        }
        self.store.clear_command_history()
    }

    fn store_command_history(&self, owner: &str, commands: &[String]) -> Result<()> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;
        let plain = Zeroizing::new(serde_json::to_vec(commands)?);
        self.store.put_command_history(&CommandHistoryRow {
            owner: owner.to_owned(),
            commands: seal(&unlocked.vault_key, aad(owner).as_bytes(), &plain)?.to_json(),
            updated_at: self.clock.now_ms(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Host, TelnetHost};
    use crate::store::ItemRow;
    use crate::sync::SyncConfig;
    use crate::sync::backend::{RemoteItem, VaultMeta};
    use crate::sync::engine::remote_row;

    const SHELL: &str = "local:pwsh";

    fn vault() -> Vault {
        crate::vault_ai::tests::vault().0
    }

    fn host(vault: &mut Vault) -> String {
        vault.put(None, Item::Host(Host::default())).unwrap()
    }

    fn telnet_host(vault: &mut Vault) -> String {
        vault
            .put(None, Item::TelnetHost(TelnetHost::default()))
            .unwrap()
    }

    fn history(vault: &Vault, owner: &str) -> Vec<String> {
        vault.command_history(owner).unwrap().to_vec()
    }

    fn add(vault: &mut Vault, owner: &str, commands: &[&str]) {
        for command in commands {
            assert!(
                vault.command_history_add(owner, command).unwrap(),
                "{command}"
            );
        }
    }

    #[test]
    fn histories_are_sealed_newest_first_and_kept_on_the_device() {
        let mut vault = vault();
        let h = host(&mut vault);
        let pending = vault.pending_count();
        add(&mut vault, &h, &["ls -la", "cd /srv", "SECRET=1 make"]);
        assert_eq!(history(&vault, &h), ["SECRET=1 make", "cd /srv", "ls -la"]);

        // Nothing readable in the database, and nothing to sync.
        let row = vault.store.command_history(&h).unwrap().unwrap();
        assert!(!row.commands.contains("SECRET"));
        assert_eq!(vault.pending_count(), pending);

        // A command run again moves to the top instead of showing twice.
        add(&mut vault, &h, &["ls -la"]);
        assert_eq!(history(&vault, &h), ["ls -la", "SECRET=1 make", "cd /srv"]);

        // Each owner has its own: a Telnet host and a local shell.
        let t = telnet_host(&mut vault);
        add(&mut vault, &t, &["show run"]);
        add(&mut vault, SHELL, &["git status"]);
        assert_eq!(history(&vault, &t), ["show run"]);
        assert_eq!(history(&vault, SHELL), ["git status"]);
        assert!(history(&vault, "local:bash").is_empty());

        // A history opens only in its own row.
        vault
            .store
            .put_command_history(&CommandHistoryRow {
                owner: t.clone(),
                ..row
            })
            .unwrap();
        assert!(history(&vault, &t).is_empty());
    }

    #[test]
    fn what_a_history_leaves_out() {
        let mut vault = vault();
        let h = host(&mut vault);
        let long = "é".repeat(COMMAND_MAX_CHARS + 1);
        for command in ["", "   ", "\t", " export TOKEN=abc", long.as_str()] {
            assert!(!vault.command_history_add(&h, command).unwrap());
        }
        assert!(vault.store.command_history(&h).unwrap().is_none());
        // The limit counts characters, not bytes, and a space inside is kept.
        let longest = "é".repeat(COMMAND_MAX_CHARS);
        add(&mut vault, &h, &[longest.as_str(), "echo  hi "]);
        assert_eq!(history(&vault, &h), ["echo  hi ", longest.as_str()]);

        // Only saved hosts and local shells keep a history: not an unknown id, another kind of
        // item, or a host deleted while its tab stays open.
        let gone = host(&mut vault);
        vault.delete_host(&gone).unwrap();
        for owner in ["nope", crate::model::SETTINGS_ID, gone.as_str()] {
            assert!(!vault.command_history_add(owner, "ls").unwrap(), "{owner}");
            assert!(vault.store.command_history(owner).unwrap().is_none());
        }
    }

    #[test]
    fn a_history_keeps_the_newest_commands() {
        let mut vault = vault();
        for i in 0..=COMMAND_HISTORY_MAX {
            vault
                .command_history_add(SHELL, &format!("echo {i}"))
                .unwrap();
        }
        let commands = history(&vault, SHELL);
        assert_eq!(commands.len(), COMMAND_HISTORY_MAX);
        assert_eq!(commands[0], format!("echo {COMMAND_HISTORY_MAX}"));
        assert_eq!(commands[COMMAND_HISTORY_MAX - 1], "echo 1");

        // One already kept moves up and drops nothing.
        add(&mut vault, SHELL, &["echo 500"]);
        let commands = history(&vault, SHELL);
        assert_eq!(commands.len(), COMMAND_HISTORY_MAX);
        assert_eq!(
            commands[..2],
            ["echo 500".to_owned(), format!("echo {COMMAND_HISTORY_MAX}")]
        );
        assert_eq!(commands[COMMAND_HISTORY_MAX - 1], "echo 1");
    }

    #[test]
    fn commands_are_removed_and_histories_cleared() {
        let mut vault = vault();
        let h = host(&mut vault);
        add(&mut vault, &h, &["ls", "pwd"]);
        add(&mut vault, SHELL, &["dir"]);

        assert!(vault.command_history_remove(&h, "ls").unwrap());
        assert!(!vault.command_history_remove(&h, "ls").unwrap());
        assert!(!vault.command_history_remove("local:bash", "ls").unwrap());
        assert_eq!(history(&vault, &h), ["pwd"]);
        // The last command takes its row with it.
        assert!(vault.command_history_remove(&h, "pwd").unwrap());
        assert!(vault.store.command_history(&h).unwrap().is_none());

        add(&mut vault, &h, &["ls"]);
        vault.command_history_clear().unwrap();
        assert!(vault.store.command_history_rows().unwrap().is_empty());
        assert!(history(&vault, SHELL).is_empty());
    }

    #[test]
    fn histories_need_the_vault_unlocked() {
        let mut vault = vault();
        add(&mut vault, SHELL, &["ls"]);
        vault.lock();
        assert!(matches!(vault.command_history(SHELL), Err(Error::Locked)));
        assert!(matches!(
            vault.command_history_add(SHELL, "pwd"),
            Err(Error::Locked)
        ));
        assert!(matches!(
            vault.command_history_remove(SHELL, "ls"),
            Err(Error::Locked)
        ));
        assert!(matches!(vault.command_history_clear(), Err(Error::Locked)));
        assert_eq!(vault.store.command_history_rows().unwrap().len(), 1);
    }

    #[test]
    fn a_join_seals_the_histories_under_the_new_key() {
        let mut vault = vault();
        let h = host(&mut vault);
        add(&mut vault, &h, &["uptime"]);
        add(&mut vault, SHELL, &["git pull", "cargo test"]);
        // A history no key of this vault opens.
        let damaged = CommandHistoryRow {
            owner: "local:bash".into(),
            commands: seal(
                &crypto::random_key().unwrap(),
                aad("local:bash").as_bytes(),
                b"[]",
            )
            .unwrap()
            .to_json(),
            updated_at: 1,
        };
        vault.store.put_command_history(&damaged).unwrap();

        // Another vault, as flow C finds it in the cloud.
        let cloud = crate::vault_ai::tests::vault().0;
        let meta = cloud.local_meta().unwrap();
        let remote = VaultMeta {
            schema_version: meta.schema_version,
            kdf_salt: meta.kdf_salt,
            kdf_params: meta.kdf_params,
            protected_vault_key: meta.protected_vault_key,
            recovery_vault_key: meta.recovery_vault_key,
            seq: 0,
        };
        let old = vault.vault_key_copy().unwrap();
        let config = SyncConfig::Worker {
            url: "https://sync.example.workers.dev".into(),
            deployment: None,
        };
        assert!(
            vault
                .join_remote_vault(&remote, cloud.vault_key_copy().unwrap(), &config, None)
                .unwrap()
        );

        assert_eq!(history(&vault, &h), ["uptime"]);
        assert_eq!(history(&vault, SHELL), ["cargo test", "git pull"]);
        assert!(vault.store.command_history("local:bash").unwrap().is_none());
        // The old key no longer opens them.
        let row = vault.store.command_history(SHELL).unwrap().unwrap();
        assert!(open(&old, &row).is_none());
    }

    #[test]
    fn a_deleted_host_takes_its_history_with_it() {
        let mut vault = vault();
        let ssh = host(&mut vault);
        let telnet = telnet_host(&mut vault);
        let plain = host(&mut vault);
        let pulled = host(&mut vault);
        for owner in [&ssh, &telnet, &plain, &pulled] {
            add(&mut vault, owner, &["ls"]);
        }
        add(&mut vault, SHELL, &["ls"]);

        // Deleted on this device.
        vault.delete_host(&ssh).unwrap();
        vault.delete_telnet_host(&telnet).unwrap();
        vault.delete(&plain).unwrap();
        // Deleted on another device: a sync pull writes its tombstone.
        let row = vault.store.item_row(&pulled).unwrap().unwrap();
        let tombstone = RemoteItem {
            id: pulled.clone(),
            envelope: None,
            revision: row.revision + 1,
            seq: 1,
            deleted: true,
            updated_at: row.updated_at + 1,
        };
        vault
            .store
            .transaction(|tx| tx.upsert_item_row(&remote_row(&tombstone)))
            .unwrap();

        let owners: Vec<String> = vault
            .store
            .command_history_rows()
            .unwrap()
            .into_iter()
            .map(|row| row.owner)
            .collect();
        assert_eq!(owners, [SHELL]);

        // A row dropped outright, as a join drops one its key cannot open, goes too.
        let other = host(&mut vault);
        add(&mut vault, &other, &["ls"]);
        vault.store.delete_item_row(&other).unwrap();
        assert!(vault.store.command_history(&other).unwrap().is_none());
        // An edit is no deletion.
        let kept = host(&mut vault);
        add(&mut vault, &kept, &["ls"]);
        vault
            .store
            .upsert_item_row(&ItemRow {
                dirty: false,
                ..vault.store.item_row(&kept).unwrap().unwrap()
            })
            .unwrap();
        assert_eq!(history(&vault, &kept), ["ls"]);
    }
}
