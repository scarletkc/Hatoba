//! Biometric unlock: Windows Hello (SEC-07) and Touch ID (SEC-10).
//!
//! One interface, two schemes. Both keep material that only this machine can use, so a copy of it
//! is worth nothing anywhere else:
//!
//! * **Windows** (SEC-07, P1): a Hello key credential (TPM-backed, `KeyCredentialManager`) signs a
//!   fixed challenge. PKCS#1 v1.5 signatures are deterministic, so the signature is a stable secret
//!   that only this user can produce after Windows Hello verification. A wrapping key derived from
//!   it (HKDF-SHA256) encrypts the vault key, and that envelope is kept in the Windows Credential
//!   Manager. This follows the approach of the KeePass Windows Hello plugin.
//! * **macOS** (SEC-10): a P-256 key in the Secure Enclave whose `SecAccessControl` requires user
//!   presence (`biometryCurrentSet`), so using the key is what raises the system's own Touch ID
//!   sheet. The key is *not* left in a keychain: its wrapped form (the `toid` attribute, the blob
//!   CryptoKit carries in `dataRepresentation`) goes into a 0600 file in the app data directory,
//!   together with the point of an ephemeral P-256 key and the vault key sealed under the wrapping
//!   key that their ECDH and HKDF make. The blob opens only on this Mac's Secure Enclave, and only
//!   after Touch ID: it carries the control the key was made with, so a key restored from it
//!   refuses to be used until the sheet is answered. `toid` is undocumented, and the restore has to
//!   hand the blob back under that same attribute, or the import quietly makes a new key. A
//!   keychain item with a biometric access control would be the stronger design, and is the plan
//!   once the build is signed with a Developer ID: an ad-hoc signed build has no keychain access
//!   group, so `SecItemAdd` with an access control answers `errSecMissingEntitlement` (issue #145).
//!
//! The frontend is the same on both: `vault_unlock_biometric`, and the `biometric_available` /
//! `biometric_enabled` fields of the vault status.

use hatoba_core::crypto::Key32;
use hatoba_core::platform::SecretStore;
use tauri::AppHandle;

use crate::error::AppResult;

/// Whether this machine has a biometric unlock at all (Windows Hello set up, or Touch ID with a
/// finger enrolled).
pub fn available() -> bool {
    imp::available()
}

/// Whether this vault can be unlocked without the master password on this machine.
pub fn enrolled(app: &AppHandle, secrets: &dyn SecretStore) -> bool {
    available() && imp::enrolled(app, secrets)
}

/// Stores what `unlock` needs. The master password is verified by the caller first.
pub async fn enroll(
    app: &AppHandle,
    secrets: &dyn SecretStore,
    vault_key: &Key32,
) -> AppResult<()> {
    imp::enroll(app, secrets, vault_key).await
}

/// Reads the vault key back, after the platform's own verification of the user.
pub async fn unlock(app: &AppHandle, secrets: &dyn SecretStore) -> AppResult<Key32> {
    imp::unlock(app, secrets).await
}

/// Forgets the enrolment (the user turned it off, or the vault key changed and this device's copy
/// is stale).
pub fn remove(app: &AppHandle, secrets: &dyn SecretStore) -> AppResult<()> {
    imp::remove(app, secrets)
}

#[cfg(target_os = "windows")]
mod imp {
    use hatoba_core::crypto::{Key32, hkdf_sha256, open_json, seal};
    use hatoba_core::platform::{SecretStore, secret_keys};
    use tauri::AppHandle;
    use zeroize::Zeroizing;

    use crate::error::{AppError, AppResult, ErrorCode};

    const AAD: &[u8] = b"hatoba/hello-vault-key/v1";
    const HKDF_INFO: &str = "hatoba/hello-wrap/v1";

    pub fn available() -> bool {
        credential::available()
    }

    pub fn enrolled(_app: &AppHandle, secrets: &dyn SecretStore) -> bool {
        secrets
            .get(secret_keys::BIOMETRIC_VAULT_KEY)
            .ok()
            .flatten()
            .is_some()
    }

    pub fn remove(_app: &AppHandle, secrets: &dyn SecretStore) -> AppResult<()> {
        secrets.delete(secret_keys::BIOMETRIC_VAULT_KEY)?;
        credential::delete_credential();
        Ok(())
    }

    pub async fn enroll(
        app: &AppHandle,
        secrets: &dyn SecretStore,
        vault_key: &Key32,
    ) -> AppResult<()> {
        let signature = credential::sign(app, true).await?;
        let wrap = hkdf_sha256(&signature, HKDF_INFO);
        let envelope = seal(&wrap, AAD, vault_key.as_slice())?;
        secrets.set(secret_keys::BIOMETRIC_VAULT_KEY, &envelope.to_json())?;
        Ok(())
    }

    pub async fn unlock(app: &AppHandle, secrets: &dyn SecretStore) -> AppResult<Key32> {
        let sealed = secrets
            .get(secret_keys::BIOMETRIC_VAULT_KEY)?
            .ok_or_else(|| {
                AppError::new(
                    ErrorCode::NotFound,
                    "Windows Hello is not set up for this vault",
                )
            })?;
        let signature = credential::sign(app, false).await?;
        let wrap = hkdf_sha256(&signature, HKDF_INFO);
        let plain = open_json(&wrap, AAD, &sealed).map_err(|_| {
            AppError::new(
                ErrorCode::BiometricStale,
                "the saved Windows Hello credential no longer opens this vault; unlock with the \
                 master password and turn it on again",
            )
        })?;
        let bytes: [u8; 32] = plain
            .as_slice()
            .try_into()
            .map_err(|_| AppError::internal("bad biometric envelope"))?;
        Ok(Zeroizing::new(bytes))
    }

