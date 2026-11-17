//! ProxyJump host chains (SSH-10), connection configuration versions, and salted secret hashes
//! used to detect changed credentials without holding or copying them (SEC-03, HOST-14).

use std::collections::HashSet;
use std::hash::{BuildHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use hatoba_core::model::{Host, HostAuth, Item};
use hatoba_core::vault::Vault;

use crate::ssh::{known_host_keys, resolve_proxy};

/// Maximum ProxyJump depth allowed before stopping (`chain`) or failing (`build_config`).
const MAX_JUMP_DEPTH: usize = 8;

/// What a host's connection depends on. A change to `config` (the host or a jump host was
/// edited) closes the dashboard's connection made before it; a change to either ends a hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Versions {
    pub(crate) config: String,
    /// The trusted host keys of every hop: a key accepted in a tab ends a host key hold.
    pub(crate) trust: String,
    /// Each jump host, outwards, with its own part of the two.
    pub(crate) jumps: Vec<JumpVersion>,
}

/// A jump host of a chain, and how it connects: what its shared hold is valid for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JumpVersion {
    pub(crate) id: String,
    pub(crate) version: String,
}

impl Versions {
    /// Only what the connection is made from counts, so starring a host or a sync pull that
    /// changes its notes neither ends a hold nor closes a connection.
    pub(crate) fn of(vault: &Vault, host: &Host) -> Self {
        let hops = chain(vault, host);
        let mut config = Vec::new();
        let mut trust = Vec::new();
        let mut jumps: Vec<(usize, JumpVersion)> = Vec::new();
        for (i, (id, hop)) in hops.iter().enumerate() {
            let hop_config = hop_config(vault, hop, i + 1 == hops.len());
            let mut keys: Vec<String> = known_host_keys(vault, &hop.address, hop.port)
                .into_iter()
                .map(|(_, k)| k.fingerprint)
                .collect();
            keys.sort();
            let hop_trust = keys.join(",");
            if let Some(id) = id {
                jumps.push((
                    i,
                    JumpVersion {
                        id: (*id).to_owned(),
                        version: String::new(),
                    },
                ));
            }
            config.push(hop_config);
            trust.push(hop_trust);
        }
        // A jump host connects through the hops outwards of it as well.
        let jumps = jumps
            .into_iter()
            .map(|(i, jump)| JumpVersion {
                version: format!("{}|{}", config[i..].join(";"), trust[i..].join(";")),
                ..jump
            })
            .collect();
        Self {
            config: config.join(";"),
            trust: trust.join(";"),
            jumps,
        }
    }

    pub(crate) fn all(&self) -> String {
        format!("{}|{}", self.config, self.trust)
    }

    /// [`Versions::config`] alone, without reading the trusted keys of every hop.
    pub(crate) fn config_of(vault: &Vault, host: &Host) -> String {
        let hops = chain(vault, host);
        hops.iter()
            .enumerate()
            .map(|(i, (_, hop))| hop_config(vault, hop, i + 1 == hops.len()))
            .collect::<Vec<_>>()
            .join(";")
    }
}

/// The version `take_connection` compares, for `ssh_connect`.
pub fn config_version(vault: &Vault, host: &Host) -> String {
    Versions::config_of(vault, host)
}

/// How one hop of a chain connects: its target, its credentials (hashed) and, on the first
/// hop (`outermost`), the proxy.
fn hop_config(vault: &Vault, hop: &Host, outermost: bool) -> String {
    let auth = match &hop.auth {
        HostAuth::Password { password } => {
            format!("password#{:x}", secret_hash(&[password]))
        }
        HostAuth::Key { key_id } => {
            let key = vault.get(key_id).and_then(Item::as_key).map(|k| {
                let passphrase = k.passphrase.as_deref().map_or("", |p| p.as_str());
                secret_hash(&[&k.private_key, passphrase])
            });
            format!("key:{key_id}#{:x}", key.unwrap_or(0))
        }
        HostAuth::Agent => "agent".to_owned(),
        HostAuth::Ask => "ask".to_owned(),
    };
    // The proxy only matters on the first hop, the outermost one.
    let proxy = if outermost {
        match resolve_proxy(vault, &hop.proxy) {
            Ok(Some(p)) => format!(
                "{:?} {}@{}:{}#{:x}",
                p.kind,
                p.username,
                p.host,
                p.port,
                secret_hash(&[&p.password])
            ),
            Ok(None) => "direct".to_owned(),
            Err(_) => "missing".to_owned(),
        }
    } else {
        String::new()
    };
    format!(
        "{}@{}:{} {auth} {proxy}",
        hop.username, hop.address, hop.port
    )
}

/// Mixed into every hash of secrets, and replaced at the lock: a hash made before it, which
/// may linger in memory, can then no longer be checked against guesses. 0 until first used.
static SALT: AtomicU64 = AtomicU64::new(0);

/// A salt from the system's random source, never 0.
fn new_salt() -> u64 {
    let random = uuid::Uuid::new_v4().as_u128();
    ((random as u64) ^ ((random >> 64) as u64)).max(1)
}

