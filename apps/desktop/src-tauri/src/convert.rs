//! Model → DTO conversions. This is the boundary where secret fields are dropped (spec §10.1).

use hatoba_core::model::{Group, Host, HostAuth, KeyAlgorithm as CoreAlg, SshKey};
use hatoba_core::vault::Vault;

use crate::dto::{AuthKind, GroupView, HostView, KeyAlgorithm, KeyView};

pub fn host_view(id: &str, host: &Host, vault: &Vault) -> HostView {
    let (auth_kind, has_password, key_id) = match &host.auth {
        HostAuth::Password { password } => (AuthKind::Password, !password.is_empty(), None),
        HostAuth::Key { key_id } => (AuthKind::Key, false, Some(key_id.clone())),
        HostAuth::Agent => (AuthKind::Agent, false, None),
        HostAuth::Ask => (AuthKind::Ask, false, None),
    };
    HostView {
        id: id.to_owned(),
        name: host.name.clone(),
        address: host.address.clone(),
        port: host.port,
        username: host.username.clone(),
        auth_kind,
        has_password,
        key_id,
        group_id: host.group_id.clone(),
        tags: host.tags.clone(),
        favorite: host.favorite,
        jump_host_id: host.jump_host_id.clone(),
        note: host.note.clone(),
        ai_notes: host.ai_notes.clone(),
        updated_at: host.updated_at,
        last_connected_at: vault.last_connected(id),
        os: vault.host_os(id),
    }
}

pub fn group_view(id: &str, group: &Group) -> GroupView {
    GroupView {
        id: id.to_owned(),
        name: group.name.clone(),
        parent_id: group.parent_id.clone(),
        sort: group.sort.clamp(i32::MIN as i64, i32::MAX as i64) as i32,
    }
}

pub fn key_algorithm(alg: CoreAlg) -> KeyAlgorithm {
    match alg {
        CoreAlg::Ed25519 => KeyAlgorithm::Ed25519,
        CoreAlg::Ecdsa => KeyAlgorithm::Ecdsa,
        CoreAlg::Rsa => KeyAlgorithm::Rsa,
    }
}

/// Key size in bits, derived from the public key so it needn't be stored.
pub fn key_bits(key: &SshKey) -> u32 {
    match key.algorithm {
        CoreAlg::Ed25519 => 256,
        CoreAlg::Ecdsa => {
            if key.public_key.contains("nistp521") {
                521
            } else if key.public_key.contains("nistp384") {
                384
            } else {
                256
            }
        }
        CoreAlg::Rsa => rsa_bits(&key.public_key).unwrap_or(0),
    }
}

/// Reads the modulus length out of an `ssh-rsa AAAA...` line.
fn rsa_bits(public_line: &str) -> Option<u32> {
    use base64::Engine as _;
    let blob = public_line.split_whitespace().nth(1)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(blob)
        .ok()?;
    let mut rest = bytes.as_slice();
    let mut field = || -> Option<&[u8]> {
        let len = u32::from_be_bytes(rest.get(..4)?.try_into().ok()?) as usize;
        let value = rest.get(4..4 + len)?;
        rest = &rest[4 + len..];
        Some(value)
    };
    let _alg = field()?;
    let _e = field()?;
    let n = field()?;
    let n = n.strip_prefix(&[0]).unwrap_or(n);
    let leading = n.first().map_or(0, |b| b.leading_zeros());
    Some((n.len() as u32 * 8).saturating_sub(leading))
}

pub fn key_view(id: &str, key: &SshKey, vault: &Vault) -> KeyView {
    let used_by = vault
        .hosts()
        .into_iter()
        .filter(|(_, h)| matches!(&h.auth, HostAuth::Key { key_id } if key_id == id))
        .map(|(hid, _)| hid)
        .collect();
    KeyView {
        id: id.to_owned(),
        name: key.name.clone(),
        algorithm: key_algorithm(key.algorithm),
        bits: key_bits(key),
        public_key: key.public_key.clone(),
        fingerprint: key.fingerprint.clone(),
        comment: key.comment.clone(),
        has_passphrase: key.passphrase.is_some()
            || key.private_key.contains("ENCRYPTED")
            || is_encrypted_openssh(&key.private_key),
        created_at: key.created_at,
        updated_at: key.updated_at,
        used_by,
    }
}

/// OpenSSH-format keys don't say "ENCRYPTED" in the armor; the cipher name is inside the blob.
fn is_encrypted_openssh(pem: &str) -> bool {
    use base64::Engine as _;
    if !pem.contains("BEGIN OPENSSH PRIVATE KEY") {
        return false;
    }
    let body: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(body.trim()) else {
        return false;
    };
    // "openssh-key-v1\0" then a length-prefixed cipher name; "none" means unencrypted.
    let magic = b"openssh-key-v1\0";
    let Some(rest) = bytes.strip_prefix(magic.as_slice()) else {
        return false;
    };
    let Some(len) = rest
        .get(..4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
    else {
        return false;
    };
    rest.get(4..4 + len).is_some_and(|cipher| cipher != b"none")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rsa_bits_from_public_line() {
        // 2048-bit test key (public part only).
        let line = "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAABAQC5yXqO2G+uN7XwKr6wYxn9xmR8PUYBu7T9EE1T+NF4iXL/JdPuRZl735qQOkeXPRqyCFwWCLpoWG/YFoKhIHh2mFa+7eEkQsJ8gXDrt1uiMiTkMEryyxSiDnV883q1SBgdbtPczDrE9qOHttWYk5nSfmK544NjFy8XSE+depQOwtCP2nn4hhcpX+FxJwFOj9ZV5cDToN4NoHS775IySxu8hz2Aa3zgy93eKEQk2R8NMGOB7lBl2kggZZojrJrm6yvMQgMufB9u9FCu3YRukdqk5E/94kir5xXi/Z2bU75Gjz69tf8k5GJDUL63XeNF8jlygqGg+99KLEkxwKfrFqJn test";
        assert_eq!(rsa_bits(line), Some(2048));
    }
}
