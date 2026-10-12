//! Private key import, generation and fingerprints (KEY-01, KEY-02, WIN-09).
//!
//! Supported input formats: OpenSSH (plain or encrypted), PEM (PKCS#1 RSA,
//! SEC1 EC, PKCS#8, encrypted PKCS#8, legacy `Proc-Type: 4,ENCRYPTED`
//! PKCS#1/SEC1 with AES-CBC) and PuTTY `.ppk` v2/v3. Everything but OpenSSH
//! input is re-encoded as an unencrypted OpenSSH private key.

use std::fmt;

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD};
use russh::keys::ssh_key::private::{KeypairData, RsaKeypair};
use russh::keys::ssh_key::{Algorithm, EcdsaCurve, HashAlg, LineEnding, PrivateKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::ppk;

/// Key family, serialized as `"ed25519" | "ecdsa" | "rsa"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KeyAlgorithm {
    /// Ed25519.
    Ed25519,
    /// ECDSA (NIST P-256 / P-384 / P-521).
    Ecdsa,
    /// RSA.
    Rsa,
}

/// Which kind of key [`generate_key`] should create.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GenerateKind {
    /// Ed25519 (recommended).
    Ed25519,
    /// RSA with a 4096-bit modulus (slow to generate).
    Rsa4096,
}

/// Why a private key could not be parsed. Drives the messages for KEY-01.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    /// The text is not a private key format this crate understands.
    #[error("unsupported key format")]
    UnsupportedFormat,
    /// The key is encrypted and no passphrase was supplied.
    #[error("the key is encrypted and needs a passphrase")]
    PassphraseRequired,
    /// A passphrase was supplied but it does not decrypt the key.
    #[error("wrong passphrase")]
    WrongPassphrase,
    /// The key (or cipher/KDF) uses an algorithm that is not supported.
    #[error("unsupported key algorithm: {0}")]
    UnsupportedAlgorithm(String),
    /// The data looks like a key of a known format but is corrupt.
    #[error("invalid key: {0}")]
    Invalid(String),
}

/// A validated private key plus the metadata the vault stores about it.
#[derive(Clone)]
pub struct ParsedKey {
    /// Key family.
    pub algorithm: KeyAlgorithm,
    /// Key size in bits (256 for Ed25519, curve size for ECDSA, modulus size for RSA).
    pub bits: u32,
    /// OpenSSH private key PEM. For OpenSSH input this is the original text
    /// (possibly passphrase-encrypted, see `encrypted`); for every other
    /// format it is the re-encoded, unencrypted key.
    pub openssh_private: Zeroizing<String>,
    /// Whether `openssh_private` still needs the passphrase.
    pub encrypted: bool,
    /// `ssh-ed25519 AAAA... comment`
    pub public_openssh: String,
    /// `SHA256:...` fingerprint (unpadded base64, same as `ssh-keygen -l`).
    pub fingerprint: String,
    /// Comment stored in the key (may be empty).
    pub comment: String,
}

impl fmt::Debug for ParsedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ParsedKey")
            .field("algorithm", &self.algorithm)
            .field("bits", &self.bits)
            .field("openssh_private", &"<redacted>")
            .field("encrypted", &self.encrypted)
            .field("public_openssh", &self.public_openssh)
            .field("fingerprint", &self.fingerprint)
            .field("comment", &self.comment)
            .finish()
    }
}

/// How the input text is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    OpenSsh,
    Ppk,
    /// PKCS#1 / SEC1 / PKCS#8 PEM, possibly encrypted.
    Pem {
        legacy_encrypted: bool,
        pkcs8_encrypted: bool,
    },
}

fn detect(text: &str) -> Result<Format, KeyError> {
    if ppk::is_ppk(text) {
        return Ok(Format::Ppk);
    }
    let label = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("-----BEGIN ")?.strip_suffix("-----"))
        .ok_or(KeyError::UnsupportedFormat)?;
    match label {
        "OPENSSH PRIVATE KEY" => Ok(Format::OpenSsh),
        "RSA PRIVATE KEY" | "EC PRIVATE KEY" => Ok(Format::Pem {
            legacy_encrypted: text.contains("Proc-Type: 4,ENCRYPTED"),
            pkcs8_encrypted: false,
        }),
        "PRIVATE KEY" => Ok(Format::Pem {
            legacy_encrypted: false,
            pkcs8_encrypted: false,
        }),
        "ENCRYPTED PRIVATE KEY" => Ok(Format::Pem {
            legacy_encrypted: false,
            pkcs8_encrypted: true,
        }),
        "DSA PRIVATE KEY" => Err(KeyError::UnsupportedAlgorithm("ssh-dss".into())),
        _ => Err(KeyError::UnsupportedFormat),
    }
}

