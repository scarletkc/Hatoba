//! Key derivation and the authenticated-encryption envelope (spec §4.1 and §4.2).
//!
//! ```text
//! password ─ Argon2id(kdf_salt, KdfParams) → master_key (32 B)
//!               ├─ HKDF-SHA256(info = "hatoba/enc/v1")  → enc_key   (never leaves the client)
//!               └─ HKDF-SHA256(info = "hatoba/auth/v1") → auth_key  (sent to the Worker)
//! vault_key (32 random B) ─ AES-256-GCM(enc_key)      → protected_vault_key
//!                         └ AES-256-GCM(recovery_key) → recovery_vault_key
//! every item ─ AES-256-GCM(vault_key, aad = "hatoba/item/v1/{id}")
//! ```
//!
//! All key material is held in [`Zeroizing`] containers.

use aes_gcm::aead::{AeadInOut, Nonce};
use aes_gcm::{Aes256Gcm, KeyInit};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::{Error, Result};

/// Length in bytes of every symmetric key in the hierarchy.
pub const KEY_LEN: usize = 32;
/// Length in bytes of the Argon2 salt.
pub const SALT_LEN: usize = 16;
/// Length in bytes of an AES-GCM nonce.
pub const NONCE_LEN: usize = 12;
/// Length in bytes of the AES-GCM authentication tag.
pub const TAG_LEN: usize = 16;
/// The only envelope version this build reads and writes.
pub const ENVELOPE_VERSION: u8 = 1;

/// A 32-byte secret that wipes itself on drop.
pub type Key32 = Zeroizing<[u8; KEY_LEN]>;

// ---- AAD (associated data) domain separation ---------------------------------------------

/// Prefix of the AAD of an item envelope: `hatoba/item/v1/{item_id}` (spec §4.2).
pub const AAD_ITEM_PREFIX: &str = "hatoba/item/v1/";
/// AAD of `protected_vault_key` (the vault key wrapped by `enc_key`).
pub const AAD_VAULT_KEY: &str = "hatoba/vault-key/v1";
/// AAD of `recovery_vault_key` (the vault key wrapped by `recovery_key`).
pub const AAD_RECOVERY_VAULT_KEY: &str = "hatoba/recovery-vault-key/v1";
/// Prefix of the AAD of a sealed device name: `hatoba/device/v1/{device_id}`.
pub const AAD_DEVICE_PREFIX: &str = "hatoba/device/v1/";
/// AAD of the local-only `recovery_auth_sealed` meta value (recovery_auth under the vault key).
pub const AAD_RECOVERY_AUTH: &str = "hatoba/recovery-auth/v1";
/// AAD of the local-only `vault_check` meta value used to verify a candidate vault key.
pub const AAD_VAULT_CHECK: &str = "hatoba/vault-check/v1";
/// AAD of the local-only `recent_targets` meta value (recent quick-connect targets).
pub const AAD_RECENT_TARGETS: &str = "hatoba/recent-targets/v1";

/// AAD for the item with the given id. Conflict-log copies reuse the item AAD of their item.
#[must_use]
pub fn item_aad(item_id: &str) -> String {
    format!("{AAD_ITEM_PREFIX}{item_id}")
}

/// AAD for the sealed name of the device with the given id.
#[must_use]
pub fn device_aad(device_id: &str) -> String {
    format!("{AAD_DEVICE_PREFIX}{device_id}")
}

// ---- randomness ---------------------------------------------------------------------------

/// Fills a fixed-size array from the OS CSPRNG.
///
/// # Errors
/// [`Error::Random`] if the OS generator fails.
pub fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).map_err(|_| Error::Random)?;
    Ok(out)
}

/// A fresh random 32-byte key.
///
/// # Errors
/// [`Error::Random`] if the OS generator fails.
pub fn random_key() -> Result<Key32> {
    let mut key: Key32 = Zeroizing::new([0u8; KEY_LEN]);
    getrandom::fill(&mut *key).map_err(|_| Error::Random)?;
    Ok(key)
}

/// A fresh random Argon2 salt.
///
/// # Errors
/// [`Error::Random`] if the OS generator fails.
pub fn random_salt() -> Result<[u8; SALT_LEN]> {
    random_bytes::<SALT_LEN>()
}

// ---- base64 / hex helpers -----------------------------------------------------------------

