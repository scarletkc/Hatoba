//! End-to-end sync scenarios: several `Vault`s ("devices") sharing one in-memory server that has
//! the Worker's semantics (see `fake.rs`).

use std::collections::BTreeMap;
use std::sync::{Arc, MutexGuard};

use zeroize::Zeroizing;

use super::fake::{FakeBackend, FakeServer, SETUP_TOKEN};
use super::*;
use crate::crypto::{KdfParams, random_key, seal};
use crate::error::Error;
use crate::model::{Host, HostAuth, Item, RightClick, SETTINGS_ID, SshKey, new_id};
use crate::platform::DeviceInfo;
use crate::recovery::RecoveryCode;
use crate::store::StoreOps;
use crate::sync::engine::{SyncOptions, sync_round};
use crate::vault::{ManualClock, Vault};

const PW: &str = "sync-test-password";
const T0: i64 = 1_700_000_000_000;

struct Dev {
    vault: SharedVault,
    backend: FakeBackend,
    clock: Arc<ManualClock>,
    name: &'static str,
}

impl Dev {
    fn v(&self) -> MutexGuard<'_, Vault> {
        self.vault.lock().unwrap()
    }

    fn info(&self) -> DeviceInfo {
        DeviceInfo {
            name: self.name.to_owned(),
            platform: "test-os".to_owned(),
        }
    }

    async fn sync(&self) -> SyncReport {
        sync_round(&self.vault, &self.backend, &SyncOptions::default())
            .await
            .unwrap()
    }

    async fn try_sync(&self) -> Result<SyncReport> {
        sync_round(&self.vault, &self.backend, &SyncOptions::default()).await
    }

    fn put(&self, item: Item) -> String {
        self.v().put(None, item).unwrap()
    }

    fn edit(&self, id: &str, f: impl FnOnce(&mut Item)) {
        let mut v = self.v();
        let mut item = v.get(id).expect("item exists").clone();
        f(&mut item);
        v.put(Some(id), item).unwrap();
    }

    fn rename(&self, id: &str, name: &str) {
        self.edit(id, |item| match item {
            Item::Host(h) => h.name = name.into(),
            Item::Key(k) => k.name = name.into(),
            other => panic!("cannot rename {}", other.kind_str()),
        });
    }

    fn name_of(&self, id: &str) -> Option<String> {
        self.v().get(id).map(Item::display_name)
    }

    /// Sorted display names of everything except the settings item.
    fn names(&self) -> Vec<String> {
        let v = self.v();
        let mut names: Vec<String> = v
            .items()
            .filter(|(id, _)| *id != SETTINGS_ID)
            .map(|(_, i)| i.display_name())
            .collect();
        names.sort();
        names
    }

    fn snapshot(&self) -> BTreeMap<String, Item> {
        self.v()
            .items()
            .map(|(id, item)| (id.to_owned(), item.clone()))
            .collect()
    }

    fn pending(&self) -> u64 {
        self.v().pending_count()
    }

    fn find(&self, name: &str) -> Option<String> {
        self.v()
            .items()
            .find(|(_, i)| i.display_name() == name)
            .map(|(id, _)| id.to_owned())
    }
}

fn host(name: &str) -> Item {
    Item::Host(Host {
        name: name.into(),
        address: format!("{name}.example.org"),
        ..Host::default()
    })
}

fn key(name: &str, body: &str) -> Item {
    Item::Key(SshKey {
        name: name.into(),
        private_key: Zeroizing::new(body.into()),
        ..SshKey::default()
    })
}

fn device(
    server: &Arc<FakeServer>,
    clock: &Arc<ManualClock>,
    name: &'static str,
    created: bool,
) -> Dev {
    let mut vault = Vault::open_in_memory().unwrap().with_clock(clock.clone());
    if created {
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
    }
    Dev {
        vault: share(vault),
        backend: FakeBackend::new(server),
        clock: clock.clone(),
        name,
    }
}

/// A fresh server, shared clock, device A with a vault, and an empty device B.
fn world() -> (Arc<FakeServer>, Arc<ManualClock>, Dev, Dev) {
    let server = FakeServer::new();
    let clock = Arc::new(ManualClock::new(T0));
    let a = device(&server, &clock, "device-a", true);
    let b = device(&server, &clock, "device-b", false);
    (server, clock, a, b)
}

async fn enable(dev: &Dev) -> Session {
    enable_sync(&dev.vault, &dev.backend, PW, Some(SETUP_TOKEN), dev.info())
        .await
        .unwrap()
}

fn config() -> SyncConfig {
    SyncConfig::Worker {
        url: "https://sync.example.workers.dev".into(),
        deployment: None,
    }
}

/// Flow B followed by the first sync round, as the app runs them.
async fn restore(dev: &Dev) -> Session {
    let session = restore_from_cloud(&dev.vault, &dev.backend, PW, dev.info(), &config())
        .await
        .unwrap();
    dev.sync().await;
    session
}

/// A and B both synced and holding the same data.
async fn two_devices() -> (Arc<FakeServer>, Arc<ManualClock>, Dev, Dev) {
    let (server, clock, a, b) = world();
    a.put(host("seed-host"));
    enable(&a).await;
    restore(&b).await;
    assert_eq!(a.snapshot(), b.snapshot());
    (server, clock, a, b)
}

/// Syncs A, B, A so both have seen each other's changes, and asserts they agree.
async fn converge(a: &Dev, b: &Dev) {
    a.sync().await;
    b.sync().await;
    a.sync().await;
    b.sync().await;
    assert_eq!(a.snapshot(), b.snapshot(), "devices must converge");
    assert_eq!(a.pending() + b.pending(), 0);
}

// ---- flows A and B ------------------------------------------------------------------------

#[tokio::test]
async fn enable_on_a_then_restore_on_b() {
    let (server, _clock, a, b) = world();
    let host_id = a.put(Item::Host(Host {
        name: "prod-api".into(),
        auth: HostAuth::password("s3cret"),
        ..Host::default()
    }));
    let key_id = a.put(key("deploy", "PRIVATE-KEY-BODY"));
    assert_eq!(a.pending(), 3, "settings + host + key");

    let session = enable(&a).await;
    assert!(!session.token.is_empty());
    assert_eq!(a.pending(), 0);
    assert_eq!(server.item_count(), 3);
    assert_eq!(
        a.v().sync_cursor(),
        0,
        "own pushes never advance the pull cursor"
    );

    // Everything on the server is ciphertext.
    let dump = server.dump();
    for secret in [
        "prod-api",
        "s3cret",
        "PRIVATE-KEY-BODY",
        "deploy",
        "\"type\"",
        PW,
    ] {
        assert!(!dump.contains(secret), "server state leaked {secret:?}");
    }

    let b_session = restore(&b).await;
    assert!(!b_session.token.is_empty());
    assert!(b.v().is_unlocked());
    assert_ne!(a.v().device_id(), b.v().device_id());
    assert_eq!(b.name_of(&host_id).as_deref(), Some("prod-api"));
    let Item::Host(h) = b.v().get(&host_id).unwrap().clone() else {
        panic!()
    };
    assert_eq!(h.auth, HostAuth::password("s3cret"));
    assert!(b.v().get(&key_id).is_some());
    assert_eq!(b.pending(), 0);
    assert_eq!(a.snapshot(), b.snapshot());
    assert_eq!(b.v().sync_cursor(), server.current_seq());

    // B really holds the same vault: it unlocks with the same password after a restart.
    b.v().lock();
    b.v().unlock(PW).unwrap();
    assert_eq!(a.snapshot(), b.snapshot());
}