    /// The Windows Hello key credential: available, created, signed, deleted.
    mod credential {
        use tauri::AppHandle;
        use windows::Security::Credentials::{
            KeyCredentialCreationOption, KeyCredentialManager, KeyCredentialStatus,
        };
        use windows::Security::Cryptography::CryptographicBuffer;
        use windows::core::{Array, HSTRING};
        use zeroize::Zeroizing;

        use crate::error::{AppError, AppResult, ErrorCode};

        const CREDENTIAL: &str = "app.hatoba.desktop.vault";
        /// Fixed challenge; secrecy comes from the TPM-held private key, not from this value.
        const CHALLENGE: &[u8] = b"hatoba/windows-hello/challenge/v1";

        pub fn available() -> bool {
            KeyCredentialManager::IsSupportedAsync()
                .and_then(|op| op.join())
                .unwrap_or(false)
        }

        pub fn delete_credential() {
            if let Ok(op) = KeyCredentialManager::DeleteAsync(&HSTRING::from(CREDENTIAL)) {
                let _ = op.join();
            }
        }

        fn status_error(status: KeyCredentialStatus) -> AppError {
            if status == KeyCredentialStatus::UserCanceled {
                AppError::new(ErrorCode::Cancelled, "Windows Hello was cancelled")
            } else {
                AppError::new(
                    ErrorCode::Internal,
                    format!("Windows Hello failed ({status:?})"),
                )
            }
        }

        pub async fn sign(app: &AppHandle, create: bool) -> AppResult<Zeroizing<Vec<u8>>> {
            // Hello's dialog must belong to our foreground window; make sure it is in front.
            if let Some(w) = tauri::Manager::get_webview_window(app, crate::platform::window::MAIN)
            {
                let _ = w.set_focus();
            }
            tauri::async_runtime::spawn_blocking(move || {
                let name = HSTRING::from(CREDENTIAL);
                let win =
                    |e: windows::core::Error| AppError::internal(format!("Windows Hello: {e}"));
                let result = if create {
                    KeyCredentialManager::RequestCreateAsync(
                        &name,
                        KeyCredentialCreationOption::ReplaceExisting,
                    )
                } else {
                    KeyCredentialManager::OpenAsync(&name)
                }
                .and_then(|op| op.join())
                .map_err(win)?;
                let status = result.Status().map_err(win)?;
                if status != KeyCredentialStatus::Success {
                    return Err(status_error(status));
                }
                let credential = result.Credential().map_err(win)?;
                let challenge = CryptographicBuffer::CreateFromByteArray(CHALLENGE).map_err(win)?;
                let signed = credential
                    .RequestSignAsync(&challenge)
                    .and_then(|op| op.join())
                    .map_err(win)?;
                let status = signed.Status().map_err(win)?;
                if status != KeyCredentialStatus::Success {
                    return Err(status_error(status));
                }
                let buffer = signed.Result().map_err(win)?;
                let mut bytes = Array::<u8>::new();
                CryptographicBuffer::CopyToByteArray(&buffer, &mut bytes).map_err(win)?;
                Ok(Zeroizing::new(bytes.to_vec()))
            })
            .await
            .map_err(|e| AppError::internal(e.to_string()))?
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::fs;
    use std::os::raw::c_void;
    use std::os::unix::fs::OpenOptionsExt;
    use std::path::{Path, PathBuf};

    use std::io::Write as _;

    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as B64;
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::data::CFData;
    use core_foundation::dictionary::CFMutableDictionary;
    use core_foundation::string::CFString;
    use core_foundation_sys::base::CFTypeRef;
    use core_foundation_sys::dictionary::CFDictionaryGetValue;
    use core_foundation_sys::error::CFErrorRef;
    use core_foundation_sys::string::CFStringRef;
    use hatoba_core::crypto::{Envelope, Key32, hkdf_sha256, open_json, seal};
    use hatoba_core::platform::SecretStore;
    use objc2_local_authentication::{LABiometryType, LAContext, LAPolicy};
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::key::{Algorithm, GenerateKeyOptions, KeyType, SecKey, Token};
    use security_framework::passwords::AccessControlOptions;
    use security_framework_sys::item::{
        kSecAttrAccessControl, kSecAttrKeyClass, kSecAttrKeyClassPrivate, kSecAttrKeyClassPublic,
        kSecAttrKeyType, kSecAttrKeyTypeECSECPrimeRandom, kSecAttrTokenID,
        kSecAttrTokenIDSecureEnclave,
    };
    use security_framework_sys::key::SecKeyCreateWithData;
    use tauri::AppHandle;
    use zeroize::Zeroizing;

    use crate::error::{AppError, AppResult, ErrorCode};

    /// The record: beside the vault database, mode 0600.
    const RECORD: &str = "touch-id.json";
    const RECORD_VERSION: u8 = 1;
    const AAD: &[u8] = b"hatoba/touch-id-vault-key/v1";
    const HKDF_INFO: &str = "hatoba/touch-id-wrap/v1";
    /// The attribute a Secure Enclave key's wrapped form comes in. Undocumented, and the same blob
    /// CryptoKit puts in `SecureEnclave.P256.KeyAgreement.PrivateKey.dataRepresentation`.
    const BLOB_ATTRIBUTE: &str = "toid";
    /// An uncompressed P-256 point.
    const POINT_LEN: usize = 65;
    /// `errSecUserCanceled`: the sheet was dismissed.
    const ERR_USER_CANCELED: isize = -128;
    /// `errSecAuthFailed` and `errSecInteractionNotAllowed`: the user was not confirmed.
    const ERR_AUTH_FAILED: isize = -25293;
    const ERR_INTERACTION_NOT_ALLOWED: isize = -25308;
    /// `LAError.systemCancel`: the system refused to ask at all. A Mac in clamshell mode answers
    /// exactly this (`Touch ID is not available in closed clamshell mode`), and so does a sheet the
    /// system took away.
    const ERR_SYSTEM_CANCEL: isize = -4;
    /// `LAError.userCancel`: the user dismissed the sheet.
    const ERR_USER_CANCEL: isize = -2;
    /// `LAError.biometryNotAvailable` and `biometryNotEnrolled`: no sensor to read, or no finger on
    /// it.
    const ERR_BIOMETRY_NOT_AVAILABLE: isize = -6;
    const ERR_BIOMETRY_NOT_ENROLLED: isize = -7;
    /// Too many wrong fingers; LocalAuthentication's own code.
    const ERR_BIOMETRY_LOCKOUT: isize = -8;
    /// The process cannot show an authentication UI at all.
    const ERR_NOT_INTERACTIVE: isize = -1004;
    /// LocalAuthentication's error domain. Its numbers are not OSStatus ones.
    const LA_DOMAIN: &str = "com.apple.LocalAuthentication";
    /// The domain the Security framework reports its OSStatus values in.
    const OS_DOMAIN: &str = "NSOSStatusErrorDomain";
    /// The domain the Secure Enclave reports through. Its numbers are not OSStatus ones either —
    /// `-3` here is not `errSecNoSuchAttr`, and cannot be read from another domain's table.
    const CTK_DOMAIN: &str = "CryptoTokenKit";
    /// `CryptoTokenKit`: the key was made with `biometryCurrentSet` and the enrolled fingerprints
    /// have changed since, so the enclave will not do a private-key operation with it again
    /// (measured: `unable to compute shared secret`, with the enclave's own `AKSError=-536362999`,
    /// after one fingerprint was added to this Mac).
    const ERR_KEY_INVALIDATED: isize = -3;

    /// Whether this Mac can do it at all: Touch ID is there, so a Mac without one hides the switch,
    /// as a machine without Windows Hello does on Windows.
    ///
    /// The answer comes from the biometric type, not from `canEvaluatePolicy`, which answers
    /// "Authentication canceled" on a Mac where Touch ID works and a finger is enrolled: evaluating
    /// that policy asks for the prompt there and then, which a call that only wants to know what
    /// the hardware is cannot satisfy. Whether a finger is really enrolled is settled where it
    /// matters instead, by the key's own control at enrolment.
    pub fn available() -> bool {
        let context = unsafe { LAContext::new() };
        // SAFETY: only the policy result and the biometric type of a context made here are read.
        unsafe {
            // Documented as what fills in `biometryType`; its answer is not usable here and is
            // logged for the next machine that disagrees.
            if let Err(error) =
                context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics)
            {
                tracing::debug!("the biometrics policy cannot be evaluated here: {error}");
            }
            context.biometryType() != LABiometryType::None
        }
    }