/// An empty passphrase is treated as "no passphrase".
fn normalize_passphrase(passphrase: Option<&str>) -> Option<&str> {
    passphrase.filter(|p| !p.is_empty())
}

/// Decodes any supported private key text. Returns the key and whether the
/// input was passphrase-protected.
fn decode(text: &str, passphrase: Option<&str>) -> Result<(PrivateKey, Format, bool), KeyError> {
    let passphrase = normalize_passphrase(passphrase);
    let format = detect(text)?;
    match format {
        Format::Ppk => {
            let encrypted = ppk::inspect(text)?.encrypted;
            Ok((ppk::decode(text, passphrase)?, format, encrypted))
        }
        Format::OpenSsh => {
            let key = PrivateKey::from_openssh(text.trim()).map_err(map_ssh_key_error)?;
            if !key.is_encrypted() {
                return Ok((key, format, false));
            }
            let passphrase = passphrase.ok_or(KeyError::PassphraseRequired)?;
            let key = key
                .decrypt(passphrase)
                .map_err(|_| KeyError::WrongPassphrase)?;
            Ok((key, format, true))
        }
        Format::Pem {
            legacy_encrypted,
            pkcs8_encrypted,
        } => {
            let encrypted = legacy_encrypted || pkcs8_encrypted;
            if encrypted && passphrase.is_none() {
                return Err(KeyError::PassphraseRequired);
            }
            let key = if legacy_encrypted {
                let plain = decrypt_legacy_pem(text, passphrase.unwrap_or_default())?;
                russh::keys::decode_secret_key(plain.as_str(), None)
                    .map_err(|_| KeyError::WrongPassphrase)?
            } else {
                russh::keys::decode_secret_key(text, passphrase.filter(|_| pkcs8_encrypted))
                    .map_err(|e| map_russh_key_error(e, pkcs8_encrypted))?
            };
            Ok((key, format, encrypted))
        }
    }
}

fn map_ssh_key_error(e: russh::keys::ssh_key::Error) -> KeyError {
    use russh::keys::ssh_key::Error as E;
    match e {
        E::AlgorithmUnsupported { algorithm } => {
            KeyError::UnsupportedAlgorithm(algorithm.to_string())
        }
        E::AlgorithmUnknown => KeyError::UnsupportedAlgorithm("unknown".into()),
        other => KeyError::Invalid(other.to_string()),
    }
}

fn map_russh_key_error(e: russh::keys::Error, encrypted: bool) -> KeyError {
    use russh::keys::Error as E;
    match e {
        E::KeyIsEncrypted => KeyError::PassphraseRequired,
        E::UnknownAlgorithm(oid) => KeyError::UnsupportedAlgorithm(oid.to_string()),
        E::UnsupportedKeyType {
            key_type_string, ..
        } => KeyError::UnsupportedAlgorithm(key_type_string),
        E::SshKey(inner) => map_ssh_key_error(inner),
        // A wrong passphrase surfaces as a padding, DER or PBES2 error.
        _ if encrypted => KeyError::WrongPassphrase,
        other => KeyError::Invalid(other.to_string()),
    }
}