#[tokio::test]
async fn enable_sync_requires_unlocked_vault_and_an_empty_remote() {
    let (server, clock, a, _b) = world();
    a.v().lock();
    let err = enable_sync(&a.vault, &a.backend, PW, Some(SETUP_TOKEN), a.info())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Locked));
    assert_eq!(
        server.session_count(),
        0,
        "nothing may reach the server while locked"
    );
    a.v().unlock(PW).unwrap();

    // Wrong password: caught locally, before any setup request.
    let err = enable_sync(&a.vault, &a.backend, "nope", Some(SETUP_TOKEN), a.info())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::WrongPassword));
    assert!(!a.backend.health().await.unwrap().initialized);

    // Wrong setup token.
    let err = enable_sync(&a.vault, &a.backend, PW, Some("wrong-token"), a.info())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidSetupToken));
    let err = enable_sync(&a.vault, &a.backend, PW, None, a.info())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidSetupToken));

    enable(&a).await;

    // A second vault cannot enable sync against an initialised remote (flow C is post-MVP).
    let other = device(&server, &clock, "other", true);
    let err = enable_sync(
        &other.vault,
        &other.backend,
        PW,
        Some(SETUP_TOKEN),
        other.info(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::RemoteInitialized));
}

/// Enables sync on `a` after `interrupt` broke the first attempt, and checks the result is as
/// good as an uninterrupted setup: everything uploaded and readable from a second device.
async fn enable_after_interruption(interrupt: impl FnOnce(&Arc<FakeServer>, &Dev)) {
    let (server, _clock, a, b) = world();
    a.put(host("web-1"));
    a.put(key("deploy", "PRIVATE-KEY-BODY"));
    interrupt(&server, &a);
    let err = enable_sync(&a.vault, &a.backend, PW, Some(SETUP_TOKEN), a.info())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Offline), "got {err:?}");
    server.set_offline(false);
    assert!(a.backend.health().await.unwrap().initialized);

    enable(&a).await;
    assert_eq!(a.pending(), 0);
    assert_eq!(server.item_count(), 3, "settings + host + key");
    restore(&b).await;
    assert_eq!(a.snapshot(), b.snapshot());
}

#[tokio::test]
async fn enable_sync_resumes_after_a_lost_setup_response() {
    enable_after_interruption(|server, _| server.fail_next("setup")).await;
}

#[tokio::test]
async fn enable_sync_resumes_after_a_failed_login() {
    enable_after_interruption(|server, _| server.fail_next("login")).await;
}

#[tokio::test]
async fn enable_sync_resumes_after_a_failed_first_pull() {
    enable_after_interruption(|server, _| server.fail_next("pull")).await;
}

#[tokio::test]
async fn enable_sync_resumes_after_a_failed_push() {
    enable_after_interruption(|server, a| {
        let offline = Arc::clone(server);
        a.backend.on_next_pull(move || offline.set_offline(true));
    })
    .await;
}

#[tokio::test]
async fn enable_sync_resumes_after_a_lost_push_response() {
    let (server, _clock, a, b) = world();
    a.put(host("web-1"));
    server.lose_next_push_response();
    let err = enable_sync(&a.vault, &a.backend, PW, Some(SETUP_TOKEN), a.info())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Offline), "got {err:?}");
    assert_eq!(server.item_count(), 2, "the server applied the push");

    enable(&a).await;
    assert_eq!(a.pending(), 0);
    assert_eq!(server.item_count(), 2);
    assert!(
        a.v().conflicts(false).unwrap().is_empty(),
        "identical content converges silently"
    );
    restore(&b).await;
    assert_eq!(a.snapshot(), b.snapshot());
}

#[tokio::test]
async fn enable_sync_still_rejects_another_vault_with_the_same_password() {
    let (server, clock, a, _b) = world();
    enable(&a).await;
    let other = device(&server, &clock, "other", true);
    other.put(host("other-host"));
    let err = enable_sync(
        &other.vault,
        &other.backend,
        PW,
        Some(SETUP_TOKEN),
        other.info(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::RemoteInitialized));
    assert_eq!(server.session_count(), 1, "the other vault never logged in");
    assert_eq!(server.item_count(), 1, "only A's settings item");
}

#[tokio::test]
async fn restore_errors() {
    let (server, clock, a, b) = world();
    // Nothing deployed yet.
    let err = restore_from_cloud(&b.vault, &b.backend, PW, b.info(), &config())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::RemoteNotInitialized));
    enable(&a).await;

    let err = restore_from_cloud(&b.vault, &b.backend, "wrong password", b.info(), &config())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::WrongPassword));
    assert!(
        !b.v().status().initialized,
        "a failed restore must leave no local vault"
    );

    restore(&b).await;
    // Restoring over an existing vault is refused.
    let err = restore_from_cloud(&b.vault, &b.backend, PW, b.info(), &config())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::VaultAlreadyInitialized));
    let _ = (server, clock);
}

#[tokio::test]
async fn restore_sets_up_sync_before_the_first_pull() {
    let (server, _clock, a, b) = world();
    for i in 0..5 {
        a.put(host(&format!("bulk-{i}")));
    }
    enable(&a).await;
    restore_from_cloud(&b.vault, &b.backend, PW, b.info(), &config())
        .await
        .unwrap();
    // Installed and configured, but nothing pulled yet.
    assert!(b.v().status().unlocked);
    assert_eq!(b.v().sync_config().unwrap(), Some(config()));
    assert!(b.names().is_empty());

    // The network drops before the first page arrives: the next round simply starts over.
    server.set_offline(true);
    assert!(matches!(b.try_sync().await, Err(Error::Offline)));
    server.set_offline(false);
    b.sync().await;
    assert_eq!(a.snapshot(), b.snapshot());
}

#[tokio::test]
async fn interrupted_first_pull_resumes_after_a_restart() {
    let (server, clock, a, _b) = world();
    for i in 0..7 {
        a.put(host(&format!("bulk-{i}")));
    }
    enable(&a).await;
    server.set_pull_page_cap(3);

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vault.db");
    let b = Dev {
        vault: share(Vault::open(&path).unwrap().with_clock(clock.clone())),
        backend: FakeBackend::new(&server),
        clock: clock.clone(),
        name: "device-b",
    };
    restore_from_cloud(&b.vault, &b.backend, PW, b.info(), &config())
        .await
        .unwrap();
    // The first page is answered, then the network drops.
    let offline = Arc::clone(&server);
    b.backend.on_next_pull(move || offline.set_offline(true));
    assert!(matches!(b.try_sync().await, Err(Error::Offline)));
    let cursor = b.v().sync_cursor();
    assert!(
        cursor > 0 && cursor < server.current_seq(),
        "cursor {cursor}"
    );
    let first_page = b.v().items().count();
    assert_eq!(first_page, 3);

    // The app restarts: the same database, unlocked with the master password, and the session
    // the app kept in the credential store (here: the same backend).
    let Dev { vault, backend, .. } = b;
    drop(vault);
    server.set_offline(false);
    let mut reopened = Vault::open(&path).unwrap().with_clock(clock.clone());
    assert!(reopened.status().initialized);
    assert_eq!(reopened.sync_config().unwrap(), Some(config()));
    reopened.unlock(PW).unwrap();
    let b = Dev {
        vault: share(reopened),
        backend,
        clock: clock.clone(),
        name: "device-b",
    };
    let report = b.sync().await;
    assert_eq!(
        report.pulled,
        u32::try_from(server.item_count() - first_page).unwrap(),
        "only the items after the saved cursor"
    );
    assert_eq!(a.snapshot(), b.snapshot());
    assert_eq!(b.v().sync_cursor(), server.current_seq());
}

