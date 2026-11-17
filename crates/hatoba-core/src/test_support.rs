//! Shared fixtures for `hatoba-core` unit tests.

use std::sync::Arc;

use zeroize::Zeroizing;

use crate::crypto::KdfParams;
use crate::model::{Host, Item, SshKey};
use crate::recovery::RecoveryCode;
use crate::vault::{ManualClock, Vault};

pub(crate) const TEST_PASSWORD: &str = "correct horse battery staple";
pub(crate) const T0: i64 = 1_700_000_000_000;

/// An uninitialized in-memory vault on `clock`.
pub(crate) fn vault_on(clock: &Arc<ManualClock>) -> Vault {
    Vault::open_in_memory().unwrap().with_clock(clock.clone())
}

/// An unlocked in-memory vault on `clock`, created with `password` and `KdfParams::for_tests()`.
pub(crate) fn unlocked_vault_on(clock: &Arc<ManualClock>, password: &str) -> (Vault, RecoveryCode) {
    let mut vault = vault_on(clock);
    let code = vault
        .create_with_params(password, KdfParams::for_tests())
        .unwrap();
    (vault, code)
}

/// An unlocked in-memory vault on a fresh `ManualClock` at `now_ms`.
pub(crate) fn unlocked_vault_at(
    now_ms: i64,
    password: &str,
) -> (Vault, Arc<ManualClock>, RecoveryCode) {
    let clock = Arc::new(ManualClock::new(now_ms));
    let (vault, code) = unlocked_vault_on(&clock, password);
    (vault, clock, code)
}

/// An unlocked in-memory vault at [`T0`] with [`TEST_PASSWORD`].
pub(crate) fn unlocked_vault() -> (Vault, Arc<ManualClock>) {
    let (vault, clock, _) = unlocked_vault_at(T0, TEST_PASSWORD);
    (vault, clock)
}

/// A minimal SSH [`Item::Host`] for `name` at `<name>.example.org`.
pub(crate) fn host_item(name: &str) -> Item {
    Item::Host(Host {
        name: name.into(),
        address: format!("{name}.example.org"),
        ..Host::default()
    })
}

/// An SSH [`Item::Host`] built like [`host_item`] and customised with `edit`.
pub(crate) fn host_item_with(name: &str, edit: impl FnOnce(&mut Host)) -> Item {
    let mut host = Host {
        name: name.into(),
        address: format!("{name}.example.org"),
        ..Host::default()
    };
    edit(&mut host);
    Item::Host(host)
}

/// A minimal [`Item::Key`] with `name` and private key `body`.
pub(crate) fn key_item(name: &str, body: &str) -> Item {
    Item::Key(SshKey {
        name: name.into(),
        private_key: Zeroizing::new(body.into()),
        ..SshKey::default()
    })
}

/// An [`Item::Key`] built like [`key_item`] and customised with `edit`.
pub(crate) fn key_item_with(name: &str, body: &str, edit: impl FnOnce(&mut SshKey)) -> Item {
    let mut key = SshKey {
        name: name.into(),
        private_key: Zeroizing::new(body.into()),
        ..SshKey::default()
    };
    edit(&mut key);
    Item::Key(key)
}