/// Decrypts a legacy OpenSSL-style encrypted PEM (`Proc-Type: 4,ENCRYPTED` /
/// `DEK-Info`) and returns the same PEM without the encryption headers.
fn decrypt_legacy_pem(text: &str, passphrase: &str) -> Result<Zeroizing<String>, KeyError> {
    use aes::cipher::block_padding::Pkcs7;
    use aes::cipher::{BlockModeDecrypt, KeyIvInit};

    let mut lines = text.lines().map(str::trim);
    let begin = lines
        .by_ref()
        .find(|l| l.starts_with("-----BEGIN "))
        .ok_or(KeyError::UnsupportedFormat)?
        .to_owned();
    let end = begin.replace("BEGIN", "END");

    let mut dek: Option<(String, Vec<u8>)> = None;
    let mut body = String::new();
    for line in lines {
        if line.starts_with("-----END ") {
            break;
        }
        if let Some(info) = line.strip_prefix("DEK-Info:") {
            let (alg, iv) = info
                .trim()
                .split_once(',')
                .ok_or_else(|| KeyError::Invalid("malformed DEK-Info header".into()))?;
            dek = Some((alg.trim().to_ascii_uppercase(), decode_hex(iv.trim())?));
        } else if line.contains(':') || line.is_empty() {
            // Other RFC 1421 headers (Proc-Type) and the blank separator line.
        } else {
            body.push_str(line);
        }
    }
    let (alg, iv) = dek.ok_or_else(|| KeyError::Invalid("missing DEK-Info header".into()))?;
    let key_len = match alg.as_str() {
        "AES-128-CBC" => 16,
        "AES-192-CBC" => 24,
        "AES-256-CBC" => 32,
        other => {
            return Err(KeyError::UnsupportedAlgorithm(format!(
                "{other} (legacy PEM encryption)"
            )));
        }
    };
    if iv.len() != 16 {
        return Err(KeyError::Invalid(
            "malformed DEK-Info initialization vector".into(),
        ));
    }
    let mut data = Zeroizing::new(
        STANDARD
            .decode(body.as_bytes())
            .map_err(|e| KeyError::Invalid(format!("invalid base64 body: {e}")))?,
    );

    // OpenSSL EVP_BytesToKey with MD5, one iteration, first 8 IV bytes as salt.
    let mut key = Zeroizing::new(Vec::with_capacity(key_len + 16));
    let mut prev: Vec<u8> = Vec::new();
    while key.len() < key_len {
        let mut input = prev.clone();
        input.extend_from_slice(passphrase.as_bytes());
        input.extend_from_slice(&iv[..8]);
        prev = md5::compute(&input).0.to_vec();
        key.extend_from_slice(&prev);
    }
    let key = &key[..key_len];

    let plain: &[u8] = match key_len {
        16 => cbc::Decryptor::<aes::Aes128>::new_from_slices(key, &iv)
            .map_err(|_| KeyError::WrongPassphrase)?
            .decrypt_padded::<Pkcs7>(&mut data),
        24 => cbc::Decryptor::<aes::Aes192>::new_from_slices(key, &iv)
            .map_err(|_| KeyError::WrongPassphrase)?
            .decrypt_padded::<Pkcs7>(&mut data),
        _ => cbc::Decryptor::<aes::Aes256>::new_from_slices(key, &iv)
            .map_err(|_| KeyError::WrongPassphrase)?
            .decrypt_padded::<Pkcs7>(&mut data),
    }
    .map_err(|_| KeyError::WrongPassphrase)?;

    // A correct passphrase yields a DER SEQUENCE.
    if plain.first() != Some(&0x30) {
        return Err(KeyError::WrongPassphrase);
    }
    let b64 = Zeroizing::new(STANDARD.encode(plain));
    let mut out = Zeroizing::new(String::with_capacity(b64.len() + 100));
    out.push_str(&begin);
    out.push('\n');
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push('\n');
    }
    out.push_str(&end);
    out.push('\n');
    Ok(out)
}

fn decode_hex(s: &str) -> Result<Vec<u8>, KeyError> {
    if !s.len().is_multiple_of(2) || !s.is_ascii() {
        return Err(KeyError::Invalid("malformed hex value".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|_| KeyError::Invalid("malformed hex value".into()))
        })
        .collect()
}

/// Family and size of a decoded key, or [`KeyError::UnsupportedAlgorithm`].
fn describe(key: &PrivateKey) -> Result<(KeyAlgorithm, u32), KeyError> {
    match key.algorithm() {
        Algorithm::Ed25519 => Ok((KeyAlgorithm::Ed25519, 256)),
        Algorithm::Ecdsa { curve } => Ok((
            KeyAlgorithm::Ecdsa,
            match curve {
                EcdsaCurve::NistP256 => 256,
                EcdsaCurve::NistP384 => 384,
                EcdsaCurve::NistP521 => 521,
            },
        )),
        Algorithm::Rsa { .. } => {
            let bits = key
                .public_key()
                .key_data()
                .rsa()
                .map(|rsa| rsa.key_size())
                .unwrap_or(0);
            Ok((KeyAlgorithm::Rsa, bits))
        }
        other => Err(KeyError::UnsupportedAlgorithm(other.to_string())),
    }
}

fn build_parsed(
    key: &PrivateKey,
    openssh_private: Zeroizing<String>,
    encrypted: bool,
) -> Result<ParsedKey, KeyError> {
    let (algorithm, bits) = describe(key)?;
    let public = key.public_key();
    Ok(ParsedKey {
        algorithm,
        bits,
        openssh_private,
        encrypted,
        public_openssh: public
            .to_openssh()
            .map_err(|e| KeyError::Invalid(e.to_string()))?,
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
        comment: public.comment().as_str_lossy().to_owned(),
    })
}