#[tokio::test]
async fn restore_rejects_weakened_kdf_parameters_before_deriving_anything() {
    let (_server, _clock, a, b) = world();
    enable(&a).await;
    // A hostile server swaps in parameters that are structurally unacceptable.
    let hostile = FakeServer::new();
    let evil = FakeBackend::new(&hostile);
    let mut weak = a.v().local_meta().unwrap();
    weak.kdf_params = r#"{"alg":"argon2id","version":19,"m_kib":4,"t":1,"p":1}"#.into();
    evil.setup(VaultInit {
        schema_version: 1,
        kdf_salt: weak.kdf_salt.clone(),
        kdf_params: weak.kdf_params.clone(),
        auth_key: random_key().unwrap(),
        protected_vault_key: weak.protected_vault_key.clone(),
        recovery_vault_key: weak.recovery_vault_key.clone(),
        recovery_auth: random_key().unwrap(),
        setup_token: Some(Zeroizing::new(SETUP_TOKEN.into())),
    })
    .await
    .unwrap();
    let err = restore_from_cloud(&b.vault, &evil, PW, b.info(), &config())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::WeakKdfParams(_)), "got {err:?}");
    assert_eq!(
        hostile.session_count(),
        0,
        "must not log in (and so must not reveal auth_key) with bad parameters"
    );

    // Unknown algorithm is refused too.
    let hostile2 = FakeServer::new();
    let evil2 = FakeBackend::new(&hostile2);
    evil2
        .setup(VaultInit {
            schema_version: 1,
            kdf_salt: weak.kdf_salt.clone(),
            kdf_params: r#"{"alg":"pbkdf2","version":19,"m_kib":65536,"t":3,"p":4}"#.into(),
            auth_key: random_key().unwrap(),
            protected_vault_key: weak.protected_vault_key,
            recovery_vault_key: weak.recovery_vault_key,
            recovery_auth: random_key().unwrap(),
            setup_token: Some(Zeroizing::new(SETUP_TOKEN.into())),
        })
        .await
        .unwrap();
    let err = restore_from_cloud(&b.vault, &evil2, PW, b.info(), &config())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Format(_)));
    assert_eq!(hostile2.session_count(), 0);
}

// ---- ordinary two-way sync ------------------------------------------------------------------

#[tokio::test]
async fn bidirectional_edits_propagate() {
    let (_server, clock, a, b) = two_devices().await;

    clock.advance(1000);
    let from_a = a.put(host("added-on-a"));
    clock.advance(1000);
    let from_b = b.put(host("added-on-b"));
    let seed = a.find("seed-host").unwrap();
    clock.advance(1000);
    a.rename(&seed, "seed-renamed-on-a");

    let ra = a.sync().await;
    assert_eq!((ra.pushed, ra.pulled, ra.conflicts_resolved), (2, 0, 0));
    let rb = b.sync().await;
    assert_eq!((rb.pulled, rb.pushed, rb.conflicts_resolved), (2, 1, 0));
    let ra2 = a.sync().await;
    assert_eq!((ra2.pulled, ra2.pushed), (1, 0));

    assert_eq!(a.snapshot(), b.snapshot());
    assert_eq!(a.name_of(&from_b).as_deref(), Some("added-on-b"));
    assert_eq!(b.name_of(&from_a).as_deref(), Some("added-on-a"));
    assert_eq!(b.name_of(&seed).as_deref(), Some("seed-renamed-on-a"));
    assert_eq!(
        a.v().unreviewed_conflict_count() + b.v().unreviewed_conflict_count(),
        0
    );
}

#[tokio::test]
async fn offline_edits_merge_when_back_online() {
    let (server, clock, a, b) = two_devices().await;
    server.set_offline(true);

    clock.advance(1000);
    a.put(host("a-offline-1"));
    a.put(host("a-offline-2"));
    clock.advance(1000);
    b.put(host("b-offline-1"));
    assert!(matches!(a.try_sync().await, Err(Error::Offline)));
    assert!(matches!(b.try_sync().await, Err(Error::Offline)));
    assert_eq!(a.pending(), 2);
    assert_eq!(b.pending(), 1);
    // Local use is unaffected while offline.
    assert_eq!(a.names().len(), 3);

    server.set_offline(false);
    converge(&a, &b).await;
    assert_eq!(
        a.names(),
        ["a-offline-1", "a-offline-2", "b-offline-1", "seed-host"]
    );
}

