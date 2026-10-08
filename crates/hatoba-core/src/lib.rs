//! Crypto, vault, data model, local store and sync engine for Hatoba.
//!
//! `hatoba-core` holds everything security-critical and has no dependency on Tauri or any
//! desktop-only crate, so it can be reused on mobile (spec §3.4). OS-specific capabilities
//! (keychain, biometrics) are injected through the traits in [`platform`].
//!
//! # Map
//!
//! | Module | Role |
//! |---|---|
//! | [`crypto`] | Argon2id + HKDF key hierarchy, AES-256-GCM envelopes (spec §4.1, §4.2) |
//! | [`recovery`] | the 128-bit recovery code in Crockford Base32 |
//! | [`model`] | plaintext item structures (spec §5.1) |
//! | [`store`] | local SQLite: ciphertext only (spec §5.2) |
//! | [`vault`] | lock state, password and recovery operations, the decrypted item map |
//! | [`sync`] | backends (Worker / D1), engine, conflict rules, user-facing flows (spec §6) |
//! | [`backup`] | encrypted backup export / import (VAULT-07) |
//! | [`platform`] | injected capabilities (`SecretStore`, `DeviceInfo`) |
//! | [`version`] | release versions of the app and the sync Worker |
//!
//! # Security conventions
//!
//! * Secrets live in [`zeroize::Zeroizing`] containers and are wiped on lock (SEC-01).
//! * Nothing secret is ever logged (SEC-04): log lines carry item ids and short error codes
//!   only, never envelopes, item contents, passwords or tokens, and every type that holds a
//!   secret has a redacting `Debug`.
//! * KDF parameters are never accepted below the spec defaults, wherever they come from
//!   ([`crypto::KdfParams`]).
//!
//! # Cargo features
//!
//! * `test-util`: relaxes that KDF floor and exposes `KdfParams::for_tests()` so other crates'
//!   tests can create vaults quickly. Never enable it in a shipping build.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod backup;
pub mod crypto;
pub mod error;
pub mod model;
pub mod platform;
pub mod recovery;
pub mod store;
pub mod sync;
pub mod vault;
pub mod version;

pub use crypto::{Envelope, KdfParams, Key32};
pub use error::{Error, Result};
pub use model::{Item, new_id};
pub use platform::{DeviceInfo, SecretStore};
pub use recovery::RecoveryCode;
pub use vault::{PasswordChange, RecoveryChange, Vault, VaultStatusInfo};