    /// Whether this device holds a record. The file is the enrolment, so this asks nothing of the
    /// Secure Enclave and shows no sheet.
    pub fn enrolled(app: &AppHandle, _secrets: &dyn SecretStore) -> bool {
        record_path(app).is_ok_and(|path| path.is_file())
    }

    pub fn remove(app: &AppHandle, _secrets: &dyn SecretStore) -> AppResult<()> {
        let path = record_path(app)?;
        match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(AppError::io(e)),
        }
    }

    pub async fn enroll(
        app: &AppHandle,
        _secrets: &dyn SecretStore,
        vault_key: &Key32,
    ) -> AppResult<()> {
        let path = record_path(app)?;
        let vault = Zeroizing::new(vault_key.to_vec());
        // The Secure Enclave work blocks, and it is what shows the sheet: keep it off the async
        // runtime, as the Hello path does.
        let record = tauri::async_runtime::spawn_blocking(move || {
            let enclave = enclave_key()?;
            let ephemeral = ephemeral_key()?;
            let peer = public_key(&ephemeral)?;
            let point = point_of(&peer)?;
            let wrap = wrap_key(&enclave, &peer)?;
            let envelope: Envelope = seal(&wrap, AAD, vault.as_slice())?;
            let blob = key_blob(&enclave)?;
            // The record has to hold this exact key before it is kept, and proving that must cost
            // no second sheet: creating the key already asked for one, and a person who turns the
            // switch on sees exactly one. So the blob comes back without the control — attaching
            // it is what asks — and comparing public points says the same key came back. The
            // record's own restore at unlock passes the control, as `key_from_blob` does.
            let private = unsafe { kSecAttrKeyClassPrivate };
            let restored = import(&blob, &import_attributes(private, Some(&blob), None))?;
            if point_of(&public_key(&enclave)?)? != point_of(&public_key(&restored)?)? {
                return Err(AppError::internal(
                    "the Secure Enclave key did not survive its own blob",
                ));
            }
            let record = Record {
                version: RECORD_VERSION,
                key_blob: B64.encode(blob),
                ephemeral_public: B64.encode(&point),
                envelope: envelope.to_json(),
            };
            Ok::<Record, AppError>(record)
        })
        .await
        .map_err(|e| AppError::internal(e.to_string()))??;
        write_record(&path, &record)
    }

    pub async fn unlock(app: &AppHandle, _secrets: &dyn SecretStore) -> AppResult<Key32> {
        let path = record_path(app)?;
        let record = read_record(&path)?;
        tauri::async_runtime::spawn_blocking(move || record.open())
            .await
            .map_err(|e| AppError::internal(e.to_string()))?
    }