/// Standard (padded) base64, as used for salts, nonces, ciphertexts and the wire format.
#[must_use]
pub fn b64_encode(bytes: &[u8]) -> String {
    B64.encode(bytes)
}

/// Decodes standard padded base64.
///
/// # Errors
/// [`Error::Format`] on invalid input.
pub fn b64_decode(s: &str) -> Result<Vec<u8>> {
    B64.decode(s)
        .map_err(|_| Error::Format("invalid base64".into()))
}

/// Lowercase hex encoding.
#[must_use]
pub fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// `hex(SHA-256(bytes))`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_encode(&Sha256::digest(bytes))
}

/// `hex(SHA-256(auth_key))`: what the Worker stores as `auth_hash` (the client sends the raw key).
#[must_use]
pub fn auth_hash(auth_key: &[u8; KEY_LEN]) -> String {
    sha256_hex(auth_key)
}

// ---- KDF parameters -----------------------------------------------------------------------

/// The KDF algorithm named in `kdf_params`. Unknown names fail to deserialize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum KdfAlg {
    /// Argon2id (RFC 9106).
    #[serde(rename = "argon2id")]
    Argon2id,
}

/// Argon2 version number 0x13 = 19, the only one supported.
const ARGON2_VERSION: u32 = 19;
/// Largest memory cost accepted from any source (512 MiB), so a hostile server cannot make a
/// device allocate gigabytes.
const MAX_M_KIB: u32 = 512 * 1024;
const MAX_T: u32 = 32;
const MAX_P: u32 = 16;

/// The weakest cost parameters this build accepts.
///
/// A hostile Worker (or a tampered local database) could otherwise downgrade the KDF so the
/// `auth_key` derived from the password becomes cheap to brute-force. Parameters are therefore
/// never accepted if weaker than the spec defaults (stronger ones are fine).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfFloor {
    m_kib: u32,
    t: u32,
    p: u32,
}

impl KdfFloor {
    /// Production floor: the spec defaults, `m = 64 MiB, t = 3, p = 1`.
    pub const PRODUCTION: Self = Self {
        m_kib: 64 * 1024,
        t: 3,
        p: 1,
    };
    /// Structural minimum only, for unit tests.
    pub const RELAXED: Self = Self {
        m_kib: 8,
        t: 1,
        p: 1,
    };

    /// The floor in force for this build: relaxed under `cfg(test)` or the `test-util` feature,
    /// [`Self::PRODUCTION`] otherwise.
    #[must_use]
    pub const fn active() -> Self {
        if cfg!(any(test, feature = "test-util")) {
            Self::RELAXED
        } else {
            Self::PRODUCTION
        }
    }
}

#[derive(Deserialize)]
struct RawKdfParams {
    alg: KdfAlg,
    version: u32,
    m_kib: u32,
    t: u32,
    p: u32,
}

impl TryFrom<RawKdfParams> for KdfParams {
    type Error = String;

    fn try_from(raw: RawKdfParams) -> std::result::Result<Self, String> {
        Self::from_raw(raw).map_err(|e| e.to_string())
    }
}

/// Argon2id cost parameters, persisted as JSON in `meta.kdf_params` (spec §4.1) so they can be
/// raised later without breaking old vaults.
///
/// Fields are private and every value is validated, against the [`KdfFloor`] in force, on
/// construction and on deserialization. A weak or absurd parameter set therefore cannot exist
/// in memory, which is also what lets [`derive_master_key`] be infallible.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawKdfParams")]
pub struct KdfParams {
    alg: KdfAlg,
    version: u32,
    m_kib: u32,
    t: u32,
    p: u32,
}

impl Default for KdfParams {
    /// The spec values: Argon2id v19, m = 64 MiB, t = 3, p = 4.
    fn default() -> Self {
        Self {
            alg: KdfAlg::Argon2id,
            version: ARGON2_VERSION,
            m_kib: 64 * 1024,
            t: 3,
            p: 4,
        }
    }
}

impl KdfParams {
    /// Builds validated Argon2id parameters.
    ///
    /// # Errors
    /// [`Error::WeakKdfParams`] if a value is out of the supported range or below the floor.
    pub fn new(m_kib: u32, t: u32, p: u32) -> Result<Self> {
        let params = Self {
            alg: KdfAlg::Argon2id,
            version: ARGON2_VERSION,
            m_kib,
            t,
            p,
        };
        params.validate()?;
        Ok(params)
    }