#[tokio::test]
async fn deletions_propagate_as_tombstones() {
    let (server, clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();
    clock.advance(1000);
    a.v().delete(&seed).unwrap();
    a.sync().await;
    let tomb = server.item(&seed).unwrap();
    assert!(
        tomb.deleted && tomb.envelope.is_none(),
        "tombstones keep envelope = NULL"
    );
    b.sync().await;
    assert!(b.v().get(&seed).is_none());
    let row = b.v().store.item_row(&seed).unwrap().unwrap();
    assert!(row.deleted && row.envelope.is_none() && !row.dirty);
}

#[tokio::test]
async fn delete_vs_delete_leaves_one_tombstone_and_no_conflict() {
    let (_server, clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();
    clock.advance(1000);
    a.v().delete(&seed).unwrap();
    clock.advance(1000);
    b.v().delete(&seed).unwrap();
    converge(&a, &b).await;
    assert!(a.v().get(&seed).is_none() && b.v().get(&seed).is_none());
    assert_eq!(
        a.v().unreviewed_conflict_count() + b.v().unreviewed_conflict_count(),
        0
    );
}

// ---- conflicts ------------------------------------------------------------------------------

#[tokio::test]
async fn same_host_edited_on_both_devices_last_writer_wins_and_is_logged() {
    let (_server, clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();

    clock.advance(1000);
    a.rename(&seed, "edited-by-a");
    clock.advance(1000);
    b.rename(&seed, "edited-by-b"); // newer

    let ra = a.sync().await;
    assert_eq!(ra.conflicts_resolved, 0);
    let rb = b.sync().await;
    assert_eq!(rb.conflicts_resolved, 1);
    a.sync().await;

    assert_eq!(a.name_of(&seed).as_deref(), Some("edited-by-b"));
    assert_eq!(b.name_of(&seed).as_deref(), Some("edited-by-b"));
    assert_eq!(a.snapshot(), b.snapshot());

    // The conflict was logged on the device that resolved it, with both versions readable.
    let log = b.v().conflicts(false).unwrap();
    assert_eq!(log.len(), 1);
    let entry = &log[0];
    assert_eq!(entry.item_id, seed);
    assert_eq!(entry.resolution, Resolution::LocalWins);
    assert_eq!(entry.local.as_ref().unwrap().display_name(), "edited-by-b");
    assert_eq!(entry.remote.as_ref().unwrap().display_name(), "edited-by-a");
    assert!(!entry.reviewed);
    assert_eq!(b.v().unreviewed_conflict_count(), 1);
    assert_eq!(a.v().unreviewed_conflict_count(), 0);
    assert_eq!(
        b.names(),
        ["edited-by-b"],
        "no stray copies for non-key items"
    );
}

#[tokio::test]
async fn older_local_edit_loses_to_newer_remote() {
    let (_server, clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();
    clock.advance(1000);
    b.rename(&seed, "older-local");
    clock.advance(1000);
    a.rename(&seed, "newer-remote");
    a.sync().await;
    let rb = b.sync().await;
    assert_eq!(
        (rb.conflicts_resolved, rb.pushed),
        (1, 0),
        "the loser has nothing left to push"
    );
    assert_eq!(b.name_of(&seed).as_deref(), Some("newer-remote"));
    assert_eq!(b.pending(), 0);
    let log = b.v().conflicts(false).unwrap();
    assert_eq!(log[0].resolution, Resolution::RemoteWins);
    assert_eq!(log[0].local.as_ref().unwrap().display_name(), "older-local");
    converge(&a, &b).await;
}

#[tokio::test]
async fn timestamp_tie_goes_to_the_remote_on_both_sides() {
    let (_server, _clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();
    // Same updated_at on both edits: put() stamps "now", and the clock is shared and still.
    a.rename(&seed, "tie-a");
    b.rename(&seed, "tie-b");
    assert_eq!(
        a.v().get(&seed).unwrap().updated_at(),
        b.v().get(&seed).unwrap().updated_at()
    );
    a.sync().await; // pushes first: "tie-a" is the server version
    b.sync().await; // tie: remote ("tie-a") wins
    a.sync().await;
    assert_eq!(a.name_of(&seed).as_deref(), Some("tie-a"));
    assert_eq!(b.name_of(&seed).as_deref(), Some("tie-a"));
    assert_eq!(a.snapshot(), b.snapshot());
}

#[tokio::test]
async fn same_key_edited_on_both_devices_keeps_a_conflict_copy_on_both() {
    let (_server, clock, a, b) = world();
    let key_id = a.put(key("deploy-key", "BODY-0"));
    enable(&a).await;
    restore(&b).await;

    clock.advance(1000);
    a.edit(&key_id, |i| {
        if let Item::Key(k) = i {
            k.comment = "edited on A".into();
        }
    });
    clock.advance(1000);
    b.edit(&key_id, |i| {
        if let Item::Key(k) = i {
            k.comment = "edited on B".into();
        }
    });

    converge(&a, &b).await;

    // Both devices now hold the winner (B, newer) plus a copy of the loser (A's version).
    for dev in [&a, &b] {
        assert_eq!(dev.names(), ["deploy-key", "deploy-key (conflict copy)"]);
        let Item::Key(winner) = dev.v().get(&key_id).unwrap().clone() else {
            panic!()
        };
        assert_eq!(winner.comment, "edited on B");
        let copy_id = dev.find("deploy-key (conflict copy)").unwrap();
        assert_ne!(copy_id, key_id);
        let Item::Key(copy) = dev.v().get(&copy_id).unwrap().clone() else {
            panic!()
        };
        assert_eq!(copy.comment, "edited on A");
        assert_eq!(
            copy.private_key.as_str(),
            "BODY-0",
            "the key material itself is preserved"
        );
    }
    let log = b.v().conflicts(false).unwrap();
    assert_eq!(log[0].resolution, Resolution::LocalWinsRemoteCopied);
}

#[tokio::test]
async fn key_conflict_where_the_local_edit_loses_still_preserves_it() {
    let (_server, clock, a, b) = world();
    let key_id = a.put(key("ci-key", "BODY"));
    enable(&a).await;
    restore(&b).await;
    clock.advance(1000);
    b.edit(&key_id, |i| {
        if let Item::Key(k) = i {
            k.comment = "local-loser".into();
        }
    });
    clock.advance(1000);
    a.edit(&key_id, |i| {
        if let Item::Key(k) = i {
            k.comment = "remote-winner".into();
        }
    });
    a.sync().await;
    b.sync().await;
    let copy_id = b
        .find("ci-key (conflict copy)")
        .expect("loser saved as a copy");
    let Item::Key(copy) = b.v().get(&copy_id).unwrap().clone() else {
        panic!()
    };
    assert_eq!(copy.comment, "local-loser");
    assert_eq!(
        b.pending(),
        0,
        "the copy was created during the pull and uploaded in the same round"
    );
    converge(&a, &b).await;
    assert!(a.find("ci-key (conflict copy)").is_some());
    assert_eq!(
        b.v().conflicts(false).unwrap()[0].resolution,
        Resolution::RemoteWinsLocalCopied
    );
}

#[tokio::test]
async fn localized_conflict_suffix_is_used() {
    let (_server, clock, a, b) = world();
    let key_id = a.put(key("k", "BODY"));
    enable(&a).await;
    restore(&b).await;
    clock.advance(1000);
    a.rename(&key_id, "k-a");
    clock.advance(1000);
    b.rename(&key_id, "k-b");
    a.sync().await;
    let opts = SyncOptions {
        conflict_suffix: "（冲突副本）".into(),
    };
    sync_round(&b.vault, &b.backend, &opts).await.unwrap();
    assert!(
        b.names().contains(&"k-a（冲突副本）".to_owned()),
        "{:?}",
        b.names()
    );
}

#[tokio::test]
async fn delete_vs_modify_the_modified_side_wins() {
    // Local modifies, remote deletes.
    let (_server, clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();
    clock.advance(1000);
    a.v().delete(&seed).unwrap();
    clock.advance(1000);
    b.rename(&seed, "modified-after-delete");
    a.sync().await; // server: tombstone
    let rb = b.sync().await;
    assert_eq!(rb.conflicts_resolved, 1);
    a.sync().await;
    assert_eq!(
        a.name_of(&seed).as_deref(),
        Some("modified-after-delete"),
        "item resurrected on the deleter"
    );
    assert_eq!(b.name_of(&seed).as_deref(), Some("modified-after-delete"));
    assert_eq!(
        b.v().conflicts(false).unwrap()[0].resolution,
        Resolution::LocalWins
    );
    converge(&a, &b).await;

    // Local deletes, remote modifies; the deletion is *newer*, the edit still wins.
    let (_server, clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();
    clock.advance(1000);
    a.rename(&seed, "modified-remotely");
    clock.advance(1000);
    b.v().delete(&seed).unwrap();
    a.sync().await;
    b.sync().await;
    assert_eq!(b.name_of(&seed).as_deref(), Some("modified-remotely"));
    assert_eq!(b.pending(), 0);
    assert_eq!(
        b.v().conflicts(false).unwrap()[0].resolution,
        Resolution::RemoteWins
    );
    converge(&a, &b).await;
}

#[tokio::test]
async fn deleted_key_vs_modified_key_never_loses_the_key() {
    let (_server, clock, a, b) = world();
    let key_id = a.put(key("precious", "PRIVATE"));
    enable(&a).await;
    restore(&b).await;
    clock.advance(1000);
    a.v().delete(&key_id).unwrap();
    clock.advance(1000);
    b.rename(&key_id, "precious-edited");
    converge(&a, &b).await;
    for dev in [&a, &b] {
        let Item::Key(k) = dev.v().get(&key_id).unwrap().clone() else {
            panic!()
        };
        assert_eq!(k.private_key.as_str(), "PRIVATE");
    }
}

#[tokio::test]
async fn conflict_found_while_pushing_is_resolved_and_a_winning_local_edit_is_retried_once() {
    let (server, clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();

    clock.advance(1000);
    a.rename(&seed, "a-edit-older");
    clock.advance(1000);
    b.rename(&seed, "b-edit-newer");
    let (b_envelope, b_updated) = {
        let row = b.v().store.item_row(&seed).unwrap().unwrap();
        (row.envelope.unwrap(), row.updated_at)
    };

    // After A pulls (nothing new), B's edit lands on the server: A's push now conflicts.
    let srv = server.clone();
    let id = seed.clone();
    a.backend
        .on_next_pull(move || srv.inject_item(&id, Some(&b_envelope), false, b_updated));
    let report = a.sync().await;

    // The newer remote wins, A's local edit is discarded, nothing is left to push.
    assert_eq!(report.conflicts_resolved, 1);
    assert_eq!(report.pushed, 0);
    assert_eq!(report.pending_after, 0);
    assert_eq!(a.name_of(&seed).as_deref(), Some("b-edit-newer"));
    assert_eq!(
        a.v().conflicts(false).unwrap()[0].resolution,
        Resolution::RemoteWins
    );

    // Now the same race where the local edit is newer: it must win and be retried once.
    clock.advance(1000);
    b.sync().await;
    clock.advance(1000);
    b.rename(&seed, "b-second-older");
    clock.advance(1000);
    a.rename(&seed, "a-second-newer");
    let (b_envelope, b_updated) = {
        let row = b.v().store.item_row(&seed).unwrap().unwrap();
        (row.envelope.unwrap(), row.updated_at)
    };
    let srv = server.clone();
    let id = seed.clone();
    a.backend
        .on_next_pull(move || srv.inject_item(&id, Some(&b_envelope), false, b_updated));
    let pushes_before = server.push_calls();
    let report = a.sync().await;
    assert_eq!(
        report.pushed, 1,
        "the retry on top of the server revision succeeded"
    );
    assert_eq!(report.conflicts_resolved, 1);
    assert_eq!(report.pending_after, 0);
    assert_eq!(server.push_calls() - pushes_before, 2, "exactly one retry");
    assert_eq!(
        a.v().conflicts(false).unwrap()[0].resolution,
        Resolution::LocalWins
    );
    converge(&a, &b).await;
    assert_eq!(a.name_of(&seed).as_deref(), Some("a-second-newer"));
}

#[tokio::test]
async fn lost_push_response_converges_without_a_phantom_conflict() {
    let (server, clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();
    clock.advance(1000);
    a.rename(&seed, "applied-but-unacknowledged");
    server.lose_next_push_response();
    assert!(matches!(a.try_sync().await, Err(Error::Offline)));
    assert_eq!(
        a.pending(),
        1,
        "the client does not know the push succeeded"
    );

    let report = a.sync().await;
    assert_eq!(
        report.conflicts_resolved, 0,
        "identical content is not a conflict"
    );
    assert_eq!(report.pending_after, 0);
    assert_eq!(a.v().unreviewed_conflict_count(), 0);
    converge(&a, &b).await;
    assert_eq!(
        b.name_of(&seed).as_deref(),
        Some("applied-but-unacknowledged")
    );
}

// ---- engine mechanics -----------------------------------------------------------------------

#[tokio::test]
async fn cursor_advances_only_from_pulled_sequence_numbers() {
    let (server, clock, a, b) = two_devices().await;
    let cursor_b = b.v().sync_cursor();
    assert_eq!(cursor_b, server.current_seq());

    // A pushes two items; B's cursor is unchanged until it pulls.
    clock.advance(1000);
    a.put(host("n1"));
    a.put(host("n2"));
    a.sync().await;
    assert_eq!(b.v().sync_cursor(), cursor_b);
    assert_eq!(
        a.v().sync_cursor(),
        cursor_b,
        "A ignores the seq numbers of its own pushes"
    );

    let pulls = server.pull_calls();
    let rb = b.sync().await;
    assert_eq!(rb.pulled, 2);
    assert_eq!(b.v().sync_cursor(), server.current_seq());
    assert!(b.v().sync_last_at().is_some());

    // Nothing new: an empty pull, cursor unchanged.
    let cursor = b.v().sync_cursor();
    let rb = b.sync().await;
    assert_eq!((rb.pulled, rb.pushed), (0, 0));
    assert_eq!(b.v().sync_cursor(), cursor);
    assert!(server.pull_calls() > pulls);

    // A catches up on its own items by pulling them back; they are already current, so no-ops.
    let ra = a.sync().await;
    assert_eq!(ra.pulled, 0);
    assert_eq!(a.v().sync_cursor(), server.current_seq());
}

#[tokio::test]
async fn pagination_walks_every_page() {
    let (server, _clock, a, b) = world();
    for i in 0..7 {
        a.put(host(&format!("bulk-{i}")));
    }
    enable(&a).await;
    server.set_pull_page_cap(3);
    let pulls_before = server.pull_calls();
    restore(&b).await;
    assert_eq!(b.names().len(), 7);
    assert_eq!(a.snapshot(), b.snapshot());
    // 8 items (7 + settings) in pages of 3 = 3 pages (+1 for the sync_round after restore's first
    // pass is already at the end, so the last pull is empty or the final page).
    assert!(server.pull_calls() - pulls_before >= 3);
    assert_eq!(b.v().sync_cursor(), server.current_seq());
}

#[tokio::test]
async fn pushes_are_chunked_to_100_changes() {
    let (server, _clock, a, _b) = world();
    for i in 0..250 {
        a.put(host(&format!("many-{i}")));
    }
    let report = {
        enable(&a).await;
        a.sync().await
    };
    assert_eq!(report.pending_after, 0);
    assert_eq!(server.item_count(), 251);
    assert_eq!(server.pushed_change_count(), 251);
    assert_eq!(
        server.push_calls(),
        3,
        "251 changes need exactly three requests of ≤ 100"
    );
}

#[tokio::test]
async fn edit_during_an_in_flight_push_stays_dirty() {
    let (server, clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();
    clock.advance(1000);
    a.rename(&seed, "v1-being-pushed");
    let base_revision = server.item(&seed).unwrap().revision;

    // While each push is in flight the user edits the same item again (v2 during the first
    // push, v3 during the second).
    let vault = a.vault.clone();
    let id = seed.clone();
    let clock2 = a.clock.clone();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    a.backend.on_push(move || {
        let n = calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 2;
        clock2.advance(1000);
        let mut v = vault.lock().unwrap();
        let mut item = v.get(&id).unwrap().clone();
        if let Item::Host(h) = &mut item {
            h.name = format!("v{n}-typed-during-push");
        }
        v.put(Some(&id), item).unwrap();
    });
    let report = a.sync().await;
    a.backend.clear_push_hook();

    // v1 was accepted, the item stayed dirty, so v2 went out in the same round (revision +2);
    // v3 was typed during that second push and is still pending.
    assert_eq!(report.pushed, 2);
    assert_eq!(
        report.pending_after, 1,
        "the newest local edit must not be marked as pushed"
    );
    assert_eq!(report.conflicts_resolved, 0);
    let row = a.v().store.item_row(&seed).unwrap().unwrap();
    assert!(row.dirty);
    assert_eq!(
        row.revision,
        base_revision + 2,
        "revision tracks the server so the next push is a clean update"
    );
    assert_eq!(a.name_of(&seed).as_deref(), Some("v3-typed-during-push"));
    let on_server = {
        let key = a.v().vault_key_copy().unwrap();
        let env = server.item(&seed).unwrap().envelope.unwrap();
        crate::vault::decode_envelope(&key, &seed, &env)
            .unwrap()
            .display_name()
    };
    assert_eq!(on_server, "v2-typed-during-push");

    // The next round delivers v3, with no conflict.
    let report = a.sync().await;
    assert_eq!(
        (
            report.pushed,
            report.conflicts_resolved,
            report.pending_after
        ),
        (1, 0, 0)
    );
    converge(&a, &b).await;
    assert_eq!(b.name_of(&seed).as_deref(), Some("v3-typed-during-push"));
}

#[tokio::test]
async fn undecryptable_remote_items_are_skipped_not_fatal() {
    let (server, _clock, a, b) = two_devices().await;

    // 1. Encrypted under a different key.
    let alien = seal(
        &random_key().unwrap(),
        crate::crypto::item_aad("alien").as_bytes(),
        br#"{"type":"host"}"#,
    )
    .unwrap()
    .to_json();
    server.inject_item("alien", Some(&alien), false, T0);
    // 2. Valid ciphertext of an item type from a future app version.
    let vault_key = b.v().vault_key_copy().unwrap();
    let future = seal(
        &vault_key,
        crate::crypto::item_aad("future").as_bytes(),
        br#"{"type":"teleporter","x":1}"#,
    )
    .unwrap()
    .to_json();
    server.inject_item("future", Some(&future), false, T0);
    // 3. Not even JSON.
    server.inject_item("junk", Some("not an envelope"), false, T0);
    // 4. A healthy item alongside, which must still arrive.
    let healthy = a.put(host("healthy"));
    a.sync().await;

    let report = b.sync().await;
    assert_eq!(report.skipped, 3);
    assert_eq!(b.name_of(&healthy).as_deref(), Some("healthy"));
    for id in ["alien", "future", "junk"] {
        assert!(b.v().get(id).is_none());
    }
    // Unknown-but-valid rows are kept for a future version; the round did not fail.
    assert!(b.v().store.item_row("future").unwrap().is_some());
    assert_eq!(b.v().sync_cursor(), server.current_seq());
    // Skipped rows are not retried forever: the next round is quiet.
    let report = b.sync().await;
    assert_eq!((report.pulled, report.skipped), (0, 0));
}

#[tokio::test]
async fn local_edit_overwrites_an_unreadable_remote_version() {
    let (server, clock, a, _b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();
    clock.advance(1000);
    a.rename(&seed, "healthy-local");
    server.inject_item(
        &seed,
        Some("{\"v\":1,\"n\":\"AAAAAAAAAAAAAAAA\",\"c\":\"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA\"}"),
        false,
        T0,
    );
    let report = a.sync().await;
    assert_eq!((report.pushed, report.pending_after), (1, 0));
    assert_eq!(a.name_of(&seed).as_deref(), Some("healthy-local"));
    // The server now holds a readable version again.
    let stored = server.item(&seed).unwrap();
    assert!(stored.envelope.unwrap().len() > 60);
}

#[tokio::test]
async fn server_without_a_row_for_a_known_item_is_recovered_with_base_revision_zero() {
    let (server, clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();
    // Simulate a reset server: forget the row, but keep the vault meta.
    server.reset_items_for_test();
    clock.advance(1000);
    a.rename(&seed, "after-server-reset");
    let report = a.sync().await;
    assert_eq!(
        (report.pushed, report.pending_after, report.failed),
        (1, 0, 0)
    );
    assert_eq!(server.item(&seed).unwrap().revision, 1);
    let _ = b;
}

#[tokio::test]
async fn locked_vault_cannot_sync_and_unauthorized_is_reported() {
    let (server, _clock, a, _b) = two_devices().await;
    a.v().lock();
    assert!(matches!(a.try_sync().await, Err(Error::Locked)));
    a.v().unlock(PW).unwrap();

    server.revoke_all_sessions();
    assert!(matches!(a.try_sync().await, Err(Error::Unauthorized)));
    // Local data and the dirty queue are untouched by the failure.
    a.put(host("while-signed-out"));
    assert_eq!(a.pending(), 1);
    sign_in(&a.vault, &a.backend, PW, a.info()).await.unwrap();
    let report = a.sync().await;
    assert_eq!((report.pushed, report.pending_after), (1, 0));
}

#[tokio::test]
async fn settings_item_syncs_like_any_other() {
    let (_server, clock, a, b) = two_devices().await;
    clock.advance(1000);
    let mut s = a.v().settings();
    s.auto_lock_minutes = 42;
    a.v().put(None, Item::Settings(s)).unwrap();
    converge(&a, &b).await;
    assert_eq!(b.v().settings().auto_lock_minutes, 42);
}

#[tokio::test]
async fn device_terminal_prefs_move_in_after_the_pull_without_outdating_newer_edits() {
    let (_server, clock, a, b) = two_devices().await;
    // A edits the settings after B last synced, and records copy/paste for right-click.
    clock.advance(1000);
    let mut s = a.v().settings();
    s.terminal.font_size = 16;
    s.auto_lock_minutes = 42;
    s.terminal.right_click = Some(RightClick::CopyPaste);
    a.v().put(None, Item::Settings(s)).unwrap();
    a.sync().await;

    // B upgrades later. Its device-local prefs say menu and no paste confirmation; it moves
    // them in after its first round has pulled A's edit.
    clock.advance(1000);
    b.sync().await;
    assert!(
        b.v()
            .adopt_device_terminal_prefs(Some(RightClick::Menu), Some(false))
            .unwrap()
    );
    converge(&a, &b).await;

    let s = a.v().settings();
    assert_eq!(s.terminal.right_click, Some(RightClick::CopyPaste));
    assert_eq!(s.terminal.confirm_multiline_paste, Some(false));
    assert_eq!(s.terminal.font_size, 16);
    assert_eq!(s.auto_lock_minutes, 42);
    // B wrote on top of A's version, so nothing had to be resolved.
    assert!(b.v().conflicts(false).unwrap().is_empty());
}

// ---- password change, recovery, devices -----------------------------------------------------

#[tokio::test]
async fn password_change_revokes_other_devices_and_they_adopt_the_new_password() {
    let (server, clock, a, b) = two_devices().await;
    change_password_remote(&a.vault, &a.backend, PW, "the new password")
        .await
        .unwrap();

    // A keeps working with its own session; B was signed out by the server.
    a.put(host("after-change"));
    a.sync().await;
    assert!(matches!(b.try_sync().await, Err(Error::Unauthorized)));
    assert!(matches!(
        sign_in(&b.vault, &b.backend, PW, b.info()).await,
        Err(Error::WrongPassword)
    ));

    clock.advance(1000);
    sign_in(&b.vault, &b.backend, "the new password", b.info())
        .await
        .unwrap();
    b.sync().await;
    assert!(b.find("after-change").is_some());
    // B's local password now matches too.
    b.v().lock();
    assert!(matches!(b.v().unlock(PW), Err(Error::WrongPassword)));
    b.v().unlock("the new password").unwrap();
    // And a brand-new device can only join with the new password.
    let c = device(&server, &clock, "device-c", false);
    assert!(matches!(
        restore_from_cloud(&c.vault, &c.backend, PW, c.info(), &config()).await,
        Err(Error::WrongPassword)
    ));
    restore_from_cloud(
        &c.vault,
        &c.backend,
        "the new password",
        c.info(),
        &config(),
    )
    .await
    .unwrap();
    c.sync().await;
    assert!(c.find("after-change").is_some());
}

#[tokio::test]
async fn failed_remote_password_change_leaves_both_sides_on_the_old_password() {
    let (server, _clock, a, _b) = two_devices().await;
    server.set_offline(true);
    let err = change_password_remote(&a.vault, &a.backend, PW, "new-pw")
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Offline));
    server.set_offline(false);
    a.v().lock();
    a.v().unlock(PW).unwrap();
    assert!(matches!(a.v().unlock("new-pw"), Err(Error::WrongPassword)));
    // Wrong current password is rejected before anything is sent.
    assert!(matches!(
        change_password_remote(&a.vault, &a.backend, "wrong", "x").await,
        Err(Error::WrongPassword)
    ));
}

#[tokio::test]
async fn recovery_code_resets_the_password_on_an_existing_device() {
    let server = FakeServer::new();
    let clock = Arc::new(ManualClock::new(T0));
    let mut vault = Vault::open_in_memory().unwrap().with_clock(clock.clone());
    let code = vault
        .create_with_params(PW, KdfParams::for_tests())
        .unwrap();
    let a = Dev {
        vault: share(vault),
        backend: FakeBackend::new(&server),
        clock: clock.clone(),
        name: "device-a",
    };
    a.put(host("kept"));
    enable(&a).await;
    let b = device(&server, &clock, "device-b", false);
    restore(&b).await;

    // The user forgot the password on A.
    a.v().lock();
    let wrong = RecoveryCode::generate().unwrap().to_string();
    assert!(matches!(
        recover_remote(&a.vault, &a.backend, &wrong, "fresh-pw", a.info()).await,
        Err(Error::WrongRecoveryCode)
    ));
    assert!(matches!(
        recover_remote(&a.vault, &a.backend, "garbage", "fresh-pw", a.info()).await,
        Err(Error::WrongRecoveryCode)
    ));
    a.backend.set_session(None);

    let session = recover_remote(
        &a.vault,
        &a.backend,
        &code.to_string(),
        "fresh-pw",
        a.info(),
    )
    .await
    .unwrap();
    assert!(!session.token.is_empty());
    assert!(a.v().is_unlocked());
    assert!(a.find("kept").is_some());

    // Everything agrees on the new password; the old one is dead everywhere.
    a.v().lock();
    assert!(matches!(a.v().unlock(PW), Err(Error::WrongPassword)));
    a.v().unlock("fresh-pw").unwrap();
    assert!(
        matches!(b.try_sync().await, Err(Error::Unauthorized)),
        "other devices were signed out"
    );
    sign_in(&b.vault, &b.backend, "fresh-pw", b.info())
        .await
        .unwrap();
    b.sync().await;
}

#[tokio::test]
async fn recovery_code_restores_a_vault_on_a_fresh_device() {
    let server = FakeServer::new();
    let clock = Arc::new(ManualClock::new(T0));
    let mut vault = Vault::open_in_memory().unwrap().with_clock(clock.clone());
    let code = vault
        .create_with_params(PW, KdfParams::for_tests())
        .unwrap();
    let a = Dev {
        vault: share(vault),
        backend: FakeBackend::new(&server),
        clock: clock.clone(),
        name: "device-a",
    };
    a.put(host("from-the-old-laptop"));
    enable(&a).await;

    let fresh = device(&server, &clock, "new-laptop", false);
    recover_remote(
        &fresh.vault,
        &fresh.backend,
        &code.to_string(),
        "chosen-after-loss",
        fresh.info(),
    )
    .await
    .unwrap();
    assert!(fresh.v().is_unlocked());
    assert!(fresh.find("from-the-old-laptop").is_some());
    fresh.v().lock();
    fresh.v().unlock("chosen-after-loss").unwrap();
}

#[tokio::test]
async fn recovery_code_for_a_different_vault_is_refused() {
    let (server, clock, a, _b) = two_devices().await;
    // A second, unrelated local vault tries to "recover" using A's cloud vault code.
    let mut other = Vault::open_in_memory().unwrap().with_clock(clock.clone());
    let other_code = other
        .create_with_params("other-pw", KdfParams::for_tests())
        .unwrap();
    let dev = Dev {
        vault: share(other),
        backend: FakeBackend::new(&server),
        clock: clock.clone(),
        name: "other",
    };
    // The other vault's own code does not match the server → wrong code.
    assert!(matches!(
        recover_remote(
            &dev.vault,
            &dev.backend,
            &other_code.to_string(),
            "x",
            dev.info()
        )
        .await,
        Err(Error::WrongRecoveryCode)
    ));
    let _ = a;
}

#[tokio::test]
async fn rotating_the_recovery_code_invalidates_the_old_one_remotely() {
    let server = FakeServer::new();
    let clock = Arc::new(ManualClock::new(T0));
    let mut vault = Vault::open_in_memory().unwrap().with_clock(clock.clone());
    let old_code = vault
        .create_with_params(PW, KdfParams::for_tests())
        .unwrap();
    let a = Dev {
        vault: share(vault),
        backend: FakeBackend::new(&server),
        clock: clock.clone(),
        name: "device-a",
    };
    enable(&a).await;

    let new_code = rotate_recovery_remote(&a.vault, &a.backend, PW)
        .await
        .unwrap();
    assert_ne!(new_code, old_code);

    let fresh = device(&server, &clock, "fresh", false);
    assert!(matches!(
        recover_remote(
            &fresh.vault,
            &fresh.backend,
            &old_code.to_string(),
            "x",
            fresh.info()
        )
        .await,
        Err(Error::WrongRecoveryCode)
    ));
    recover_remote(
        &fresh.vault,
        &fresh.backend,
        &new_code.to_string(),
        "via-new-code",
        fresh.info(),
    )
    .await
    .unwrap();
    assert!(fresh.v().is_unlocked());
}

#[tokio::test]
async fn device_list_shows_decrypted_names_and_revocation_signs_a_device_out() {
    let (_server, _clock, a, b) = two_devices().await;
    let listed = devices(&a.vault, &a.backend).await.unwrap();
    assert_eq!(listed.len(), 2);
    let mine = listed.iter().find(|d| d.current).unwrap();
    assert_eq!(mine.name.as_deref(), Some("device-a"));
    assert_eq!(mine.platform.as_deref(), Some("test-os"));
    assert_eq!(mine.device_id, a.v().device_id());
    let other = listed.iter().find(|d| !d.current).unwrap();
    assert_eq!(
        other.name.as_deref(),
        Some("device-b"),
        "B's name is sealed with the vault key, not the provisional enc_key"
    );

    revoke_device(&a.backend, &other.device_id).await.unwrap();
    assert_eq!(devices(&a.vault, &a.backend).await.unwrap().len(), 1);
    assert!(matches!(b.try_sync().await, Err(Error::Unauthorized)));

    a.v().lock();
    assert!(matches!(
        devices(&a.vault, &a.backend).await,
        Err(Error::Locked)
    ));
}

// ---- engine over a real, file-backed vault --------------------------------------------------

#[tokio::test]
async fn engine_wrapper_and_persistence_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.db");
    let server = FakeServer::new();
    let clock = Arc::new(ManualClock::new(T0));
    {
        let mut v = Vault::open(&path).unwrap().with_clock(clock.clone());
        v.create_with_params(PW, KdfParams::for_tests()).unwrap();
        v.put(None, host("persisted")).unwrap();
        let vault = share(v);
        let backend = Arc::new(FakeBackend::new(&server));
        enable_sync(
            &vault,
            backend.as_ref(),
            PW,
            Some(SETUP_TOKEN),
            DeviceInfo {
                name: "a".into(),
                platform: "t".into(),
            },
        )
        .await
        .unwrap();
        vault
            .lock()
            .unwrap()
            .set_sync_config(Some(&SyncConfig::Worker {
                url: "https://x.example".into(),
                deployment: None,
            }))
            .unwrap();
        // Edit after the first sync, then "quit" with the change still pending.
        vault
            .lock()
            .unwrap()
            .put(None, host("pending-at-exit"))
            .unwrap();
    }
    // Restart: reopen, unlock, resume via the engine wrapper.
    let mut v = Vault::open(&path).unwrap().with_clock(clock.clone());
    v.unlock(PW).unwrap();
    assert_eq!(v.pending_count(), 1);
    assert_eq!(
        v.sync_config().unwrap(),
        Some(SyncConfig::Worker {
            url: "https://x.example".into(),
            deployment: None,
        })
    );
    let engine = SyncEngine::new(share(v), Arc::new(FakeBackend::new(&server)));
    // The saved session would come from the keychain; here the new backend simply signs in again.
    sign_in(
        engine.vault(),
        engine.backend().as_ref(),
        PW,
        DeviceInfo {
            name: "a".into(),
            platform: "t".into(),
        },
    )
    .await
    .unwrap();
    let report = engine
        .with_conflict_suffix("（冲突副本）")
        .sync()
        .await
        .unwrap();
    assert_eq!((report.pushed, report.pending_after), (1, 0));
    assert_eq!(server.item_count(), 3);
    let _ = new_id;
}

// ---- locking discipline ---------------------------------------------------------------------

fn assert_send<T: Send>(_: &T) {}

#[tokio::test]
async fn flows_are_send_so_no_mutex_guard_can_live_across_an_await() {
    // `std::sync::MutexGuard` is `!Send`: if any of these futures held the vault lock while
    // suspended, this would not compile.
    let (_server, _clock, a, _b) = world();
    let vault = &a.vault;
    let backend: &dyn SyncBackend = &a.backend;
    let opts = SyncOptions::default();
    assert_send(&sync_round(vault, backend, &opts));
    assert_send(&enable_sync(vault, backend, PW, None, a.info()));
    assert_send(&restore_from_cloud(vault, backend, PW, a.info(), &config()));
    assert_send(&sign_in(vault, backend, PW, a.info()));
    assert_send(&change_password_remote(vault, backend, PW, "x"));
    assert_send(&rotate_recovery_remote(vault, backend, PW));
    assert_send(&recover_remote(vault, backend, "code", "x", a.info()));
    assert_send(&devices(vault, backend));
    assert_send(&revoke_device(backend, "d"));
    let engine = SyncEngine::new(
        share(Vault::open_in_memory().unwrap()),
        Arc::new(FakeBackend::new(&FakeServer::new())),
    );
    assert_send(&engine.sync());
}

#[tokio::test]
async fn the_vault_is_not_locked_while_waiting_for_the_network() {
    let (_server, clock, a, _b) = two_devices().await;
    clock.advance(1000);
    a.put(host("trigger-a-push"));

    let (during_pull, during_push) = (
        Arc::new(std::sync::atomic::AtomicU8::new(0)),
        Arc::new(std::sync::atomic::AtomicU8::new(0)),
    );
    let (v1, flag1) = (a.vault.clone(), during_pull.clone());
    a.backend.on_next_pull(move || {
        flag1.store(
            if v1.try_lock().is_ok() { 1 } else { 2 },
            std::sync::atomic::Ordering::SeqCst,
        );
    });
    let (v2, flag2) = (a.vault.clone(), during_push.clone());
    a.backend.on_push(move || {
        flag2.store(
            if v2.try_lock().is_ok() { 1 } else { 2 },
            std::sync::atomic::Ordering::SeqCst,
        );
    });
    a.sync().await;
    assert_eq!(
        during_pull.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "vault was locked during pull"
    );
    assert_eq!(
        during_push.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "vault was locked during push"
    );
}

// ---- conflict log API -----------------------------------------------------------------------

#[tokio::test]
async fn conflict_log_review_and_restoring_the_loser() {
    let (_server, clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();
    clock.advance(1000);
    a.rename(&seed, "loser-from-a");
    clock.advance(1000);
    b.rename(&seed, "winner-from-b");
    a.sync().await;
    b.sync().await;
    a.sync().await;
    assert_eq!(a.name_of(&seed).as_deref(), Some("winner-from-b"));

    let id = {
        let log = b.v().conflicts(true).unwrap();
        assert_eq!(log.len(), 1);
        log[0].id
    };
    assert!(matches!(
        b.v().restore_conflict_loser(9999),
        Err(Error::ItemNotFound(_))
    ));

    // Restoring brings the losing version back as a brand-new edit, which then syncs and wins.
    clock.advance(1000);
    let restored = b.v().restore_conflict_loser(id).unwrap();
    assert_eq!(restored, seed);
    assert_eq!(b.name_of(&seed).as_deref(), Some("loser-from-a"));
    assert_eq!(b.pending(), 1);
    assert_eq!(
        b.v().unreviewed_conflict_count(),
        0,
        "restoring marks the entry reviewed"
    );
    assert!(b.v().conflicts(true).unwrap().is_empty());
    assert_eq!(
        b.v().conflicts(false).unwrap().len(),
        1,
        "the log keeps reviewed entries"
    );
    converge(&a, &b).await;
    assert_eq!(a.name_of(&seed).as_deref(), Some("loser-from-a"));

    // Marking as reviewed without restoring.
    clock.advance(1000);
    a.rename(&seed, "second-a");
    clock.advance(1000);
    b.rename(&seed, "second-b");
    a.sync().await;
    b.sync().await;
    let id = b.v().conflicts(true).unwrap()[0].id;
    b.v().mark_conflict_reviewed(id).unwrap();
    assert_eq!(b.v().unreviewed_conflict_count(), 0);
    assert_eq!(
        b.name_of(&seed).as_deref(),
        Some("second-b"),
        "reviewing changes no data"
    );
}

#[tokio::test]
async fn restoring_a_losing_deletion_deletes_the_item_again() {
    let (_server, clock, a, b) = two_devices().await;
    let seed = a.find("seed-host").unwrap();
    clock.advance(1000);
    b.v().delete(&seed).unwrap();
    clock.advance(1000);
    a.rename(&seed, "edited-instead-of-deleted");
    a.sync().await;
    b.sync().await; // the edit beats the deletion
    assert_eq!(
        b.name_of(&seed).as_deref(),
        Some("edited-instead-of-deleted")
    );
    let entry = b.v().conflicts(false).unwrap().remove(0);
    assert!(entry.local_deleted && !entry.remote_deleted);
    assert_eq!(entry.resolution, Resolution::RemoteWins);
    assert!(entry.local.is_none(), "a deletion has no content to show");
    assert_eq!(
        entry.remote.as_ref().unwrap().display_name(),
        "edited-instead-of-deleted"
    );

    clock.advance(1000);
    b.v().restore_conflict_loser(entry.id).unwrap();
    assert!(b.v().get(&seed).is_none());
    converge(&a, &b).await;
    assert!(a.v().get(&seed).is_none());
}
