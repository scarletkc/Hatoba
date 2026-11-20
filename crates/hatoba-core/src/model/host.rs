use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::ssh::HostProxy;

/// How a host authenticates.
///
/// The password lives in a [`Zeroizing`] string, so it is wiped whenever the value is dropped.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostAuth {
    /// A saved password (secret).
    Password {
        /// The password. Never sent to the front-end.
        password: Zeroizing<String>,
    },
    /// A key from the key store.
    Key {
        /// Id of the [`SshKey`] item.
        key_id: String,
    },
    /// The system SSH agent (P1).
    Agent,
    /// Ask on every connection; nothing is stored.
    #[default]
    Ask,
}

impl HostAuth {
    /// Password authentication with the given password.
    #[must_use]
    pub fn password(password: impl Into<String>) -> Self {
        Self::Password {
            password: Zeroizing::new(password.into()),
        }
    }
}

impl fmt::Debug for HostAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Password { .. } => f.write_str("Password { password: <redacted> }"),
            Self::Key { key_id } => f.debug_struct("Key").field("key_id", key_id).finish(),
            Self::Agent => f.write_str("Agent"),
            Self::Ask => f.write_str("Ask"),
        }
    }
}

fn default_ssh_port() -> u16 {
    22
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct Host {
    /// Display name, e.g. `prod-api-tokyo`.
    pub name: String,
    /// Domain name or IP address.
    pub address: String,
    /// TCP port, default 22.
    pub port: u16,
    pub username: String,
    pub auth: HostAuth,
    /// Containing [`Group`], if any.
    pub group_id: Option<String>,
    pub tags: Vec<String>,
    pub favorite: bool,
    /// Place in the custom order of the hosts page (HOST-16), which Telnet hosts share: lower
    /// comes first. 0 until the host is placed, and then absent.
    #[serde(skip_serializing_if = "is_zero")]
    pub sort: i64,
    /// ProxyJump host (P1).
    pub jump_host_id: Option<String>,
    /// SOCKS5 or HTTP proxy for the TCP connection (SSH-13). With a jump host, the jump host's
    /// own choice applies instead. Absent while it is the device's default.
    #[serde(skip_serializing_if = "HostProxy::is_device_default")]
    pub proxy: HostProxy,
    pub note: String,
    /// What the AI assistant is told about this host in its system prompt (AI-37), at most
    /// [`MAX_HOST_AI_NOTES_BYTES`] bytes of UTF-8. Absent while empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ai_notes: String,
    /// Last modification, Unix ms. Drives last-writer-wins conflict resolution.
    pub updated_at: i64,
    /// Environment variables the terminal asks the server to set (SSH-14), like OpenSSH's
    /// `SetEnv`. Checked with [`check_host_env`]. Absent while empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<EnvVar>,
}

/// The longest [`Host::ai_notes`], in bytes of UTF-8 (AI-37). Notes this long fit an item beside
/// the most environment variables a host can have, unless much of the text needs escaping in JSON;
/// while sync is on, saving a host that does not fit [`MAX_ITEM_PLAINTEXT_BYTES`] is refused.
pub const MAX_HOST_AI_NOTES_BYTES: usize = 16 * 1024;

/// One of a host's environment variables (SSH-14).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct EnvVar {
    /// The name, such as `TZ`.
    pub name: String,
    /// The value, which may be empty.
    pub value: String,
}

impl EnvVar {
    /// A variable with the given name and value.
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// The most environment variables a host can have (SSH-14).
pub const MAX_HOST_ENV_VARS: usize = 64;
/// The longest environment variable name, in characters (SSH-14).
pub const MAX_ENV_NAME_CHARS: usize = 128;
/// The longest environment variable value, in characters (SSH-14).
pub const MAX_ENV_VALUE_CHARS: usize = 4_096;
/// The most UTF-8 bytes a host's variable names and values take together (SSH-14). With
/// control characters ruled out, their JSON stays well within [`MAX_ITEM_PLAINTEXT_BYTES`].
pub const MAX_HOST_ENV_BYTES: usize = 16 * 1024;

/// Why a host's environment variables can't be saved (SSH-14).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EnvVarError {
    /// More than [`MAX_HOST_ENV_VARS`].
    #[error("a host can have at most {MAX_HOST_ENV_VARS} environment variables")]
    TooMany,
    #[error("an environment variable needs a name")]
    NameMissing,
    /// The name has a character other than ASCII letters, digits and `_`, or starts with a digit.
    #[error(
        "{0:?} is not a valid variable name: use letters, digits and _, not starting with a digit"
    )]
    NameInvalid(String),
    /// The name is longer than [`MAX_ENV_NAME_CHARS`].
    #[error("variable names are limited to {MAX_ENV_NAME_CHARS} characters")]
    NameTooLong,
    /// Two variables have the same name (names are case-sensitive).
    #[error("{0} is set more than once")]
    NameDuplicate(String),
    /// The value has a control character other than tab, such as a line break.
    #[error("the value of {0} can't contain line breaks or other control characters")]
    ValueInvalid(String),
    /// The value is longer than [`MAX_ENV_VALUE_CHARS`].
    #[error("the value of {0} is longer than {MAX_ENV_VALUE_CHARS} characters")]
    ValueTooLong(String),
    /// The names and values take more than [`MAX_HOST_ENV_BYTES`] together.
    #[error("the environment variables take more than {MAX_HOST_ENV_BYTES} bytes together")]
    TooLarge,
}