    /// Tiny parameters (64 KiB, 1 pass, 1 lane) so tests run in milliseconds.
    ///
    /// **Test-only**: these offer no brute-force resistance, so the function (and the relaxed
    /// floor that accepts such parameters) exists only under `cfg(test)` or the `test-util`
    /// cargo feature. Release builds reject them everywhere.
    #[cfg(any(test, feature = "test-util"))]
    #[must_use]
    pub fn for_tests() -> Self {
        Self {
            alg: KdfAlg::Argon2id,
            version: ARGON2_VERSION,
            m_kib: 64,
            t: 1,
            p: 1,
        }
    }

    /// Memory cost in KiB.
    #[must_use]
    pub fn m_kib(&self) -> u32 {
        self.m_kib
    }

    /// Number of passes.
    #[must_use]
    pub fn t(&self) -> u32 {
        self.t
    }

    /// Degree of parallelism.
    #[must_use]
    pub fn p(&self) -> u32 {
        self.p
    }

    fn from_raw(raw: RawKdfParams) -> Result<Self> {
        let params = Self {
            alg: raw.alg,
            version: raw.version,
            m_kib: raw.m_kib,
            t: raw.t,
            p: raw.p,
        };
        params.validate()?;
        Ok(params)
    }

    fn validate(&self) -> Result<()> {
        self.validate_with(KdfFloor::active())
    }

    /// Validates structure, upper bounds and the given floor.
    ///
    /// # Errors
    /// [`Error::UnsupportedVersion`] for an unknown Argon2 version; [`Error::WeakKdfParams`]
    /// for out-of-range or too-cheap values.
    pub fn validate_with(&self, floor: KdfFloor) -> Result<()> {
        if self.version != ARGON2_VERSION {
            return Err(Error::UnsupportedVersion(format!(
                "argon2 version {}",
                self.version
            )));
        }
        if self.p > MAX_P || self.t > MAX_T || self.m_kib > MAX_M_KIB {
            return Err(Error::WeakKdfParams("cost parameter out of range".into()));
        }
        if self.p < floor.p
            || self.t < floor.t
            || self.m_kib < floor.m_kib
            || self.m_kib < 8 * self.p
        {
            return Err(Error::WeakKdfParams(
                "cost parameters are below the minimum".into(),
            ));
        }
        Ok(())
    }

    /// Serialises to the JSON stored in `meta.kdf_params` and sent as `kdf_params`.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("KdfParams always serialises")
    }

    /// Parses and validates the stored JSON.
    ///
    /// # Errors
    /// [`Error::Format`] for malformed JSON or an unknown `alg`; [`Error::UnsupportedVersion`]
    /// for another Argon2 version; [`Error::WeakKdfParams`] for values out of range or weaker
    /// than the floor (the downgrade a hostile server might attempt).
    pub fn from_json(s: &str) -> Result<Self> {
        let raw: RawKdfParams =
            serde_json::from_str(s).map_err(|e| Error::Format(format!("kdf_params: {e}")))?;
        Self::from_raw(raw)
    }
}

/// Derives the 32-byte master key with Argon2id.
///
/// This is deliberately expensive (≈1 s with the default parameters): call it from a blocking
/// context (`spawn_blocking`) when running inside an async runtime.
#[must_use]
pub fn derive_master_key(password: &str, salt: &[u8; SALT_LEN], params: &KdfParams) -> Key32 {
    let argon_params = Params::new(params.m_kib, params.t, params.p, Some(KEY_LEN))
        .expect("KdfParams are validated on construction");
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);
    let mut out: Key32 = Zeroizing::new([0u8; KEY_LEN]);
    argon
        .hash_password_into(password.as_bytes(), salt, &mut *out)
        .expect("fixed-size inputs are always valid for Argon2");
    out
}

/// HKDF-SHA256 with no salt, 32-byte output.
#[must_use]
pub fn hkdf_sha256(ikm: &[u8], info: &str) -> Key32 {
    let hk = Hkdf::<Sha256>::new(None, ikm);
    let mut out: Key32 = Zeroizing::new([0u8; KEY_LEN]);
    hk.expand(info.as_bytes(), &mut *out)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    out
}

/// The two keys derived from the master key.
pub struct MasterKeys {
    /// Wraps the vault key; stays on the client.
    pub enc_key: Key32,
    /// Sent to the Worker to log in (the server stores only its SHA-256).
    pub auth_key: Key32,
}