    /// The record: the wrapped Secure Enclave key, the point of the ephemeral key it was exchanged
    /// with, and the vault key sealed under the wrapping key they make.
    #[derive(Debug, serde::Deserialize, serde::Serialize)]
    struct Record {
        version: u8,
        /// The `toid` blob of the Secure Enclave key, base64.
        key_blob: String,
        /// The uncompressed point of the ephemeral key, base64.
        ephemeral_public: String,
        /// The vault key, sealed under the HKDF of the ECDH between the two, as JSON.
        envelope: String,
    }

    impl Record {
        /// The vault key this record holds. The ECDH is what asks for Touch ID.
        fn open(&self) -> AppResult<Key32> {
            let blob = self.field(&self.key_blob, "key blob")?;
            let point = self.field(&self.ephemeral_public, "ephemeral key")?;
            let enclave = key_from_blob(&blob)?;
            let peer = public_key_from_point(&point)?;
            let wrap = wrap_key(&enclave, &peer)?;
            let plain = open_json(&wrap, AAD, &self.envelope).map_err(|_| {
                AppError::new(
                    ErrorCode::BiometricStale,
                    "the saved Touch ID record no longer opens this vault; unlock with the master \
                     password and turn Touch ID on again",
                )
            })?;
            let bytes: [u8; 32] = plain
                .as_slice()
                .try_into()
                .map_err(|_| AppError::internal("bad Touch ID envelope"))?;
            Ok(Zeroizing::new(bytes))
        }

        /// One base64 field, decoded.
        fn field(&self, encoded: &str, what: &str) -> AppResult<Zeroizing<Vec<u8>>> {
            B64.decode(encoded).map(Zeroizing::new).map_err(|_| {
                AppError::internal(format!("the Touch ID record's {what} is not base64"))
            })
        }
    }

    /// A Secure Enclave P-256 key that no keychain holds, which only Touch ID may use.
    fn enclave_key() -> AppResult<SecKey> {
        let mut options = GenerateKeyOptions::default();
        options
            .set_key_type(KeyType::ec())
            .set_size_in_bits(256)
            .set_token(Token::SecureEnclave)
            .set_access_control(control()?);
        // No `set_location`: `kSecAttrIsPermanent` stays false, so no keychain gets the key.
        SecKey::new(&options)
            .map_err(|e| AppError::internal(format!("could not make the Secure Enclave key: {e}")))
    }

    /// An ephemeral software key of the same curve: the half that goes into the record.
    fn ephemeral_key() -> AppResult<SecKey> {
        let mut options = GenerateKeyOptions::default();
        options.set_key_type(KeyType::ec()).set_size_in_bits(256);
        SecKey::new(&options)
            .map_err(|e| AppError::internal(format!("could not make the ephemeral key: {e}")))
    }

    /// What the key is made with, and what a restored key is restored with: Touch ID, and the key
    /// only for key agreement. The blob carries the key, not the rule for using it, so the restore
    /// passes this too — without it the restored key would do the exchange with nobody present.
    fn control() -> AppResult<SecAccessControl> {
        let flags = AccessControlOptions::PRIVATE_KEY_USAGE.bits()
            | AccessControlOptions::BIOMETRY_CURRENT_SET.bits();
        SecAccessControl::create_with_protection(
            Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
            flags,
        )
        .map_err(|e| AppError::internal(format!("keychain access control: {e}")))
    }

    /// The wrapped form of a key: what the record keeps, and all that survives the process.
    fn key_blob(key: &SecKey) -> AppResult<Vec<u8>> {
        attribute(key, BLOB_ATTRIBUTE)?.ok_or_else(|| {
            AppError::internal(
                "the Secure Enclave key carries no blob, so Touch ID cannot be set up",
            )
        })
    }

    /// One attribute of a key, when it holds data.
    fn attribute(key: &SecKey, name: &str) -> AppResult<Option<Vec<u8>>> {
        let attributes = key.attributes();
        let name = CFString::new(name);
        // SAFETY: the dictionary is the key's own attributes and the name outlives the call.
        let found = unsafe {
            CFDictionaryGetValue(
                attributes.as_concrete_TypeRef(),
                name.as_concrete_TypeRef() as *const c_void,
            )
        };
        if found.is_null() {
            return Ok(None);
        }
        let value = unsafe { CFType::wrap_under_get_rule(found) };
        Ok(value.downcast::<CFData>().map(|data| data.bytes().to_vec()))
    }

    /// The key a blob holds. The import needs `kSecAttrTokenIDSecureEnclave`, the blob itself, and
    /// the control: the blob holds the key, not the rule for using it. Measured on macOS 27 with an
    /// ad-hoc signed binary, a key imported without the control answers the exchange with no sheet
    /// and no fingerprint at all, so the control is what keeps the restored key's requirement —
    /// `the_restored_key_still_needs_touch_id` fails if it is dropped again.
    fn key_from_blob(blob: &[u8]) -> AppResult<SecKey> {
        let private = unsafe { kSecAttrKeyClassPrivate };
        import(
            blob,
            &import_attributes(private, Some(blob), Some(control()?)),
        )
    }

    /// A public key from the uncompressed point the record keeps.
    fn public_key_from_point(point: &[u8]) -> AppResult<SecKey> {
        let public = unsafe { kSecAttrKeyClassPublic };
        import(point, &import_attributes(public, None, None))
    }

