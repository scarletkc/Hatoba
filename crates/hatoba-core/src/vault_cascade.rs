//! Deleting a host, group or key together with the items that refer to it.
//!
//! Each deletion repairs the references to the deleted item and tombstones what belongs to it,
//! all in one store transaction: a failed write leaves the database and the item map as they
//! were, and the item map changes only after the commit.
//!
//! - a host: hosts that jumped through it connect directly, and its port forwards go with it;
//! - a group: its hosts and child groups move to the top level;
//! - a key: hosts that signed in with it ask on every connection ([`HostAuth::Ask`]).

use zeroize::Zeroize;

use crate::error::{Error, Result};
use crate::model::{HostAuth, Item};
use crate::store::{ItemRow, StoreOps};
use crate::vault::{Vault, seal_item};

impl Vault {
    /// Deletes a host, clears it as the jump host of other hosts and deletes its port forwards.
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::ItemNotFound`] when `id` is not a host; storage errors, after
    /// which nothing has changed.
    pub fn delete_host(&mut self, id: &str) -> Result<()> {
        self.live(id, Item::as_host)?;
        let puts = self
            .hosts()
            .into_iter()
            .filter(|(other_id, other)| other_id != id && other.jump_host_id.as_deref() == Some(id))
            .map(|(other_id, mut other)| {
                other.jump_host_id = None;
                (other_id, Item::Host(other))
            })
            .collect();
        let mut deletes: Vec<String> = self
            .forwards()
            .into_iter()
            .filter(|(_, forward)| forward.host_id == id)
            .map(|(forward_id, _)| forward_id)
            .collect();
        deletes.push(id.to_owned());
        self.commit_cascade(puts, &deletes)
    }

    /// Deletes a group and moves its hosts and child groups to the top level.
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::ItemNotFound`] when `id` is not a group; storage errors, after
    /// which nothing has changed.
    pub fn delete_group(&mut self, id: &str) -> Result<()> {
        self.live(id, Item::as_group)?;
        let hosts = self
            .hosts()
            .into_iter()
            .filter(|(_, host)| host.group_id.as_deref() == Some(id))
            .map(|(host_id, mut host)| {
                host.group_id = None;
                (host_id, Item::Host(host))
            });
        let children = self
            .groups()
            .into_iter()
            .filter(|(child_id, child)| child_id != id && child.parent_id.as_deref() == Some(id))
            .map(|(child_id, mut child)| {
                child.parent_id = None;
                (child_id, Item::Group(child))
            });
        let puts = hosts.chain(children).collect();
        self.commit_cascade(puts, &[id.to_owned()])
    }

    /// Deletes a key. Hosts that signed in with it ask on every connection instead.
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::ItemNotFound`] when `id` is not a key; storage errors, after
    /// which nothing has changed.
    pub fn delete_key(&mut self, id: &str) -> Result<()> {
        self.live(id, Item::as_key)?;
        let puts = self
            .hosts()
            .into_iter()
            .filter(|(_, host)| matches!(&host.auth, HostAuth::Key { key_id } if key_id == id))
            .map(|(host_id, mut host)| {
                host.auth = HostAuth::Ask;
                (host_id, Item::Host(host))
            })
            .collect();
        self.commit_cascade(puts, &[id.to_owned()])
    }

    /// Checks that `id` is a live item of the kind `pick` accepts.
    fn live<T>(&self, id: &str, pick: impl Fn(&Item) -> Option<&T>) -> Result<()> {
        if !self.is_unlocked() {
            return Err(Error::Locked);
        }
        self.get(id)
            .and_then(pick)
            .map(|_| ())
            .ok_or_else(|| Error::ItemNotFound(id.to_owned()))
    }

    /// Writes `puts` as [`Vault::put`] does and tombstones `deletes` as [`Vault::delete`] does,
    /// in one transaction, then applies both to the item map.
    fn commit_cascade(&mut self, mut puts: Vec<(String, Item)>, deletes: &[String]) -> Result<()> {
        let now = self.clock.now_ms();
        let Self {
            store, unlocked, ..
        } = self;
        let unlocked = unlocked.as_mut().ok_or(Error::Locked)?;
        let vault_key = &unlocked.vault_key;
        store.transaction(|tx| {
            for (id, item) in &mut puts {
                let previous = tx.item_row(id)?;
                let updated_at = previous
                    .as_ref()
                    .map_or(now, |p| now.max(p.updated_at.saturating_add(1)));
                item.set_updated_at(updated_at);
                let envelope = seal_item(vault_key, id, item)?;
                tx.upsert_item_row(&ItemRow {
                    id: id.clone(),
                    envelope: Some(envelope.to_json()),
                    revision: previous.map_or(0, |p| p.revision),
                    deleted: false,
                    dirty: true,
                    updated_at,
                })?;
            }
            for id in deletes {
                let row = tx
                    .item_row(id)?
                    .filter(|r| !r.deleted)
                    .ok_or_else(|| Error::ItemNotFound(id.clone()))?;
                tx.upsert_item_row(&ItemRow {
                    id: id.clone(),
                    envelope: None,
                    revision: row.revision,
                    deleted: true,
                    dirty: true,
                    updated_at: now.max(row.updated_at.saturating_add(1)),
                })?;
            }
            Ok(())
        })?;
        for (id, item) in puts {
            if let Some(mut old) = unlocked.items.insert(id, item) {
                old.zeroize();
            }
        }
        for id in deletes {
            if let Some(mut item) = unlocked.items.remove(id) {
                item.zeroize();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::sync::Arc;

    use zeroize::Zeroizing;

    use super::*;
    use crate::crypto::KdfParams;
    use crate::model::{Group, Host, PortForward, SshKey};
    use crate::vault::ManualClock;

    const PW: &str = "correct horse battery staple";

    fn vault() -> (Vault, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new(1_000_000));
        let mut vault = Vault::open_in_memory().unwrap().with_clock(clock.clone());
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
        (vault, clock)
    }

    fn host(name: &str, edit: impl FnOnce(&mut Host)) -> Item {
        let mut host = Host {
            name: name.into(),
            address: format!("{name}.example.org"),
            ..Host::default()
        };
        edit(&mut host);
        Item::Host(host)
    }

    fn group(name: &str, parent_id: Option<&str>) -> Item {
        Item::Group(Group {
            name: name.into(),
            parent_id: parent_id.map(str::to_owned),
            ..Group::default()
        })
    }

    fn forward(host_id: &str, port: u16) -> Item {
        Item::Forward(PortForward {
            host_id: host_id.into(),
            bind_address: "127.0.0.1".into(),
            bind_port: port,
            dest_host: Some("localhost".into()),
            dest_port: Some(port),
            ..PortForward::default()
        })
    }

    fn key(name: &str) -> Item {
        Item::Key(SshKey {
            name: name.into(),
            private_key: Zeroizing::new("PRIVATE-KEY-BODY".into()),
            ..SshKey::default()
        })
    }

    fn rows(vault: &Vault) -> BTreeMap<String, ItemRow> {
        vault
            .store
            .item_rows()
            .unwrap()
            .into_iter()
            .map(|row| (row.id.clone(), row))
            .collect()
    }

    fn items(vault: &Vault) -> BTreeMap<String, Item> {
        vault
            .items()
            .map(|(id, item)| (id.to_owned(), item.clone()))
            .collect()
    }

    /// Marks every row pushed at revision 7, as after a sync, so a write shows in `dirty`.
    fn settle(vault: &Vault) {
        vault
            .store
            .conn()
            .execute_batch("UPDATE items SET dirty = 0, revision = 7")
            .unwrap();
    }

    /// The ids whose rows differ between `before` and `after`.
    fn changed(
        before: &BTreeMap<String, ItemRow>,
        after: &BTreeMap<String, ItemRow>,
    ) -> BTreeSet<String> {
        after
            .iter()
            .filter(|(id, row)| before.get(*id) != Some(*row))
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Makes every write of the row `id` fail, as a full disk would.
    fn fail_writes_of(vault: &Vault, id: &str) {
        vault
            .store
            .conn()
            .execute_batch(&format!(
                "CREATE TEMP TRIGGER fail_insert BEFORE INSERT ON items WHEN NEW.id = '{id}' \
                   BEGIN SELECT RAISE(ABORT, 'injected'); END; \
                 CREATE TEMP TRIGGER fail_update BEFORE UPDATE ON items WHEN NEW.id = '{id}' \
                   BEGIN SELECT RAISE(ABORT, 'injected'); END;"
            ))
            .unwrap();
    }

    /// Makes the commit fail: every write leaves a deferred foreign key broken.
    fn fail_commit(vault: &Vault) {
        vault
            .store
            .conn()
            .execute_batch(
                "CREATE TEMP TABLE parent (id INTEGER PRIMARY KEY); \
                 CREATE TEMP TABLE child \
                   (parent_id INTEGER REFERENCES parent(id) DEFERRABLE INITIALLY DEFERRED); \
                 CREATE TEMP TRIGGER fail_insert AFTER INSERT ON items \
                   BEGIN INSERT INTO child VALUES (1); END; \
                 CREATE TEMP TRIGGER fail_update AFTER UPDATE ON items \
                   BEGIN INSERT INTO child VALUES (1); END;",
            )
            .unwrap();
    }

    fn heal(vault: &Vault) {
        vault
            .store
            .conn()
            .execute_batch(
                "DROP TRIGGER temp.fail_insert; DROP TRIGGER temp.fail_update; \
                 DROP TABLE IF EXISTS temp.child; DROP TABLE IF EXISTS temp.parent;",
            )
            .unwrap();
    }

    /// Runs `op` once with each of `written` failing and once with the commit failing: each time
    /// it fails and neither the rows nor the item map change.
    fn assert_atomic(vault: &mut Vault, written: &[&str], op: impl Fn(&mut Vault) -> Result<()>) {
        let (rows_before, items_before) = (rows(vault), items(vault));
        for id in written {
            fail_writes_of(vault, id);
            assert!(
                op(vault).is_err(),
                "a failed write of {id} fails the deletion"
            );
            heal(vault);
            assert_eq!(
                rows(vault),
                rows_before,
                "no row changed after a failed write of {id}"
            );
            assert_eq!(
                items(vault),
                items_before,
                "no item changed after a failed write of {id}"
            );
        }
        fail_commit(vault);
        assert!(op(vault).is_err(), "a failed commit fails the deletion");
        heal(vault);
        assert_eq!(
            rows(vault),
            rows_before,
            "no row changed after a failed commit"
        );
        assert_eq!(
            items(vault),
            items_before,
            "no item changed after a failed commit"
        );
    }

    #[test]
    fn deleting_a_host_clears_jumps_to_it_and_deletes_its_forwards() {
        let (mut v, clock) = vault();
        let gone = v.put(None, host("bastion", |_| {})).unwrap();
        let through = v
            .put(None, host("inner", |h| h.jump_host_id = Some(gone.clone())))
            .unwrap();
        let other = v.put(None, host("other", |_| {})).unwrap();
        let its_forward = v.put(None, forward(&gone, 8080)).unwrap();
        let other_forward = v.put(None, forward(&other, 9090)).unwrap();
        settle(&v);
        clock.advance(1_000);

        assert_atomic(&mut v, &[&through, &its_forward, &gone], |v| {
            v.delete_host(&gone)
        });
        let before = rows(&v);
        v.delete_host(&gone).unwrap();
        let after = rows(&v);

        assert_eq!(
            changed(&before, &after),
            BTreeSet::from([gone.clone(), through.clone(), its_forward.clone()])
        );
        for id in [&gone, &its_forward] {
            let row = &after[id];
            assert!(row.deleted && row.envelope.is_none() && row.dirty && row.revision == 7);
            assert!(row.updated_at > before[id].updated_at);
            assert!(v.get(id).is_none());
        }
        let row = &after[&through];
        assert!(!row.deleted && row.dirty && row.revision == 7);
        assert_eq!(row.updated_at, 1_001_000);
        let inner = v.get(&through).and_then(Item::as_host).unwrap();
        assert_eq!(inner.jump_host_id, None);
        assert_eq!(inner.updated_at, 1_001_000);
        assert!(v.get(&other).is_some() && v.get(&other_forward).is_some());

        // The deletion is in the database, not only in the item map.
        let items_after = items(&v);
        v.lock();
        v.unlock(PW).unwrap();
        assert_eq!(items(&v), items_after);
    }

    #[test]
    fn deleting_a_group_moves_its_hosts_and_children_to_the_top() {
        let (mut v, clock) = vault();
        let gone = v.put(None, group("prod", None)).unwrap();
        let child = v.put(None, group("prod-eu", Some(&gone))).unwrap();
        let sibling = v.put(None, group("staging", None)).unwrap();
        let member = v
            .put(None, host("api", |h| h.group_id = Some(gone.clone())))
            .unwrap();
        let in_child = v
            .put(None, host("eu-api", |h| h.group_id = Some(child.clone())))
            .unwrap();
        settle(&v);
        clock.advance(1_000);

        assert_atomic(&mut v, &[&member, &child, &gone], |v| v.delete_group(&gone));
        let before = rows(&v);
        v.delete_group(&gone).unwrap();
        let after = rows(&v);

        assert_eq!(
            changed(&before, &after),
            BTreeSet::from([gone.clone(), child.clone(), member.clone()])
        );
        assert!(after[&gone].deleted && v.get(&gone).is_none());
        assert!(after[&child].dirty && after[&member].dirty);
        let Some(Item::Group(child_group)) = v.get(&child) else {
            panic!("the child group stays")
        };
        assert_eq!(child_group.parent_id, None);
        let member_host = v.get(&member).and_then(Item::as_host).unwrap();
        assert_eq!(member_host.group_id, None);
        let eu_host = v.get(&in_child).and_then(Item::as_host).unwrap();
        assert_eq!(eu_host.group_id.as_deref(), Some(child.as_str()));
        assert!(v.get(&sibling).is_some());

        let items_after = items(&v);
        v.lock();
        v.unlock(PW).unwrap();
        assert_eq!(items(&v), items_after);
    }

    #[test]
    fn deleting_a_key_makes_its_hosts_ask() {
        let (mut v, clock) = vault();
        let gone = v.put(None, key("deploy")).unwrap();
        let kept = v.put(None, key("backup")).unwrap();
        let signed_in = v
            .put(
                None,
                host("web", |h| {
                    h.auth = HostAuth::Key {
                        key_id: gone.clone(),
                    };
                }),
            )
            .unwrap();
        let other_key = v
            .put(
                None,
                host("db", |h| {
                    h.auth = HostAuth::Key {
                        key_id: kept.clone(),
                    };
                }),
            )
            .unwrap();
        let password = v
            .put(None, host("cache", |h| h.auth = HostAuth::password("pw")))
            .unwrap();
        settle(&v);
        clock.advance(1_000);

        assert_atomic(&mut v, &[&signed_in, &gone], |v| v.delete_key(&gone));
        let before = rows(&v);
        v.delete_key(&gone).unwrap();
        let after = rows(&v);

        assert_eq!(
            changed(&before, &after),
            BTreeSet::from([gone.clone(), signed_in.clone()])
        );
        assert!(after[&gone].deleted && v.get(&gone).is_none());
        let auth = |id: &str| v.get(id).and_then(Item::as_host).unwrap().auth.clone();
        assert_eq!(auth(&signed_in), HostAuth::Ask);
        assert_eq!(auth(&other_key), HostAuth::Key { key_id: kept });
        assert_eq!(auth(&password), HostAuth::password("pw"));

        let items_after = items(&v);
        v.lock();
        v.unlock(PW).unwrap();
        assert_eq!(items(&v), items_after);
    }

    #[test]
    fn a_missing_or_other_item_or_a_locked_vault_is_refused() {
        let (mut v, _clock) = vault();
        let host_id = v.put(None, host("web", |_| {})).unwrap();
        let group_id = v.put(None, group("prod", None)).unwrap();
        let before = rows(&v);

        assert!(matches!(
            v.delete_host(&group_id),
            Err(Error::ItemNotFound(_))
        ));
        assert!(matches!(
            v.delete_group(&host_id),
            Err(Error::ItemNotFound(_))
        ));
        assert!(matches!(
            v.delete_key(&host_id),
            Err(Error::ItemNotFound(_))
        ));
        assert!(matches!(
            v.delete_host("missing"),
            Err(Error::ItemNotFound(_))
        ));
        v.delete_host(&host_id).unwrap();
        assert!(matches!(
            v.delete_host(&host_id),
            Err(Error::ItemNotFound(_))
        ));
        assert_eq!(changed(&before, &rows(&v)), BTreeSet::from([host_id]));

        v.lock();
        assert!(matches!(v.delete_group(&group_id), Err(Error::Locked)));
    }
}
