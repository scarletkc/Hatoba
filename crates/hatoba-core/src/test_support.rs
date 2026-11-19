//! Shared fixtures for `hatoba-core` unit tests.

use std::sync::Arc;

use zeroize::Zeroizing;

use crate::crypto::KdfParams;
use crate::model::{Host, Item, SshKey};
use crate::recovery::RecoveryCode;
use crate::vault::{ManualClock, Vault};

pub(crate) const TEST_PASSWORD: &str = "correct horse battery staple";
pub(crate) const T0: i64 = 1_700_000_000_000;

pub(crate) fn vault_on(clock: &Arc<ManualClock>) -> Vault {
    Vault::open_in_memory().unwrap().with_clock(clock.clone())
}

pub(crate) fn unlocked_vault_on(clock: &Arc<ManualClock>, password: &str) -> (Vault, RecoveryCode) {
    let mut vault = vault_on(clock);
    let code = vault
        .create_with_params(password, KdfParams::for_tests())
        .unwrap();
    (vault, code)
}

pub(crate) fn unlocked_vault_at(
    now_ms: i64,
    password: &str,
) -> (Vault, Arc<ManualClock>, RecoveryCode) {
    let clock = Arc::new(ManualClock::new(now_ms));
    let (vault, code) = unlocked_vault_on(&clock, password);
    (vault, clock, code)
}

pub(crate) fn unlocked_vault() -> (Vault, Arc<ManualClock>) {
    let (vault, clock, _) = unlocked_vault_at(T0, TEST_PASSWORD);
    (vault, clock)
}

pub(crate) fn host_item(name: &str) -> Item {
    host_item_with(name, |_| {})
}

pub(crate) fn host_item_with(name: &str, edit: impl FnOnce(&mut Host)) -> Item {
    let mut host = Host {
        name: name.into(),
        address: format!("{name}.example.org"),
        ..Host::default()
    };
    edit(&mut host);
    Item::Host(host)
}

pub(crate) fn key_item(name: &str, body: &str) -> Item {
    key_item_with(name, body, |_| {})
}

pub(crate) fn key_item_with(name: &str, body: &str, edit: impl FnOnce(&mut SshKey)) -> Item {
    let mut key = SshKey {
        name: name.into(),
        private_key: Zeroizing::new(body.into()),
        ..SshKey::default()
    };
    edit(&mut key);
    Item::Key(key)
}