    /// The attributes `SecKeyCreateWithData` reads for a P-256 key of the record's shape.
    ///
    /// A private key also needs its own wrapped form back, under the same undocumented attribute it
    /// came from: without it the import quietly makes a **new** Secure Enclave key instead of the
    /// one that sealed the record, and every unlock would then fail on the envelope.
    fn import_attributes(
        class: CFStringRef,
        blob: Option<&[u8]>,
        control: Option<SecAccessControl>,
    ) -> CFMutableDictionary<CFType, CFType> {
        let mut attributes = CFMutableDictionary::from_CFType_pairs(&[]);
        // SAFETY: every constant is a framework-owned CFString, and the dictionary takes its own
        // references to them and to the values.
        unsafe {
            attributes.add(
                &wrap(kSecAttrKeyType),
                &wrap(kSecAttrKeyTypeECSECPrimeRandom),
            );
            attributes.add(&wrap(kSecAttrKeyClass), &wrap(class));
            if let Some(blob) = blob {
                attributes.add(&wrap(kSecAttrTokenID), &wrap(kSecAttrTokenIDSecureEnclave));
                attributes.add(
                    &CFString::new(BLOB_ATTRIBUTE).into_CFType(),
                    &CFData::from_buffer(blob).into_CFType(),
                );
            }
            if let Some(control) = &control {
                attributes.add(&wrap(kSecAttrAccessControl), &control.as_CFType());
            }
        }
        attributes
    }

    /// The key a data blob describes, as the framework's own type.
    fn import(data: &[u8], attributes: &CFMutableDictionary<CFType, CFType>) -> AppResult<SecKey> {
        let data = CFData::from_buffer(data);
        let mut error: CFErrorRef = std::ptr::null_mut();
        // SAFETY: both arguments live for the call, and the key it returns is handed to us.
        let key = unsafe {
            SecKeyCreateWithData(
                data.as_concrete_TypeRef(),
                attributes.as_concrete_TypeRef(),
                &mut error,
            )
        };
        if key.is_null() {
            let reason = if error.is_null() {
                "no reason given".to_owned()
            } else {
                // SAFETY: the framework filled the error for us to own.
                let error = unsafe { CFType::wrap_under_create_rule(error as CFTypeRef) };
                format!("{error:?}")
            };
            // A migrated record is the common case: the Secure Enclave that made the key is on
            // the other Mac. The framework's `reason` comes in the system's language, so it is
            // logged rather than handed to the user.
            tracing::warn!("the Touch ID key did not import: {reason}");
            return Err(AppError::new(
                ErrorCode::BiometricStale,
                "the saved Touch ID key cannot be used on this Mac; unlock with the master password \
                 and turn Touch ID on again",
            ));
        }
        Ok(unsafe { SecKey::wrap_under_create_rule(key) })
    }

    /// The public half of a key.
    fn public_key(key: &SecKey) -> AppResult<SecKey> {
        key.public_key()
            .ok_or_else(|| AppError::internal("the ephemeral key has no public half"))
    }

    /// The uncompressed point of a public key, which is all the record needs of it.
    fn point_of(key: &SecKey) -> AppResult<Vec<u8>> {
        key.external_representation()
            .map(|data| data.bytes().to_vec())
            .filter(|point| point.len() == POINT_LEN)
            .ok_or_else(|| AppError::internal("the ephemeral key has no point"))
    }

    /// The wrapping key: the ECDH of the Secure Enclave key and its peer, then HKDF-SHA256. This is
    /// the call that asks for Touch ID.
    fn wrap_key(enclave: &SecKey, peer_public: &SecKey) -> AppResult<Key32> {
        let shared = enclave
            .key_exchange(Algorithm::ECDHKeyExchangeStandard, peer_public, 32, None)
            .map_err(|error| exchange_error(&error))?;
        let shared = Zeroizing::new(shared);
        Ok(hkdf_sha256(&shared, HKDF_INFO))
    }

    /// What a refused exchange means to the user.
    ///
    /// The same small negative numbers mean different things in different domains: `-4` is
    /// `LAError.systemCancel` to LocalAuthentication (measured: the answer with the lid shut,
    /// whose text reads `Touch ID is not available in closed clamshell mode`) and
    /// `errSecUnimplemented` to the Security framework (measured:
    /// `SecKeyCopyExternalRepresentation` on a Secure Enclave key). A code alone would read
    /// either as the other, so every case matches on the domain *and* the code — and the
    /// framework's own text, which comes in the system's language, goes to the log rather than
    /// into `detail`, which stays short technical English (`error.rs`).
    fn exchange_error(error: &core_foundation::error::CFError) -> AppError {
        let domain = error.domain().to_string();
        let code = error.code();
        match (domain.as_str(), code) {
            (LA_DOMAIN, ERR_USER_CANCEL | ERR_USER_CANCELED) => {
                AppError::new(ErrorCode::Cancelled, "Touch ID was cancelled")
            }
            // Too many wrong fingers (`biometryLockout`), no finger enrolled, the sensor gone, the
            // lid shut, or a desktop Mac whose Touch ID keyboard is away. Every one of these means
            // "not now, use the password" — and none of them is the master password being wrong.
            (
                LA_DOMAIN,
                ERR_SYSTEM_CANCEL
                | ERR_BIOMETRY_NOT_AVAILABLE
                | ERR_BIOMETRY_NOT_ENROLLED
                | ERR_BIOMETRY_LOCKOUT
                | ERR_NOT_INTERACTIVE,
            ) => unavailable(error),
            // The Secure Enclave's own answer when it cannot reach the biometric now — including
            // the lockout the store reports instead of LocalAuthentication's own code.
            (OS_DOMAIN, ERR_INTERACTION_NOT_ALLOWED | ERR_AUTH_FAILED) => unavailable(error),
            // The fingerprints changed after the key was made: the record is not wrong, it is
            // finished — no retry brings it back, and the feature has to be turned on again.
            (CTK_DOMAIN, ERR_KEY_INVALIDATED) => stale(error),
            (_, _) => AppError::new(
                ErrorCode::Internal,
                format!("Touch ID could not be used ({error}, {domain} {code})"),
            ),
        }
    }

