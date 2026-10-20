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
//! * **macOS** (SEC-10): a P-256 key in the Secure Enclave, made with a `biometryCurrentSet`
//!   access control and kept in no keychain. Its wrapped form (the undocumented `toid` attribute,
//!   the blob CryptoKit calls `dataRepresentation`) goes into a 0600 file in the app data
//!   directory, with the point of an ephemeral P-256 key and the vault key sealed under the HKDF of
//!   their ECDH. The blob only opens on this Mac's Secure Enclave. Restoring it with the access
//!   control is what raises the Touch ID sheet; on the ad-hoc signed build the Secure Enclave was
//!   measured not to enforce the control on the restored key itself, so another process of the
//!   same user could skip the sheet (issue #145, spec §4.4). A data protection keychain item with
//!   a biometric access control closes that, and needs a Developer ID build: without a keychain
//!   access group `SecItemAdd` answers `errSecMissingEntitlement`.
//!
//! The frontend is the same on both: `vault_unlock_biometric`, and the `biometric_available` /
//! `biometric_enabled` fields of the vault status.

use hatoba_core::crypto::Key32;
use hatoba_core::platform::SecretStore;
use tauri::AppHandle;

use crate::error::AppResult;

/// Whether this machine has a biometric unlock at all (Windows Hello set up, or a Mac with Touch
/// ID).
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

