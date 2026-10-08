//! Sync against a real Hatoba Worker (`workers/sync` under `wrangler dev`), proving the Rust
//! client and the Worker agree on the wire format. Skipped unless `HATOBA_WORKER_URL` is set:
//!
//! ```sh
//! cd workers/sync && echo SETUP_TOKEN=live-it-setup-token-0123456789 > .dev.vars
//! rm -rf .wrangler/state && npx wrangler d1 migrations apply hatoba --local && npx wrangler dev
//! HATOBA_WORKER_URL=http://127.0.0.1:8787 HATOBA_SETUP_TOKEN=live-it-setup-token-0123456789 \
//!   cargo test -p hatoba-core --test worker_live -- --nocapture
//! ```
//! The Worker must be freshly initialised (empty D1) because the test runs the first-device setup.

use std::sync::Arc;

use hatoba_core::model::{Host, HostAuth, Item, KeyAlgorithm, SshKey};
use hatoba_core::platform::DeviceInfo;
use hatoba_core::sync::{SharedVault, SyncBackend, SyncEngine, WorkerBackend, flows, share};
use hatoba_core::vault::Vault;
use zeroize::Zeroizing;

const PASSWORD: &str = "live-integration-master-password";

fn device(name: &str) -> DeviceInfo {
    DeviceInfo {
        name: name.to_owned(),
        platform: "Linux".to_owned(),
    }
}

fn host(name: &str, port: u16) -> Item {
    Item::Host(Host {
        name: name.to_owned(),
        address: "10.0.0.7".to_owned(),
        port,
        username: "deploy".to_owned(),
        auth: HostAuth::password("s3cret-host-password"),
        ..Host::default()
    })
}

fn host_port(v: &SharedVault, name: &str) -> Option<u16> {
    v.lock()
        .unwrap()
        .hosts()
        .into_iter()
        .find(|(_, h)| h.name == name)
        .map(|(_, h)| h.port)
}

async fn sync(v: &SharedVault, backend: &Arc<WorkerBackend>) {
    let backend: Arc<dyn SyncBackend> = backend.clone();
    SyncEngine::new(v.clone(), backend)
        .sync()
        .await
        .expect("sync round");
}

#[tokio::test(flavor = "multi_thread")]
async fn two_devices_through_a_real_worker() {
    let Ok(url) = std::env::var("HATOBA_WORKER_URL") else {
        eprintln!("HATOBA_WORKER_URL not set: skipping live Worker test");
        return;
    };
    let token = std::env::var("HATOBA_SETUP_TOKEN").ok();

    // Device A: local vault with a host and a key, then flow A (§6.6).
    let a = share(Vault::open_in_memory().unwrap());
    let host_id = {
        let mut v = a.lock().unwrap();
        v.create(PASSWORD).unwrap();
        v.put(
            None,
            Item::Key(SshKey {
                name: "deploy-key".to_owned(),
                algorithm: KeyAlgorithm::Ed25519,
                private_key: Zeroizing::new(
                    "-----BEGIN OPENSSH PRIVATE KEY-----\nfake\n".to_owned(),
                ),
                public_key: "ssh-ed25519 AAAA test".to_owned(),
                fingerprint: "SHA256:test".to_owned(),
                ..SshKey::default()
            }),
        )
        .unwrap();
        v.put(None, host("prod-api", 22)).unwrap()
    };
    let backend_a = Arc::new(WorkerBackend::new(&url).unwrap());
    let health = backend_a.health().await.expect("health");
    assert!(
        !health.initialized,
        "the Worker must start empty (reset its local D1)"
    );

    // A wrong setup token is rejected before anything is written.
    let wrong = flows::enable_sync(
        &a,
        backend_a.as_ref(),
        PASSWORD,
        Some("wrong-token"),
        device("A"),
    )
    .await;
    assert!(wrong.is_err(), "setup with a wrong token must fail");

    flows::enable_sync(
        &a,
        backend_a.as_ref(),
        PASSWORD,
        token.as_deref(),
        device("Device A"),
    )
    .await
    .expect("enable sync");
    assert_eq!(a.lock().unwrap().pending_count(), 0, "everything pushed");

    // Device B: flow B (restore from cloud).
    let b = share(Vault::open_in_memory().unwrap());
    let backend_b = Arc::new(WorkerBackend::new(&url).unwrap());
    let wrong_pw =
        flows::restore_from_cloud(&b, backend_b.as_ref(), "not-the-password", device("B")).await;
    assert!(
        wrong_pw.is_err(),
        "restore with the wrong password must fail"
    );
    let b = share(Vault::open_in_memory().unwrap());
    flows::restore_from_cloud(&b, backend_b.as_ref(), PASSWORD, device("Device B"))
        .await
        .expect("restore");
    assert_eq!(host_port(&b, "prod-api"), Some(22));
    assert_eq!(b.lock().unwrap().keys().len(), 1);
    match &b
        .lock()
        .unwrap()
        .get(&host_id)
        .and_then(Item::as_host)
        .unwrap()
        .auth
    {
        HostAuth::Password { password } => assert_eq!(password.as_str(), "s3cret-host-password"),
        other => panic!("unexpected auth {other:?}"),
    }

    // B edits, A pulls.
    b.lock()
        .unwrap()
        .put(Some(&host_id), host("prod-api", 2222))
        .unwrap();
    sync(&b, &backend_b).await;
    sync(&a, &backend_a).await;
    assert_eq!(host_port(&a, "prod-api"), Some(2222));

    // Concurrent edits: A edits first, B edits later; both sync → last writer (B) wins everywhere,
    // and the conflict is logged on the device that resolved it.
    a.lock()
        .unwrap()
        .put(Some(&host_id), host("prod-api", 2200))
        .unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    b.lock()
        .unwrap()
        .put(Some(&host_id), host("prod-api", 2300))
        .unwrap();
    sync(&a, &backend_a).await;
    sync(&b, &backend_b).await;
    sync(&a, &backend_a).await;
    assert_eq!(host_port(&a, "prod-api"), Some(2300));
    assert_eq!(host_port(&b, "prod-api"), Some(2300));
    assert!(b.lock().unwrap().unreviewed_conflict_count() >= 1);

    // Delete on A, B follows (tombstone).
    a.lock().unwrap().delete(&host_id).unwrap();
    sync(&a, &backend_a).await;
    sync(&b, &backend_b).await;
    assert_eq!(host_port(&b, "prod-api"), None);

    // Devices: both signed in, names decrypted locally.
    let devices = flows::devices(&a, backend_a.as_ref())
        .await
        .expect("devices");
    let names: Vec<_> = devices.iter().filter_map(|d| d.name.clone()).collect();
    assert!(
        names.contains(&"Device A".to_owned()) && names.contains(&"Device B".to_owned()),
        "{names:?}"
    );
    assert_eq!(devices.iter().filter(|d| d.current).count(), 1);

    // Revoking B signs it out.
    let b_id = b.lock().unwrap().device_id();
    flows::revoke_device(backend_a.as_ref(), &b_id)
        .await
        .expect("revoke");
    let after = SyncEngine::new(b.clone(), backend_b.clone() as Arc<dyn SyncBackend>)
        .sync()
        .await;
    assert!(
        matches!(after, Err(hatoba_core::Error::Unauthorized)),
        "{after:?}"
    );
    eprintln!("live Worker sync: OK");
}