    /// A refusal that means "not now": the framework's reason goes to the log, and the user is told
    /// what they can do — which is never that their password was wrong.
    fn unavailable(error: &core_foundation::error::CFError) -> AppError {
        tracing::warn!(
            "Touch ID is not available now: {} ({error})",
            error.description()
        );
        AppError::new(
            ErrorCode::BiometricUnavailable,
            "Touch ID cannot be used right now; the master password unlocks",
        )
    }

    /// A record that cannot be used again: the framework's reason goes to the log, and the user is
    /// told what to do rather than that something went wrong.
    fn stale(error: &core_foundation::error::CFError) -> AppError {
        tracing::warn!(
            "the Touch ID key cannot be used any more: {} ({error})",
            error.description()
        );
        AppError::new(
            ErrorCode::BiometricStale,
            "the saved Touch ID record cannot be used on this Mac; unlock with the master password \
             and turn Touch ID on again",
        )
    }

    /// A keychain attribute constant as the `CFType` the dictionaries take.
    fn wrap(constant: CFStringRef) -> CFType {
        // SAFETY: the framework's own constant strings, which live as long as it does.
        unsafe { CFString::wrap_under_get_rule(constant).into_CFType() }
    }

    /// Where the record lives: in the app data directory, beside the vault database.
    fn record_path(app: &AppHandle) -> AppResult<PathBuf> {
        let dir = tauri::Manager::path(app)
            .app_local_data_dir()
            .map_err(|e| AppError::internal(format!("no app data directory: {e}")))?;
        Ok(record_file(&dir))
    }

    fn record_file(dir: &Path) -> PathBuf {
        dir.join(RECORD)
    }

    /// Writes the record, readable by this user alone, in one step: a record that is half written
    /// would end an unlock it never started.
    fn write_record(path: &Path, record: &Record) -> AppResult<()> {
        let json = serde_json::to_vec(record)
            .map_err(|e| AppError::internal(format!("Touch ID record: {e}")))?;
        let temporary = path.with_extension("json.new");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(AppError::io)?;
        file.write_all(&json).map_err(AppError::io)?;
        file.sync_all().map_err(AppError::io)?;
        drop(file);
        fs::rename(&temporary, path).map_err(AppError::io)?;
        Ok(())
    }