/// Forgets the enrolment (the user turned it off, the vault key changed, or it no longer opens).
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
        // The Hello credential was replaced since enrolment, so its signature no longer opens it.
        let plain = open_json(&wrap, AAD, &sealed).map_err(|_| {
            AppError::new(
                ErrorCode::BiometricStale,
                "Windows Hello envelope no longer opens",
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
            match status {
                KeyCredentialStatus::UserCanceled => {
                    AppError::new(ErrorCode::Cancelled, "Windows Hello was cancelled")
                }
                // The credential was deleted since enrolment, e.g. by resetting Windows Hello.
                KeyCredentialStatus::NotFound => AppError::new(
                    ErrorCode::BiometricStale,
                    "Windows Hello credential is gone",
                ),
                _ => AppError::new(
                    ErrorCode::Internal,
                    format!("Windows Hello failed ({status:?})"),
                ),
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

    /// The record, in the app data directory beside the vault database, mode 0600.
    const RECORD: &str = "touch-id.json";
    const RECORD_VERSION: u8 = 1;
    const AAD: &[u8] = b"hatoba/touch-id-vault-key/v1";
    const HKDF_INFO: &str = "hatoba/touch-id-wrap/v1";
    /// The undocumented attribute that holds a Secure Enclave key's wrapped form.
    const BLOB_ATTRIBUTE: &str = "toid";
    /// An uncompressed P-256 point.
    const POINT_LEN: usize = 65;

    // The same small numbers mean different things in each domain, so errors match on both.
    const LA_DOMAIN: &str = "com.apple.LocalAuthentication";
    const OS_DOMAIN: &str = "NSOSStatusErrorDomain";
    const CTK_DOMAIN: &str = "CryptoTokenKit";
    /// `LAError.userCancel`.
    const ERR_USER_CANCEL: isize = -2;
    /// `LAError.systemCancel`: what a closed lid answers.
    const ERR_SYSTEM_CANCEL: isize = -4;
    /// `LAError.biometryNotAvailable`, `biometryNotEnrolled`, `biometryLockout`, `notInteractive`.
    const ERR_BIOMETRY_NOT_AVAILABLE: isize = -6;
    const ERR_BIOMETRY_NOT_ENROLLED: isize = -7;
    const ERR_BIOMETRY_LOCKOUT: isize = -8;
    const ERR_NOT_INTERACTIVE: isize = -1004;
    /// `errSecUserCanceled`, `errSecAuthFailed`, `errSecInteractionNotAllowed`.
    const ERR_USER_CANCELED: isize = -128;
    const ERR_AUTH_FAILED: isize = -25293;
    const ERR_INTERACTION_NOT_ALLOWED: isize = -25308;
    /// The Secure Enclave's answer once the enrolled fingerprints changed (`biometryCurrentSet`):
    /// "unable to compute shared secret".
    const ERR_KEY_INVALIDATED: isize = -3;

    /// Whether the Mac has Touch ID. Read from the biometric type: `canEvaluatePolicy` fails on a
    /// Mac with its lid closed, which would hide the switch where Touch ID otherwise works.
    pub fn available() -> bool {
        let context = unsafe { LAContext::new() };
        // SAFETY: only the policy result and the biometric type of a context made here are read.
        unsafe {
            // Called because it is what fills in `biometryType`; its own answer is not used.
            if let Err(error) =
                context.canEvaluatePolicy_error(LAPolicy::DeviceOwnerAuthenticationWithBiometrics)
            {
                tracing::debug!("the biometrics policy cannot be evaluated here: {error}");
            }
            context.biometryType() != LABiometryType::None
        }
    }

    /// The file is the enrolment, so this asks nothing of the Secure Enclave.
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
        // The Secure Enclave calls block while the sheet is up, so they stay off the async runtime.
        let record = tauri::async_runtime::spawn_blocking(move || {
            let enclave = enclave_key()?;
            let ephemeral = ephemeral_key()?;
            let peer = public_key(&ephemeral)?;
            let point = point_of(&peer)?;
            let wrap = wrap_key(&enclave, &peer)?;
            let envelope: Envelope = seal(&wrap, AAD, vault.as_slice())?;
            let blob = key_blob(&enclave)?;
            // Check that the blob restores this same key before keeping it. Restoring without the
            // access control and comparing public points asks for no second Touch ID.
            let private = unsafe { kSecAttrKeyClassPrivate };
            // Nothing was saved yet, so a failure here is not a stale record.
            let restored = import(&blob, &import_attributes(private, Some(&blob), None))
                .map_err(|_| AppError::internal("Secure Enclave blob does not restore"))?;
            if point_of(&public_key(&enclave)?)? != point_of(&public_key(&restored)?)? {
                return Err(AppError::internal(
                    "Secure Enclave blob restored a different key",
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

    /// What `touch-id.json` holds.
    #[derive(Debug, serde::Deserialize, serde::Serialize)]
    struct Record {
        version: u8,
        /// The `toid` blob of the Secure Enclave key, base64.
        key_blob: String,
        /// The uncompressed point of the ephemeral key, base64.
        ephemeral_public: String,
        /// The vault key sealed under the HKDF of their ECDH, as envelope JSON.
        envelope: String,
    }

    impl Record {
        /// The vault key. Restoring the key is what asks for Touch ID.
        fn open(&self) -> AppResult<Key32> {
            let blob = self.field(&self.key_blob, "key blob")?;
            let point = self.field(&self.ephemeral_public, "ephemeral key")?;
            let enclave = key_from_blob(&blob)?;
            let peer = public_key_from_point(&point)?;
            let wrap = wrap_key(&enclave, &peer)?;
            let plain = open_json(&wrap, AAD, &self.envelope).map_err(|_| {
                AppError::new(
                    ErrorCode::BiometricStale,
                    "Touch ID envelope no longer opens",
                )
            })?;
            let bytes: [u8; 32] = plain
                .as_slice()
                .try_into()
                .map_err(|_| AppError::internal("bad Touch ID envelope"))?;
            Ok(Zeroizing::new(bytes))
        }

        fn field(&self, encoded: &str, what: &str) -> AppResult<Zeroizing<Vec<u8>>> {
            B64.decode(encoded).map(Zeroizing::new).map_err(|_| {
                AppError::new(
                    ErrorCode::BiometricStale,
                    format!("Touch ID record: damaged {what}"),
                )
            })
        }
    }

    /// A Secure Enclave P-256 key for key agreement, behind Touch ID, kept in no keychain.
    fn enclave_key() -> AppResult<SecKey> {
        let mut options = GenerateKeyOptions::default();
        options
            .set_key_type(KeyType::ec())
            .set_size_in_bits(256)
            .set_token(Token::SecureEnclave)
            .set_access_control(control()?);
        // No `set_location`: `kSecAttrIsPermanent` stays false, so no keychain gets the key.
        SecKey::new(&options).map_err(|e| AppError::internal(format!("Secure Enclave key: {e}")))
    }

    /// The software half of the exchange; only its point is kept.
    fn ephemeral_key() -> AppResult<SecKey> {
        let mut options = GenerateKeyOptions::default();
        options.set_key_type(KeyType::ec()).set_size_in_bits(256);
        SecKey::new(&options).map_err(|e| AppError::internal(format!("ephemeral key: {e}")))
    }

    /// Touch ID with the current fingerprints, for private key use. Given when the key is made and
    /// again when the blob is restored to unlock.
    fn control() -> AppResult<SecAccessControl> {
        let flags = AccessControlOptions::PRIVATE_KEY_USAGE.bits()
            | AccessControlOptions::BIOMETRY_CURRENT_SET.bits();
        SecAccessControl::create_with_protection(
            Some(ProtectionMode::AccessibleWhenUnlockedThisDeviceOnly),
            flags,
        )
        .map_err(|e| AppError::internal(format!("access control: {e}")))
    }

    /// The key's wrapped form, which is all of it that outlives the process.
    fn key_blob(key: &SecKey) -> AppResult<Vec<u8>> {
        attribute(key, BLOB_ATTRIBUTE)?
            .ok_or_else(|| AppError::internal("Secure Enclave key has no toid blob"))
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

    /// The key a blob holds, restored with the access control, which raises the Touch ID sheet.
    /// Without the control the restored key does the exchange without asking.
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

    /// The attributes `SecKeyCreateWithData` reads for a P-256 key. A Secure Enclave key needs its
    /// blob back under `toid` as well: given only the token, the import makes a new key.
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
            // Typically a record from another Mac. The reason is in the system language, so it
            // goes to the log, not into `detail`.
            tracing::warn!("the Touch ID key did not import: {reason}");
            return Err(AppError::new(
                ErrorCode::BiometricStale,
                "Touch ID key does not import on this Mac",
            ));
        }
        Ok(unsafe { SecKey::wrap_under_create_rule(key) })
    }

    fn public_key(key: &SecKey) -> AppResult<SecKey> {
        key.public_key()
            .ok_or_else(|| AppError::internal("key has no public half"))
    }

    /// The uncompressed point of a public key.
    fn point_of(key: &SecKey) -> AppResult<Vec<u8>> {
        key.external_representation()
            .map(|data| data.bytes().to_vec())
            .filter(|point| point.len() == POINT_LEN)
            .ok_or_else(|| AppError::internal("key has no uncompressed point"))
    }

    /// HKDF-SHA256 of the ECDH between the Secure Enclave key and its peer.
    fn wrap_key(enclave: &SecKey, peer_public: &SecKey) -> AppResult<Key32> {
        let shared = enclave
            .key_exchange(Algorithm::ECDHKeyExchangeStandard, peer_public, 32, None)
            .map_err(|error| exchange_error(&error))?;
        let shared = Zeroizing::new(shared);
        Ok(hkdf_sha256(&shared, HKDF_INFO))
    }

    /// What a refused exchange means. The framework's description is in the system language, so it
    /// goes to the log and `detail` stays English.
    fn exchange_error(error: &core_foundation::error::CFError) -> AppError {
        let domain = error.domain().to_string();
        let code = error.code();
        match (domain.as_str(), code) {
            (LA_DOMAIN, ERR_USER_CANCEL | ERR_USER_CANCELED) | (OS_DOMAIN, ERR_USER_CANCELED) => {
                AppError::new(ErrorCode::Cancelled, "Touch ID was cancelled")
            }
            // The lid is closed, too many fingers failed, there is no sensor or no finger, or the
            // Touch ID keyboard of a desktop Mac is away.
            (
                LA_DOMAIN,
                ERR_SYSTEM_CANCEL
                | ERR_BIOMETRY_NOT_AVAILABLE
                | ERR_BIOMETRY_NOT_ENROLLED
                | ERR_BIOMETRY_LOCKOUT
                | ERR_NOT_INTERACTIVE,
            )
            | (OS_DOMAIN, ERR_INTERACTION_NOT_ALLOWED | ERR_AUTH_FAILED) => {
                tracing::warn!("Touch ID unavailable: {} ({error})", error.description());
                AppError::new(ErrorCode::BiometricUnavailable, "Touch ID unavailable")
            }
            (CTK_DOMAIN, ERR_KEY_INVALIDATED) => {
                tracing::warn!(
                    "Touch ID key invalidated: {} ({error})",
                    error.description()
                );
                AppError::new(
                    ErrorCode::BiometricStale,
                    "Touch ID key invalidated (fingerprints changed)",
                )
            }
            (_, _) => {
                tracing::warn!("Touch ID failed: {} ({error})", error.description());
                AppError::new(
                    ErrorCode::Internal,
                    format!("Touch ID failed ({domain} {code})"),
                )
            }
        }
    }

    /// A keychain attribute constant as the `CFType` the dictionaries take.
    fn wrap(constant: CFStringRef) -> CFType {
        // SAFETY: the framework's own constant strings, which live as long as it does.
        unsafe { CFString::wrap_under_get_rule(constant).into_CFType() }
    }

    fn record_path(app: &AppHandle) -> AppResult<PathBuf> {
        let dir = tauri::Manager::path(app)
            .app_local_data_dir()
            .map_err(|e| AppError::internal(format!("no app data directory: {e}")))?;
        Ok(record_file(&dir))
    }

    fn record_file(dir: &Path) -> PathBuf {
        dir.join(RECORD)
    }

    /// Writes the record at mode 0600 through a temporary file, so it is never half written.
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
        let record: Record = serde_json::from_str(&json)
            .map_err(|_| AppError::new(ErrorCode::BiometricStale, "Touch ID record is damaged"))?;
        if record.version != RECORD_VERSION {
            return Err(AppError::new(
                ErrorCode::BiometricStale,
                format!("Touch ID record has version {}", record.version),
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
            // The answer after a fingerprint was added.
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
            assert_eq!(
                exchange_error(&framework_error(LA_DOMAIN, ERR_USER_CANCEL)).code,
                Cancelled
            );
            assert_eq!(
                exchange_error(&framework_error(OS_DOMAIN, ERR_USER_CANCELED)).code,
                Cancelled
            );
            // The same numbers in other domains: `-4` is `errSecUnimplemented` here, not a lid.
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

        /// A temporary directory of this test's own, removed when it ends.
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
        fn a_damaged_record_is_stale() {
            let scratch = Scratch::new("damaged");
            let path = scratch.record();
            for damaged in ["", "not json at all", r#"{"version": 1}"#] {
                fs::write(&path, damaged).unwrap();
                let err = read_record(&path).unwrap_err();
                assert_eq!(err.code, ErrorCode::BiometricStale, "{damaged}");
            }
        }

        #[test]
        fn a_record_from_another_version_is_stale() {
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
            assert_eq!(err.code, ErrorCode::BiometricStale);
            assert!(err.detail.contains("key blob"), "{}", err.detail);
        }

        /// The kept point imports as the same public key. Software keys only, so it runs anywhere.
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

        /// The blob restores the key that was made, compared by public point. Needs a Secure
        /// Enclave, so it is gated:
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
