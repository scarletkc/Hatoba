use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

/// Which proxy a host's connection goes through (SSH-13).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostProxy {
    /// The default proxy chosen on each device, if any. That choice is device-local, because a
    /// proxy such as one running on `127.0.0.1` exists only on some devices.
    #[default]
    DeviceDefault,
    /// No proxy, whatever the device default is.
    Direct,
    Proxy {
        /// Id of the [`Proxy`] item.
        proxy_id: String,
    },
}

impl HostProxy {
    /// Whether this is [`HostProxy::DeviceDefault`] (which is not written out).
    #[must_use]
    pub fn is_device_default(&self) -> bool {
        matches!(self, Self::DeviceDefault)
    }

    /// The saved proxy's id, if the host names one.
    #[must_use]
    pub fn proxy_id(&self) -> Option<&str> {
        match self {
            Self::Proxy { proxy_id } => Some(proxy_id),
            Self::DeviceDefault | Self::Direct => None,
        }
    }
}

/// The protocol of a [`Proxy`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyKind {
    /// SOCKS5 (RFC 1928), with optional username and password (RFC 1929).
    #[default]
    Socks5,
    /// An HTTP proxy that supports `CONNECT`, with optional Basic authentication.
    Http,
}

impl ProxyKind {
    /// The `kind` string as it appears in the plaintext JSON.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Socks5 => "socks5",
            Self::Http => "http",
        }
    }
}

/// A saved proxy that hosts connect through (SSH-13). The password is wiped on drop and never
/// printed.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct Proxy {
    /// Display name, e.g. `Office SOCKS`.
    pub name: String,
    #[zeroize(skip)]
    pub kind: ProxyKind,
    /// Domain name or IP address of the proxy.
    pub address: String,
    /// TCP port, default 1080.
    pub port: u16,
    /// Login name on the proxy; empty when it needs none.
    pub username: String,
    /// Password on the proxy (secret), used only with a username. Never sent to the front-end.
    pub password: Zeroizing<String>,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

impl Default for Proxy {
    fn default() -> Self {
        Self {
            name: String::new(),
            kind: ProxyKind::default(),
            address: String::new(),
            port: 1080,
            username: String::new(),
            password: Zeroizing::default(),
            updated_at: 0,
        }
    }
}

impl fmt::Debug for Proxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Proxy")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .field("address", &self.address)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// A host group (one level of nesting in the MVP).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct Group {
    pub name: String,
    pub parent_id: Option<String>,
    pub sort: i64,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

/// Key algorithm of an [`SshKey`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAlgorithm {
    #[default]
    Ed25519,
    Ecdsa,
    Rsa,
}

/// A private key in the key store. The private key and passphrase are wiped on drop.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct SshKey {
    pub name: String,
    #[zeroize(skip)]
    pub algorithm: KeyAlgorithm,
    /// Private key in OpenSSH format (secret).
    pub private_key: Zeroizing<String>,
    /// Passphrase protecting the private key, if any (secret).
    pub passphrase: Option<Zeroizing<String>>,
    /// Public key in OpenSSH format.
    pub public_key: String,
    /// `SHA256:...` fingerprint.
    pub fingerprint: String,
    pub comment: String,
    /// Creation time, Unix ms.
    pub created_at: i64,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

impl Default for SshKey {
    fn default() -> Self {
        Self {
            name: String::new(),
            algorithm: KeyAlgorithm::default(),
            private_key: Zeroizing::new(String::new()),
            passphrase: None,
            public_key: String::new(),
            fingerprint: String::new(),
            comment: String::new(),
            created_at: 0,
            updated_at: 0,
        }
    }
}

impl fmt::Debug for SshKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshKey")
            .field("name", &self.name)
            .field("algorithm", &self.algorithm)
            .field("private_key", &"<redacted>")
            .field(
                "passphrase",
                &self.passphrase.as_ref().map(|_| "<redacted>"),
            )
            .field("fingerprint", &self.fingerprint)
            .field("updated_at", &self.updated_at)
            .finish_non_exhaustive()
    }
}

/// A trusted server host key (TOFU).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct KnownHost {
    /// Host name or address the key was seen on.
    pub host: String,
    pub port: u16,
    /// Key type, e.g. `ssh-ed25519`.
    pub key_type: String,
    pub public_key: String,
    /// `SHA256:...` fingerprint.
    pub fingerprint: String,
    /// First time the key was accepted, Unix ms.
    pub first_seen_at: i64,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

/// Direction of a [`PortForward`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForwardKind {
    /// `-L`.
    #[default]
    Local,
    /// `-R`.
    Remote,
    /// `-D` (SOCKS).
    Dynamic,
}

/// A saved port forward (P1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct PortForward {
    /// The [`Host`] this forward belongs to.
    pub host_id: String,
    /// What the forward is for; empty when it has none, and it is named by its rule.
    pub name: String,
    #[zeroize(skip)]
    pub kind: ForwardKind,
    pub bind_address: String,
    pub bind_port: u16,
    /// Destination host (`None` for dynamic).
    pub dest_host: Option<String>,
    /// Destination port (`None` for dynamic).
    pub dest_port: Option<u16>,
    /// Start with the connection.
    pub auto_start: bool,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

/// A saved command snippet (P2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct Snippet {
    pub name: String,
    pub command: String,
    pub tags: Vec<String>,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}