/// The lock (SEC-03): hashes of secrets start over, so none made while the vault was unlocked
/// stays useful after it locks. Versions made before it no longer match any made after it.
pub fn forget_secret_hashes() {
    SALT.store(new_salt(), Ordering::Relaxed);
}

/// A hash of secrets, so a version can tell a changed password from the old one without
/// holding either, or copying it.
fn secret_hash(parts: &[&str]) -> u64 {
    let salt = match SALT.load(Ordering::Relaxed) {
        0 => {
            let fresh = new_salt();
            SALT.compare_exchange(0, fresh, Ordering::Relaxed, Ordering::Relaxed)
                .map_or_else(|set| set, |_| fresh)
        }
        salt => salt,
    };
    salted_hash(salt, parts)
}

fn salted_hash(salt: u64, parts: &[&str]) -> u64 {
    static KEY: std::sync::OnceLock<std::collections::hash_map::RandomState> =
        std::sync::OnceLock::new();
    let mut hasher = KEY.get_or_init(Default::default).build_hasher();
    salt.hash(&mut hasher);
    for part in parts {
        part.hash(&mut hasher);
    }
    hasher.finish()
}

/// The host, then its jump hosts outwards with their ids, as far as the chain goes without
/// looping or exceeding [`MAX_JUMP_DEPTH`] jump hosts. Stops silently on a loop or missing jump
/// host; [`crate::ssh::build_config`] walks the chain separately to report errors instead.
pub(crate) fn chain<'a>(vault: &'a Vault, host: &'a Host) -> Vec<(Option<&'a str>, &'a Host)> {
    let mut hops = vec![(None, host)];
    let mut seen = HashSet::new();
    let mut next = host.jump_host_id.as_deref();
    while let Some(id) = next.filter(|id| seen.insert(*id) && seen.len() <= MAX_JUMP_DEPTH) {
        let Some(jump) = vault.get(id).and_then(Item::as_host) else {
            break;
        };
        hops.push((Some(id), jump));
        next = jump.jump_host_id.as_deref();
    }
    hops
}

/// The host, then its jump hosts outwards.
pub(crate) fn hops<'a>(vault: &'a Vault, host: &'a Host) -> Vec<&'a Host> {
    chain(vault, host).into_iter().map(|(_, h)| h).collect()
}

/// The first hop of a connection to `host`: its outermost jump host, or the host itself without
/// one. A broken or over-deep jump chain ends where it breaks.
pub fn first_hop<'a>(vault: &'a Vault, host: &'a Host) -> &'a Host {
    chain(vault, host).last().map_or(host, |(_, h)| h)
}

#[cfg(test)]
mod tests {
    use hatoba_core::KdfParams;
    use hatoba_core::model::{HostProxy, KnownHost};

    use super::*;

    fn vault() -> Vault {
        let mut vault = Vault::open_in_memory().unwrap();
        vault
            .create_with_params(&hatoba_core::new_id(), KdfParams::for_tests())
            .unwrap();
        vault
    }

    fn host(address: &str, auth: HostAuth, jump: Option<&str>) -> Item {
        Item::Host(Host {
            name: address.to_owned(),
            address: address.to_owned(),
            port: 22,
            username: "deploy".to_owned(),
            auth,
            jump_host_id: jump.map(str::to_owned),
            proxy: HostProxy::Direct,
            ..Host::default()
        })
    }

    fn password(p: &str) -> HostAuth {
        HostAuth::Password {
            password: zeroize::Zeroizing::new(p.to_owned()),
        }
    }

    fn get(v: &Vault, id: &str) -> Host {
        v.get(id).and_then(Item::as_host).cloned().unwrap()
    }

    #[test]
    fn salted_hashes_differ_by_salt() {
        assert_eq!(salted_hash(7, &["pw"]), salted_hash(7, &["pw"]));
        assert_ne!(salted_hash(7, &["pw"]), salted_hash(8, &["pw"]));
        let (a, b) = (new_salt(), new_salt());
        assert!(a != 0 && b != 0 && a != b);
    }