impl MasterKeys {
    /// Expands a master key into `enc_key` and `auth_key`.
    #[must_use]
    pub fn derive(master_key: &[u8; KEY_LEN]) -> Self {
        Self {
            enc_key: hkdf_sha256(master_key, "hatoba/enc/v1"),
            auth_key: hkdf_sha256(master_key, "hatoba/auth/v1"),
        }
    }

    /// Argon2id + HKDF in one step.
    #[must_use]
    pub fn from_password(password: &str, salt: &[u8; SALT_LEN], params: &KdfParams) -> Self {
        let master = derive_master_key(password, salt, params);
        Self::derive(&master)
    }
}

impl std::fmt::Debug for MasterKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKeys(<redacted>)")
    }
}

/// Decodes a base64 salt into the fixed-size array.
///
/// # Errors
/// [`Error::Format`] if it is not valid base64 of exactly 16 bytes.
pub fn decode_salt(b64: &str) -> Result<[u8; SALT_LEN]> {
    b64_decode(b64)?
        .try_into()
        .map_err(|_| Error::Format("kdf_salt must be 16 bytes".into()))
}

// ---- envelope -----------------------------------------------------------------------------

/// `{"v":1,"n":"<b64 nonce>","c":"<b64 ciphertext||tag>"}` (spec §4.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    /// Format version; always [`ENVELOPE_VERSION`].
    pub v: u8,
    /// Base64 of the 12-byte random nonce.
    pub n: String,
    /// Base64 of ciphertext followed by the 16-byte GCM tag.
    pub c: String,
}

impl Envelope {
    /// Compact JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("Envelope always serialises")
    }

    /// Parses an envelope, rejecting unknown versions.
    ///
    /// # Errors
    /// [`Error::Format`] for malformed JSON, [`Error::UnsupportedVersion`] for `v != 1`.
    pub fn from_json(s: &str) -> Result<Self> {
        let env: Self =
            serde_json::from_str(s).map_err(|_| Error::Format("malformed envelope".into()))?;
        if env.v != ENVELOPE_VERSION {
            return Err(Error::UnsupportedVersion(format!("envelope v{}", env.v)));
        }
        Ok(env)
    }
}

fn cipher(key: &[u8; KEY_LEN]) -> Aes256Gcm {
    Aes256Gcm::new_from_slice(key).expect("key is exactly 32 bytes")
}

/// Encrypts `plaintext` with a fresh random nonce, binding it to `aad`.
///
/// # Errors
/// [`Error::Random`] if no nonce can be generated; [`Error::Decrypt`] is never returned here.
pub fn seal(key: &[u8; KEY_LEN], aad: &[u8], plaintext: &[u8]) -> Result<Envelope> {
    let nonce_bytes = random_bytes::<NONCE_LEN>()?;
    let nonce = Nonce::<Aes256Gcm>::from(nonce_bytes);
    let mut buf = Zeroizing::new(Vec::with_capacity(plaintext.len() + TAG_LEN));
    buf.extend_from_slice(plaintext);
    cipher(key)
        .encrypt_in_place(&nonce, aad, &mut *buf)
        .map_err(|_| Error::Format("plaintext too large".into()))?;
    Ok(Envelope {
        v: ENVELOPE_VERSION,
        n: b64_encode(&nonce_bytes),
        c: b64_encode(&buf),
    })
}

/// Authenticates and decrypts an envelope.
///
/// # Errors
/// [`Error::Decrypt`] if the key, AAD, nonce or ciphertext do not match (indistinguishable by
/// design); [`Error::Format`] / [`Error::UnsupportedVersion`] for a malformed envelope.
pub fn open(key: &[u8; KEY_LEN], aad: &[u8], envelope: &Envelope) -> Result<Zeroizing<Vec<u8>>> {
    if envelope.v != ENVELOPE_VERSION {
        return Err(Error::UnsupportedVersion(format!(
            "envelope v{}",
            envelope.v
        )));
    }
    let nonce_bytes: [u8; NONCE_LEN] = b64_decode(&envelope.n)?
        .try_into()
        .map_err(|_| Error::Format("nonce must be 12 bytes".into()))?;
    let nonce = Nonce::<Aes256Gcm>::from(nonce_bytes);
    let mut buf = Zeroizing::new(b64_decode(&envelope.c)?);
    if buf.len() < TAG_LEN {
        return Err(Error::Format("ciphertext too short".into()));
    }
    cipher(key)
        .decrypt_in_place(&nonce, aad, &mut *buf)
        .map_err(|_| Error::Decrypt)?;
    Ok(buf)
}