    fn read_record(path: &Path) -> AppResult<Record> {
        let json = match fs::read_to_string(path) {
            Ok(json) => json,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(AppError::new(
                    ErrorCode::NotFound,
                    "Touch ID is not set up for this vault",
                ));
            }
            Err(e) => return Err(AppError::io(e)),
        };
        let record: Record = serde_json::from_str(&json).map_err(|_| {
            AppError::new(
                ErrorCode::BiometricStale,
                "the saved Touch ID record is damaged; unlock with the master password and turn \
                 Touch ID on again",
            )
        })?;
        if record.version != RECORD_VERSION {
            return Err(AppError::new(
                ErrorCode::BiometricStale,
                "the saved Touch ID record was written by another version; unlock with the master \
                 password and turn Touch ID on again",
            ));
        }
        Ok(record)
    }

    #[cfg(test)]
    mod tests {
        /// A `CFError` with a domain and a code, as the frameworks hand one over.
        fn framework_error(domain: &str, code: isize) -> core_foundation::error::CFError {
            use core_foundation::base::TCFType;
            use core_foundation::string::CFString;
            use core_foundation_sys::error::CFErrorCreate;
            let domain = CFString::new(domain);
            let raw = unsafe {
                CFErrorCreate(
                    std::ptr::null(),
                    domain.as_concrete_TypeRef(),
                    code,
                    std::ptr::null(),
                )
            };
            unsafe { core_foundation::error::CFError::wrap_under_create_rule(raw) }
        }

        #[test]
        fn a_refusal_is_read_by_its_domain_and_its_code() {
            use ErrorCode::{BiometricStale, BiometricUnavailable, Cancelled, Internal};
            // What this Mac answered after a fingerprint was added: the key was invalidated.
            assert_eq!(
                exchange_error(&framework_error(CTK_DOMAIN, ERR_KEY_INVALIDATED)).code,
                BiometricStale
            );
            // A closed lid, a lockout, no finger enrolled, and a process that cannot ask at all.
            for code in [
                ERR_SYSTEM_CANCEL,
                ERR_BIOMETRY_NOT_AVAILABLE,
                ERR_BIOMETRY_NOT_ENROLLED,
                ERR_BIOMETRY_LOCKOUT,
                ERR_NOT_INTERACTIVE,
            ] {
                assert_eq!(
                    exchange_error(&framework_error(LA_DOMAIN, code)).code,
                    BiometricUnavailable,
                    "LAError {code}"
                );
            }
            assert_eq!(
                exchange_error(&framework_error(OS_DOMAIN, ERR_INTERACTION_NOT_ALLOWED)).code,
                BiometricUnavailable
            );
            // The user's own cancel, which stays quiet on the lock screen.
            assert_eq!(
                exchange_error(&framework_error(LA_DOMAIN, ERR_USER_CANCEL)).code,
                Cancelled
            );
            // The same numbers in other domains are other things: `-4` is `errSecUnimplemented`
            // to the Security framework, which is how a code-only match read a closed lid.
            assert_eq!(
                exchange_error(&framework_error(OS_DOMAIN, -4)).code,
                Internal
            );
            assert_eq!(
                exchange_error(&framework_error(CTK_DOMAIN, -1)).code,
                Internal
            );
            assert_eq!(
                exchange_error(&framework_error("Elsewhere", -3)).code,
                Internal
            );
        }

        use super::*;

        /// A directory of this test's own, gone when it ends; the crate's other tests make theirs
        /// the same way, under the system temporary directory.
        struct Scratch(PathBuf);

        impl Scratch {
            fn new(name: &str) -> Self {
                let dir = std::env::temp_dir()
                    .join(format!("hatoba-touch-id-{name}-{}", std::process::id()));
                let _ = fs::remove_dir_all(&dir);
                fs::create_dir_all(&dir).unwrap();
                Self(dir)
            }

            fn record(&self) -> PathBuf {
                record_file(&self.0)
            }
        }

        impl Drop for Scratch {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        /// A record whose fields are placeholders: nothing here touches the Secure Enclave.
        fn sample() -> Record {
            let key = Zeroizing::new([0u8; 32]);
            let envelope: Envelope = seal(&key, AAD, b"sample").expect("seal");
            Record {
                version: RECORD_VERSION,
                key_blob: B64.encode([1u8; 569]),
                ephemeral_public: B64.encode([2u8; POINT_LEN]),
                envelope: envelope.to_json(),
            }
        }

        #[test]
        fn the_record_round_trips_through_its_file() {
            let scratch = Scratch::new("round-trip");
            let path = scratch.record();
            write_record(&path, &sample()).unwrap();
            let read = read_record(&path).unwrap();
            assert_eq!(read.version, RECORD_VERSION);
            assert_eq!(read.key_blob, sample().key_blob);
            assert_eq!(read.ephemeral_public, sample().ephemeral_public);
        }

        #[test]
        fn the_record_is_readable_by_this_user_alone() {
            use std::os::unix::fs::PermissionsExt as _;
            let scratch = Scratch::new("mode");
            let path = scratch.record();
            write_record(&path, &sample()).unwrap();
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "the record is not private");
        }

        #[test]
        fn a_missing_record_says_it_is_not_set_up() {
            let scratch = Scratch::new("missing");
            let err = read_record(&scratch.record()).unwrap_err();
            assert_eq!(err.code, ErrorCode::NotFound);
        }

        #[test]
        fn a_record_that_cannot_be_opened_asks_to_be_turned_on_again() {
            let scratch = Scratch::new("damaged");
            let path = scratch.record();
            for damaged in ["", "not json at all", r#"{"version": 1}"#] {
                fs::write(&path, damaged).unwrap();
                let err = read_record(&path).unwrap_err();
                assert_eq!(err.code, ErrorCode::BiometricStale, "{damaged}");
            }
        }

        #[test]
        fn a_record_from_another_version_is_refused() {
            let scratch = Scratch::new("version");
            let path = scratch.record();
            let mut record = sample();
            record.version = RECORD_VERSION + 1;
            fs::write(&path, serde_json::to_vec(&record).unwrap()).unwrap();
            assert_eq!(
                read_record(&path).unwrap_err().code,
                ErrorCode::BiometricStale
            );
        }

        #[test]
        fn a_damaged_field_does_not_reach_the_secure_enclave() {
            let mut record = sample();
            record.key_blob = "not base64!".to_owned();
            let err = record.open().unwrap_err();
            assert_eq!(err.code, ErrorCode::Internal);
            assert!(err.detail.contains("key blob"), "{}", err.detail);
        }

        /// The point the record keeps has to come back as the same key: this is the pair of
        /// software keys, so it runs anywhere, without Touch ID and without a Secure Enclave.
        #[test]
        fn an_uncompressed_point_imports_as_the_same_public_key() {
            let one = ephemeral_key().unwrap();
            let two = ephemeral_key().unwrap();
            let one_point = point_of(&public_key(&one).unwrap()).unwrap();
            let two_point = point_of(&public_key(&two).unwrap()).unwrap();

            let imported = public_key_from_point(&one_point).unwrap();
            let straight = two
                .key_exchange(
                    Algorithm::ECDHKeyExchangeStandard,
                    &public_key(&one).unwrap(),
                    32,
                    None,
                )
                .unwrap();
            let imported_point = imported.external_representation().unwrap();
            assert_eq!(imported_point.bytes(), one_point);
            let the_same = two
                .key_exchange(Algorithm::ECDHKeyExchangeStandard, &imported, 32, None)
                .unwrap();
            assert_eq!(straight, the_same);
            assert_eq!(two_point.len(), POINT_LEN);
        }

        /// The identity the record rests on, on a machine with a Secure Enclave: a key restored
        /// from the blob is the same key, so the file brings back what it sealed. Gated, because
        /// it needs real hardware; it costs no sheet, since comparing points cannot ask for one.
        ///
        /// `HATOBA_TOUCH_ID_IT=1 cargo test -p hatoba-desktop the_enclave_blob`
        #[test]
        fn the_enclave_blob_restores_the_same_key() {
            if std::env::var("HATOBA_TOUCH_ID_IT").is_err() {
                eprintln!("skipped: set HATOBA_TOUCH_ID_IT=1 on a Mac with a Secure Enclave");
                return;
            }
            let enclave = enclave_key().expect("secure enclave key");
            let blob = key_blob(&enclave).expect("the key carries a blob");
            assert!(
                blob.len() > 100,
                "the blob looks wrong: {} bytes",
                blob.len()
            );

            // The blob, and only the blob, is what a restart has.
            let restored = key_from_blob(&blob).expect("the blob restores a key");
            assert_eq!(
                point_of(&public_key(&enclave).unwrap()).unwrap(),
                point_of(&public_key(&restored).unwrap()).unwrap(),
                "the restored key is not the key that was made"
            );
        }

        /// The baseline for the two below: a key that was *made* here, whose control the Secure
        /// Enclave attached when it created it. If this one completes too, then no Secure Enclave
        /// key asks for Touch ID in this context and the difference is not the restore at all.
        ///
        /// `HATOBA_TOUCH_ID_IT=1 cargo test -p hatoba-desktop`
        /// `the_key_that_was_made_here_needs_touch_id`
        #[test]
        fn the_key_that_was_made_here_needs_touch_id() {
            if std::env::var("HATOBA_TOUCH_ID_IT").is_err() {
                eprintln!("skipped: set HATOBA_TOUCH_ID_IT=1 on a Mac with a Secure Enclave");
                return;
            }
            let enclave = enclave_key().expect("secure enclave key");
            let peer = ephemeral_key().unwrap();
            let peer_public = public_key(&peer).unwrap();
            let (answered, waited) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = answered.send(wrap_key(&enclave, &peer_public));
            });
            match waited.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(Ok(_)) => panic!(
                    "a key made here did the exchange with nobody present: the control is not \
                     enforced in this context at all, so the restore cannot be blamed"
                ),
                Ok(Err(error)) => eprintln!("refused, as it must be: {}", error.detail),
                Err(_) => eprintln!("still waiting for a finger after 5s, as it must be"),
            }
        }

        /// The claim the whole design rests on, on a machine with a Secure Enclave: the restored
        /// key asks for Touch ID. Nobody is at the keyboard in a test, so the exchange must *not*
        /// complete — an `Ok` here means it went through with no fingerprint, which is exactly
        /// what the restore did while it imported the blob without the control. The exchange
        /// blocks while the sheet waits, so it runs in a thread with a few seconds on the clock;
        /// the sheet it may leave on screen is the system's to dismiss.
        ///
        /// `HATOBA_TOUCH_ID_IT=1 cargo test -p hatoba-desktop`
        /// `the_restored_key_still_needs_touch_id`
        #[test]
        fn the_restored_key_still_needs_touch_id() {
            if std::env::var("HATOBA_TOUCH_ID_IT").is_err() {
                eprintln!("skipped: set HATOBA_TOUCH_ID_IT=1 on a Mac with a Secure Enclave");
                return;
            }
            let enclave = enclave_key().expect("secure enclave key");
            let blob = key_blob(&enclave).expect("the key carries a blob");
            let restored = key_from_blob(&blob).expect("the blob restores a key");
            let peer = ephemeral_key().unwrap();
            let peer_public = public_key(&peer).unwrap();

            let (answered, waited) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = answered.send(wrap_key(&restored, &peer_public));
            });
            match waited.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(Ok(_)) => panic!(
                    "the restored key did the exchange without Touch ID: the control is not \
                     being passed, so anyone with the record has the vault key"
                ),
                Ok(Err(error)) => eprintln!("refused, as it must be: {}", error.detail),
                Err(_) => eprintln!("still waiting for a finger after 5s, as it must be"),
            }
        }

        /// Why the restore passes the control at all. Measured on macOS 27 with an ad-hoc signed
        /// binary: a key imported from this very blob *without* the control answers the exchange
        /// with no sheet and no fingerprint. `key_from_blob` exists to prevent that; this records
        /// the behaviour it prevents, so a platform change is visible rather than silent.
        ///
        /// `HATOBA_TOUCH_ID_IT=1 cargo test -p hatoba-desktop`
        /// `the_control_is_what_asks_for_touch_id`
        #[test]
        fn the_control_is_what_asks_for_touch_id() {
            if std::env::var("HATOBA_TOUCH_ID_IT").is_err() {
                eprintln!("skipped: set HATOBA_TOUCH_ID_IT=1 on a Mac with a Secure Enclave");
                return;
            }
            let enclave = enclave_key().expect("secure enclave key");
            let blob = key_blob(&enclave).expect("the key carries a blob");
            let without = import(
                &blob,
                &import_attributes(unsafe { kSecAttrKeyClassPrivate }, Some(&blob), None),
            )
            .expect("the blob restores a key");
            // Same key, by its public point — no sheet is involved in saying so.
            assert_eq!(
                point_of(&public_key(&enclave).unwrap()).unwrap(),
                point_of(&public_key(&without).unwrap()).unwrap(),
                "the control-less import is still the key the blob holds"
            );
            let peer = ephemeral_key().unwrap();
            let peer_public = public_key(&peer).unwrap();
            let (answered, waited) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = answered.send(wrap_key(&without, &peer_public));
            });
            match waited.recv_timeout(std::time::Duration::from_secs(5)) {
                Ok(Ok(_)) => eprintln!(
                    "measured: a key imported without the control completes the exchange with no sheet"
                ),
                Ok(Err(error)) => eprintln!(
                    "the control-less key was refused too ({}); the control is belt and braces here",
                    error.detail
                ),
                Err(_) => eprintln!("the control-less key is waiting for a finger too"),
            }
        }
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod imp {
    use hatoba_core::crypto::Key32;
    use hatoba_core::platform::SecretStore;
    use tauri::AppHandle;

    use crate::error::{AppError, AppResult, ErrorCode};

    pub fn available() -> bool {
        false
    }

    pub fn enrolled(_app: &AppHandle, _secrets: &dyn SecretStore) -> bool {
        false
    }

    pub fn remove(_app: &AppHandle, _secrets: &dyn SecretStore) -> AppResult<()> {
        Ok(())
    }

    pub async fn enroll(
        _app: &AppHandle,
        _secrets: &dyn SecretStore,
        _vault_key: &Key32,
    ) -> AppResult<()> {
        Err(unavailable())
    }

    pub async fn unlock(_app: &AppHandle, _secrets: &dyn SecretStore) -> AppResult<Key32> {
        Err(unavailable())
    }

    fn unavailable() -> AppError {
        AppError::new(
            ErrorCode::Internal,
            "biometric unlock is not available on this platform yet",
        )
    }
}