    #[test]
    fn versions_follow_the_chain_and_its_trusted_keys() {
        let mut v = vault();
        let bastion = v.put(None, host("bastion", password("pw"), None)).unwrap();
        let id = v
            .put(None, host("app", password("pw"), Some(&bastion)))
            .unwrap();
        let before = Versions::of(&v, &get(&v, &id));
        assert_eq!(before, Versions::of(&v, &get(&v, &id)));

        // A key accepted for the jump host changes the trust, not the configuration.
        for fingerprint in ["SHA256:b", "SHA256:a"] {
            v.put(
                None,
                Item::KnownHost(KnownHost {
                    host: "bastion".to_owned(),
                    port: 22,
                    key_type: fingerprint.to_owned(),
                    public_key: fingerprint.to_owned(),
                    fingerprint: fingerprint.to_owned(),
                    first_seen_at: 1,
                    updated_at: 0,
                }),
            )
            .unwrap();
        }
        let trusted = Versions::of(&v, &get(&v, &id));
        assert_eq!(trusted.config, before.config);
        assert_eq!(config_version(&v, &get(&v, &id)), trusted.config);
        assert_ne!(trusted.trust, before.trust);
        assert!(trusted.trust.contains("SHA256:a,SHA256:b"));

        // Starring or renaming a host changes nothing a connection is made from.
        let mut starred = get(&v, &id);
        starred.favorite = true;
        starred.name = "renamed".to_owned();
        v.put(Some(&id), Item::Host(starred)).unwrap();
        assert_eq!(Versions::of(&v, &get(&v, &id)), trusted);

        // A new password does, as does editing the jump host.
        let mut repassed = get(&v, &id);
        repassed.auth = password("new");
        v.put(Some(&id), Item::Host(repassed)).unwrap();
        let repassed = Versions::of(&v, &get(&v, &id));
        assert_ne!(repassed.config, trusted.config);
        assert!(!repassed.config.contains("new"));
        let mut jump = get(&v, &bastion);
        jump.port = 2222;
        v.put(Some(&bastion), Item::Host(jump)).unwrap();
        let edited = Versions::of(&v, &get(&v, &id));
        assert_ne!(edited.config, repassed.config);
        assert_eq!(config_version(&v, &get(&v, &id)), edited.config);
        assert_ne!(edited.all(), repassed.all());
    }

    #[test]
    fn each_jump_host_has_a_version_of_its_own() {
        let mut v = vault();
        let bastion = v.put(None, host("bastion", password("pw"), None)).unwrap();
        let id = v
            .put(None, host("app", password("pw"), Some(&bastion)))
            .unwrap();
        let before = Versions::of(&v, &get(&v, &id));
        assert_eq!(before.jumps.len(), 1);
        assert_eq!(before.jumps[0].id, bastion);
        // The jump host's own connection shares its gate and holds with the hosts behind it.
        assert_eq!(
            Versions::of(&v, &get(&v, &bastion)).all(),
            before.jumps[0].version
        );
        // The destination's own password does not change the jump host's version.
        let mut app = get(&v, &id);
        app.auth = password("new");
        v.put(Some(&id), Item::Host(app)).unwrap();
        let after = Versions::of(&v, &get(&v, &id));
        assert_eq!(after.jumps, before.jumps);
        let mut jump = get(&v, &bastion);
        jump.auth = password("new");
        v.put(Some(&bastion), Item::Host(jump)).unwrap();
        let repassed = Versions::of(&v, &get(&v, &id)).jumps;
        assert_ne!(repassed, before.jumps);
        // So does a change to the hops outwards of the jump host.
        let outer = v.put(None, host("outer", password("pw"), None)).unwrap();
        let mut jump = get(&v, &bastion);
        jump.jump_host_id = Some(outer);
        v.put(Some(&bastion), Item::Host(jump)).unwrap();
        assert_ne!(Versions::of(&v, &get(&v, &id)).jumps[0], repassed[0]);
    }

    #[test]
    fn chain_characterization_loop_missing_and_over_eight() {
        let mut v = vault();
        // Cycle back to self (b -> a -> b): `chain` visits `b` once as a jump before stopping.
        let a = v.put(None, host("a", password("pw"), None)).unwrap();
        let b = v.put(None, host("b", password("pw"), Some(&a))).unwrap();
        let mut ha = get(&v, &a);
        ha.jump_host_id = Some(b.clone());
        v.put(Some(&a), Item::Host(ha)).unwrap();
        let hb = get(&v, &b);
        let addrs: Vec<_> = chain(&v, &hb)
            .into_iter()
            .map(|(id, h)| (id.map(str::to_owned), h.address.clone()))
            .collect();
        assert_eq!(
            addrs,
            vec![
                (None, "b".to_owned()),
                (Some(a.clone()), "a".to_owned()),
                (Some(b.clone()), "b".to_owned()),
            ]
        );

        // Missing jump host: `chain` stops at the missing hop without error.
        let j1 = v
            .put(None, host("j1", password("pw"), Some("missing-jump")))
            .unwrap();
        let h_missing = v
            .put(None, host("target", password("pw"), Some(&j1)))
            .unwrap();
        let ht = get(&v, &h_missing);
        let addrs: Vec<_> = chain(&v, &ht)
            .into_iter()
            .map(|(id, h)| (id.map(str::to_owned), h.address.clone()))
            .collect();
        assert_eq!(
            addrs,
            vec![(None, "target".to_owned()), (Some(j1), "j1".to_owned())]
        );

        // Chain more than 8 deep: `chain` keeps the host plus 8 jump hosts (9 entries).
        let mut prev: Option<String> = None;
        for i in (1..=9).rev() {
            let id = v
                .put(
                    None,
                    host(&format!("j{i}"), password("pw"), prev.as_deref()),
                )
                .unwrap();
            prev = Some(id);
        }
        let deep = v
            .put(None, host("leaf", password("pw"), prev.as_deref()))
            .unwrap();
        let hd = get(&v, &deep);
        let addrs: Vec<_> = hops(&v, &hd)
            .into_iter()
            .map(|h| h.address.clone())
            .collect();
        assert_eq!(
            addrs,
            vec!["leaf", "j1", "j2", "j3", "j4", "j5", "j6", "j7", "j8"]
        );
    }
}