/// Parses `json` as an envelope and opens it.
///
/// # Errors
/// As [`Envelope::from_json`] and [`open`].
pub fn open_json(key: &[u8; KEY_LEN], aad: &[u8], json: &str) -> Result<Zeroizing<Vec<u8>>> {
    open(key, aad, &Envelope::from_json(json)?)
}

/// Seals a 32-byte key (e.g. the vault key) and returns the envelope JSON.
///
/// # Errors
/// As [`seal`].
pub fn wrap_key(wrapping_key: &[u8; KEY_LEN], aad: &str, key: &[u8; KEY_LEN]) -> Result<String> {
    Ok(seal(wrapping_key, aad.as_bytes(), key)?.to_json())
}

/// Opens a wrapped 32-byte key.
///
/// # Errors
/// [`Error::Decrypt`] if the wrapping key is wrong; [`Error::Format`] if the plaintext is not
/// exactly 32 bytes.
pub fn unwrap_key(wrapping_key: &[u8; KEY_LEN], aad: &str, json: &str) -> Result<Key32> {
    let plain = open_json(wrapping_key, aad.as_bytes(), json)?;
    let mut key: Key32 = Zeroizing::new([0u8; KEY_LEN]);
    if plain.len() != KEY_LEN {
        return Err(Error::Format("wrapped key must be 32 bytes".into()));
    }
    key.copy_from_slice(&plain);
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PASSWORD: &str = "correct horse battery staple";

    fn unhex<const N: usize>(s: &str) -> [u8; N] {
        let mut out = [0u8; N];
        assert_eq!(s.len(), N * 2);
        for (i, byte) in out.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }

    fn test_salt() -> [u8; SALT_LEN] {
        std::array::from_fn(|i| i as u8)
    }

    // Reference values below were produced with OpenSSL-backed `cryptography` (Python), an
    // implementation independent of the RustCrypto crates used here.

    #[test]
    fn argon2id_vector_small_params() {
        let key = derive_master_key(PASSWORD, &test_salt(), &KdfParams::for_tests());
        assert_eq!(
            *key,
            unhex::<32>("92dc5d67019623868bde079275e522f4b7e8213d3414ed85cbc2ac8a41117288")
        );
    }

    #[test]
    fn argon2id_vector_real_params() {
        let key = derive_master_key(PASSWORD, &test_salt(), &KdfParams::default());
        assert_eq!(
            *key,
            unhex::<32>("853b272a44db1421c02962669a55eb0994f3cab385ed1c4c79253eee19bab49e")
        );
    }

    #[test]
    fn hkdf_vectors() {
        let master = derive_master_key(PASSWORD, &test_salt(), &KdfParams::for_tests());
        let keys = MasterKeys::derive(&master);
        assert_eq!(
            *keys.enc_key,
            unhex::<32>("196039f465971771301e121b2d0863f9816aedaeba7eff2d992a905b656118f8")
        );
        assert_eq!(
            *keys.auth_key,
            unhex::<32>("4911c93eb526c915dc578ce8ac1b93f82e5e23d8f0a7d328f69bc571b330aa4a")
        );
        assert_eq!(
            auth_hash(&keys.auth_key),
            "cfa7d7cc2609ceaa17b0f5ecbcac86ce726d4a73470d449ebf15372014af0f89"
        );
    }

    #[test]
    fn recovery_hkdf_vectors() {
        let secret: [u8; 16] = std::array::from_fn(|i| i as u8);
        assert_eq!(
            *hkdf_sha256(&secret, "hatoba/recovery/v1"),
            unhex::<32>("0dca153820188c14ba27d5f245b93895304e977e0d2a883c0ccec8c35d16f53d")
        );
        assert_eq!(
            *hkdf_sha256(&secret, "hatoba/recovery-auth/v1"),
            unhex::<32>("2f339a3148c9d4fbec4874321938875d31fa98c0aed7340e83d757b7dacbcf6c")
        );
    }

    #[test]
    fn opens_hard_coded_envelope() {
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        let json = r#"{"v":1,"n":"AAECAwQFBgcICQoL","c":"PCCiYrWA4CGvKfj4xctUT+236lHSQX0MSgiBp2A2KoRFpJZixiTBgGO6ErDc"}"#;
        let aad = item_aad("0192aaaa-bbbb-7ccc-8ddd-eeeeffff0001");
        let plain = open_json(&key, aad.as_bytes(), json).unwrap();
        assert_eq!(&**plain, br#"{"type":"host","name":"prod"}"#);
        // Same envelope under another item's AAD must not open.
        let other = item_aad("0192aaaa-bbbb-7ccc-8ddd-eeeeffff0002");
        assert!(matches!(
            open_json(&key, other.as_bytes(), json),
            Err(Error::Decrypt)
        ));
    }

    #[test]
    fn seal_open_round_trip() {
        let key = random_key().unwrap();
        for len in [0usize, 1, 15, 16, 17, 1000, 70_000] {
            let msg = vec![0xA5u8; len];
            let env = seal(&key, b"aad", &msg).unwrap();
            assert_eq!(env.v, 1);
            assert_eq!(&**open(&key, b"aad", &env).unwrap(), &msg[..]);
            let reparsed = Envelope::from_json(&env.to_json()).unwrap();
            assert_eq!(reparsed, env);
        }
    }

    #[test]
    fn nonces_are_fresh() {
        let key = random_key().unwrap();
        let a = seal(&key, b"aad", b"same plaintext").unwrap();
        let b = seal(&key, b"aad", b"same plaintext").unwrap();
        assert_ne!(a.n, b.n);
        assert_ne!(a.c, b.c);
        assert_eq!(b64_decode(&a.n).unwrap().len(), NONCE_LEN);
    }

    #[test]
    fn tampering_is_detected() {
        let key = random_key().unwrap();
        let env = seal(&key, b"aad-1", b"secret").unwrap();

        // wrong AAD
        assert!(matches!(open(&key, b"aad-2", &env), Err(Error::Decrypt)));
        // wrong key
        let other = random_key().unwrap();
        assert!(matches!(open(&other, b"aad-1", &env), Err(Error::Decrypt)));
        // flipped ciphertext bit
        let mut ct = b64_decode(&env.c).unwrap();
        ct[0] ^= 1;
        let bad = Envelope {
            c: b64_encode(&ct),
            ..env.clone()
        };
        assert!(matches!(open(&key, b"aad-1", &bad), Err(Error::Decrypt)));
        // flipped tag bit
        let mut ct = b64_decode(&env.c).unwrap();
        *ct.last_mut().unwrap() ^= 0x80;
        let bad = Envelope {
            c: b64_encode(&ct),
            ..env.clone()
        };
        assert!(matches!(open(&key, b"aad-1", &bad), Err(Error::Decrypt)));
        // flipped nonce bit
        let mut n = b64_decode(&env.n).unwrap();
        n[3] ^= 1;
        let bad = Envelope {
            n: b64_encode(&n),
            ..env.clone()
        };
        assert!(matches!(open(&key, b"aad-1", &bad), Err(Error::Decrypt)));
        // truncated ciphertext
        let ct = b64_decode(&env.c).unwrap();
        let bad = Envelope {
            c: b64_encode(&ct[..ct.len() - 1]),
            ..env.clone()
        };
        assert!(matches!(open(&key, b"aad-1", &bad), Err(Error::Decrypt)));
        let bad = Envelope {
            c: b64_encode(&ct[..4]),
            ..env.clone()
        };
        assert!(matches!(open(&key, b"aad-1", &bad), Err(Error::Format(_))));
    }

    #[test]
    fn malformed_envelopes_are_rejected() {
        assert!(matches!(
            Envelope::from_json("not json"),
            Err(Error::Format(_))
        ));
        assert!(matches!(
            Envelope::from_json(r#"{"v":2,"n":"AA==","c":"AA=="}"#),
            Err(Error::UnsupportedVersion(_))
        ));
        let key = random_key().unwrap();
        let short_nonce = Envelope {
            v: 1,
            n: b64_encode(&[0u8; 8]),
            c: b64_encode(&[0u8; 32]),
        };
        assert!(matches!(
            open(&key, b"", &short_nonce),
            Err(Error::Format(_))
        ));
        let bad_b64 = Envelope {
            v: 1,
            n: "!!!".into(),
            c: "!!!".into(),
        };
        assert!(matches!(open(&key, b"", &bad_b64), Err(Error::Format(_))));
    }

    #[test]
    fn wrap_and_unwrap_key() {
        let wrapping = random_key().unwrap();
        let vault_key = random_key().unwrap();
        let json = wrap_key(&wrapping, AAD_VAULT_KEY, &vault_key).unwrap();
        assert_eq!(
            *unwrap_key(&wrapping, AAD_VAULT_KEY, &json).unwrap(),
            *vault_key
        );
        assert!(unwrap_key(&wrapping, AAD_RECOVERY_VAULT_KEY, &json).is_err());
    }

    #[test]
    fn kdf_params_json_round_trip_and_validation() {
        let params = KdfParams::default();
        assert_eq!(
            params.to_json(),
            r#"{"alg":"argon2id","version":19,"m_kib":65536,"t":3,"p":4}"#
        );
        assert_eq!(KdfParams::from_json(&params.to_json()).unwrap(), params);

        // unknown algorithm
        let err =
            KdfParams::from_json(r#"{"alg":"scrypt","version":19,"m_kib":65536,"t":3,"p":4}"#);
        assert!(matches!(err, Err(Error::Format(_))));
        // unknown version, absurd costs, structurally impossible values
        let err =
            KdfParams::from_json(r#"{"alg":"argon2id","version":16,"m_kib":65536,"t":3,"p":4}"#);
        assert!(matches!(err, Err(Error::UnsupportedVersion(_))));
        let err = KdfParams::from_json(
            r#"{"alg":"argon2id","version":19,"m_kib":999999999,"t":3,"p":4}"#,
        );
        assert!(matches!(err, Err(Error::WeakKdfParams(_))));
        assert!(
            KdfParams::from_json(r#"{"alg":"argon2id","version":19,"m_kib":65536,"t":0,"p":4}"#)
                .is_err()
        );
        assert!(
            KdfParams::from_json(r#"{"alg":"argon2id","version":19,"m_kib":4,"t":1,"p":1}"#)
                .is_err()
        );
        // serde path (used when KdfParams is nested in another document) validates too
        assert!(
            serde_json::from_str::<KdfParams>(
                r#"{"alg":"argon2id","version":19,"m_kib":4,"t":1,"p":1}"#
            )
            .is_err()
        );
        assert!(KdfParams::new(65536, 3, 4).is_ok());
    }

    #[test]
    fn production_floor_rejects_weak_parameters() {
        let floor = KdfFloor::PRODUCTION;
        // Exactly the floor, and stronger, are accepted.
        assert!(KdfParams::default().validate_with(floor).is_ok());
        assert!(
            KdfParams::new(65536, 3, 1)
                .unwrap()
                .validate_with(floor)
                .is_ok()
        );
        assert!(
            KdfParams::new(262_144, 4, 8)
                .unwrap()
                .validate_with(floor)
                .is_ok()
        );
        // Anything weaker on any axis is refused (these only exist because tests relax the floor).
        for (m, t, p) in [
            (65535, 3, 4),
            (19_456, 2, 1),
            (65536, 2, 4),
            (64, 1, 1),
            (8, 1, 1),
        ] {
            let weak = KdfParams::new(m, t, p).unwrap();
            assert!(
                matches!(weak.validate_with(floor), Err(Error::WeakKdfParams(_))),
                "({m},{t},{p}) must be below the production floor"
            );
        }
        assert!(KdfParams::for_tests().validate_with(floor).is_err());
        // The floor in force during unit tests is the relaxed one.
        assert_eq!(KdfFloor::active(), KdfFloor::RELAXED);
    }

    #[test]
    fn salt_round_trip() {
        let salt = random_salt().unwrap();
        assert_eq!(decode_salt(&b64_encode(&salt)).unwrap(), salt);
        assert!(decode_salt(&b64_encode(&[1, 2, 3])).is_err());
    }

    #[test]
    fn aad_strings_match_the_spec() {
        assert_eq!(item_aad("abc"), "hatoba/item/v1/abc");
        assert_eq!(device_aad("dev"), "hatoba/device/v1/dev");
        assert_eq!(AAD_VAULT_KEY, "hatoba/vault-key/v1");
        assert_eq!(AAD_RECOVERY_VAULT_KEY, "hatoba/recovery-vault-key/v1");
    }
}