/// Parses a private key in any supported format.
///
/// `passphrase` is required for encrypted keys; an empty string counts as
/// "none". Errors tell the user what to fix: [`KeyError::PassphraseRequired`],
/// [`KeyError::WrongPassphrase`], [`KeyError::UnsupportedFormat`], ...
pub fn parse_private_key(text: &str, passphrase: Option<&str>) -> Result<ParsedKey, KeyError> {
    let (key, format, was_encrypted) = decode(text, passphrase)?;
    if format == Format::OpenSsh {
        // Keep the original (possibly encrypted) text, only normalising line endings.
        let original = if text.contains('\r') {
            let mut s = text.trim().replace("\r\n", "\n");
            s.push('\n');
            s
        } else {
            text.to_owned()
        };
        return build_parsed(&key, Zeroizing::new(original), was_encrypted);
    }
    let pem = key
        .to_openssh(LineEnding::LF)
        .map_err(|e| KeyError::Invalid(e.to_string()))?;
    build_parsed(&key, pem, false)
}

/// Loads the key used to authenticate a connection (any supported format).
pub(crate) fn load_for_auth(text: &str, passphrase: Option<&str>) -> Result<PrivateKey, KeyError> {
    decode(text, passphrase).map(|(key, ..)| key)
}

/// Generates a new key pair (KEY-02). RSA 4096 can take seconds to minutes;
/// call from `spawn_blocking`.
///
/// With a passphrase the private key is encrypted (AES-256-CTR, bcrypt-pbkdf)
/// and [`ParsedKey::encrypted`] is `true`.
pub fn generate_key(
    kind: GenerateKind,
    comment: &str,
    passphrase: Option<&str>,
) -> Result<ParsedKey, KeyError> {
    let mut rng = russh::keys::key::safe_rng();
    let mut key = match kind {
        GenerateKind::Ed25519 => PrivateKey::random(&mut rng, Algorithm::Ed25519)
            .map_err(|e| KeyError::Invalid(e.to_string()))?,
        GenerateKind::Rsa4096 => {
            let pair =
                RsaKeypair::random(&mut rng, 4096).map_err(|e| KeyError::Invalid(e.to_string()))?;
            PrivateKey::new(KeypairData::from(pair), "")
                .map_err(|e| KeyError::Invalid(e.to_string()))?
        }
    };
    key.set_comment(comment);

    let passphrase = normalize_passphrase(passphrase);
    let stored = match passphrase {
        Some(p) => key
            .encrypt(&mut rng, p)
            .map_err(|e| KeyError::Invalid(e.to_string()))?,
        None => key.clone(),
    };
    let pem = stored
        .to_openssh(LineEnding::LF)
        .map_err(|e| KeyError::Invalid(e.to_string()))?;
    // Metadata comes from the plaintext key: an encrypted key does not carry its comment in the
    // clear.
    build_parsed(&key, pem, passphrase.is_some())
}

/// SHA-256 fingerprint (`SHA256:...`, unpadded base64) of a public key given
/// either as an OpenSSH line (`ssh-ed25519 AAAA... comment`) or as the bare
/// base64 blob. Returns an empty string if the input is not a valid key blob;
/// use [`try_fingerprint_sha256`] to detect that.
pub fn fingerprint_sha256(public_openssh_line_or_blob: &str) -> String {
    try_fingerprint_sha256(public_openssh_line_or_blob).unwrap_or_default()
}

/// Fallible variant of [`fingerprint_sha256`].
pub fn try_fingerprint_sha256(public_openssh_line_or_blob: &str) -> Result<String, KeyError> {
    let mut tokens = public_openssh_line_or_blob.split_whitespace();
    let first = tokens.next().ok_or(KeyError::UnsupportedFormat)?;
    // `ssh-ed25519 AAAA...` vs a bare blob (blobs always start with "AAAA").
    let blob_b64 = match tokens.next() {
        Some(second) if !first.starts_with("AAAA") => second,
        _ => first,
    };
    let blob = STANDARD
        .decode(blob_b64)
        .or_else(|_| STANDARD_NO_PAD.decode(blob_b64.trim_end_matches('=')))
        .map_err(|_| KeyError::Invalid("public key is not valid base64".into()))?;
    if blob.len() < 8 {
        return Err(KeyError::Invalid("public key blob is too short".into()));
    }
    let digest = Sha256::digest(&blob);
    Ok(format!("SHA256:{}", STANDARD_NO_PAD.encode(digest)))
}