/// Checks one variable: a name of ASCII letters, digits and `_` that does not start with a digit,
/// and a value without control characters other than tab, each within its limit.
pub fn check_env_var(var: &EnvVar) -> std::result::Result<(), EnvVarError> {
    let name = var.name.as_str();
    if name.is_empty() {
        return Err(EnvVarError::NameMissing);
    }
    if name.chars().count() > MAX_ENV_NAME_CHARS {
        return Err(EnvVarError::NameTooLong);
    }
    let valid = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !valid {
        return Err(EnvVarError::NameInvalid(name.to_owned()));
    }
    if var.value.chars().any(|c| c.is_control() && c != '\t') {
        return Err(EnvVarError::ValueInvalid(name.to_owned()));
    }
    if var.value.chars().count() > MAX_ENV_VALUE_CHARS {
        return Err(EnvVarError::ValueTooLong(name.to_owned()));
    }
    Ok(())
}

/// Checks a host's variables: each as in [`check_env_var`], no name twice, at most
/// [`MAX_HOST_ENV_VARS`] of them, and at most [`MAX_HOST_ENV_BYTES`] together.
pub fn check_host_env(vars: &[EnvVar]) -> std::result::Result<(), EnvVarError> {
    if vars.len() > MAX_HOST_ENV_VARS {
        return Err(EnvVarError::TooMany);
    }
    for (i, var) in vars.iter().enumerate() {
        check_env_var(var)?;
        if vars[..i].iter().any(|v| v.name == var.name) {
            return Err(EnvVarError::NameDuplicate(var.name.clone()));
        }
    }
    if env_bytes(vars) > MAX_HOST_ENV_BYTES {
        return Err(EnvVarError::TooLarge);
    }
    Ok(())
}

/// The UTF-8 bytes of the names and values together, as [`MAX_HOST_ENV_BYTES`] counts them.
#[must_use]
pub fn env_bytes(vars: &[EnvVar]) -> usize {
    vars.iter().map(|v| v.name.len() + v.value.len()).sum()
}

impl Default for Host {
    fn default() -> Self {
        Self {
            name: String::new(),
            address: String::new(),
            port: default_ssh_port(),
            username: String::new(),
            auth: HostAuth::default(),
            group_id: None,
            tags: Vec::new(),
            favorite: false,
            sort: 0,
            jump_host_id: None,
            proxy: HostProxy::DeviceDefault,
            note: String::new(),
            ai_notes: String::new(),
            updated_at: 0,
            env: Vec::new(),
        }
    }
}

/// A Telnet host (TELNET-01): a switch, router or embedded device that offers Telnet. It is an
/// item type of its own rather than a [`Host`] with a protocol, so an app without Telnet skips it
/// instead of reading it as an SSH host and dropping the protocol when it saves it. The password
/// is wiped on drop and never printed.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct TelnetHost {
    /// Display name, e.g. `core-switch-1`.
    pub name: String,
    /// Domain name or IP address.
    pub address: String,
    /// TCP port, default 23.
    pub port: u16,
    /// The user name typed at the server's login prompt (TELNET-03); empty to type it by hand.
    pub username: String,
    /// The password typed at the server's password prompt (secret); empty to type it by hand.
    /// Never sent to the front-end. Absent while empty.
    #[serde(skip_serializing_if = "zeroizing_is_empty")]
    pub password: Zeroizing<String>,
    /// Containing [`Group`], if any.
    pub group_id: Option<String>,
    pub tags: Vec<String>,
    pub favorite: bool,
    /// Place in the custom order of the hosts page, as for SSH hosts (HOST-16).
    #[serde(skip_serializing_if = "is_zero")]
    pub sort: i64,
    /// SOCKS5 or HTTP proxy for the TCP connection, as for SSH hosts (SSH-13). Absent while it is
    /// the device's default.
    #[serde(skip_serializing_if = "HostProxy::is_device_default")]
    pub proxy: HostProxy,
    pub note: String,
    /// What the AI assistant is told about this host (AI-37), at most
    /// [`MAX_HOST_AI_NOTES_BYTES`] bytes of UTF-8. Absent while empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ai_notes: String,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

pub const TELNET_PORT: u16 = 23;

fn zeroizing_is_empty(s: &Zeroizing<String>) -> bool {
    s.is_empty()
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

impl Default for TelnetHost {
    fn default() -> Self {
        Self {
            name: String::new(),
            address: String::new(),
            port: TELNET_PORT,
            username: String::new(),
            password: Zeroizing::default(),
            group_id: None,
            tags: Vec::new(),
            favorite: false,
            sort: 0,
            proxy: HostProxy::DeviceDefault,
            note: String::new(),
            ai_notes: String::new(),
            updated_at: 0,
        }
    }
}

impl fmt::Debug for TelnetHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TelnetHost")
            .field("name", &self.name)
            .field("address", &self.address)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("group_id", &self.group_id)
            .field("tags", &self.tags)
            .field("favorite", &self.favorite)
            .field("proxy", &self.proxy)
            .field("note", &self.note)
            .field("ai_notes", &self.ai_notes)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}
