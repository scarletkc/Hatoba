//! Plaintext item structures (spec §5.1).
//!
//! These are what gets JSON-serialised, sealed in an [`Envelope`](crate::crypto::Envelope) and
//! synced. The `type` discriminator lives *inside* the encrypted JSON only; no store ever sees it
//! in the clear.
//!
//! Every struct tolerates unknown fields and missing fields (`#[serde(default)]`) so devices on
//! different app versions can read each other's data. Secret-bearing types implement `Debug`
//! by hand so they can never leak into logs, and wipe themselves when dropped.
//!
//! The AI assistant adds seven item types (spec §5.1, §13): [`AiProvider`] and [`SearchProvider`]
//! hold API keys, [`McpServer`] holds environment and header values, and [`AiMessage`] holds a
//! slice of conversation content (SEC-04). All four have a redacting `Debug`. An `ai_message`'s
//! `data` is only held while an entry is being read or written: the in-memory item map keeps the
//! header alone (see [`Item::strip_message_data`]).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::error::{Error, Result};

/// Fixed id of the single [`Settings`] item.
pub const SETTINGS_ID: &str = "settings";

/// A fresh UUIDv7 item id (time-ordered, so the item map iterates in creation order).
#[must_use]
pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

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

/// An SSH host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct Host {
    /// Display name, e.g. `prod-api-tokyo`.
    pub name: String,
    /// Domain name or IP address.
    pub address: String,
    /// TCP port, default 22.
    pub port: u16,
    /// Login user.
    pub username: String,
    /// Authentication method.
    pub auth: HostAuth,
    /// Containing [`Group`], if any.
    pub group_id: Option<String>,
    /// Free-form tags.
    pub tags: Vec<String>,
    /// Favourite flag.
    pub favorite: bool,
    /// ProxyJump host (P1).
    pub jump_host_id: Option<String>,
    /// Free-form note.
    pub note: String,
    /// What the AI assistant is told about this host in its system prompt (AI-37), at most
    /// [`MAX_HOST_AI_NOTES_CHARS`] characters. Absent while empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub ai_notes: String,
    /// Last modification, Unix ms. Drives last-writer-wins conflict resolution.
    pub updated_at: i64,
    /// Environment variables the terminal asks the server to set (SSH-14), like OpenSSH's
    /// `SetEnv`. Checked with [`check_host_env`]. Absent while empty.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<EnvVar>,
}

/// The longest [`Host::ai_notes`], in characters (AI-37).
pub const MAX_HOST_AI_NOTES_CHARS: usize = 2_000;

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
    /// The name is empty.
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

/// The most plaintext bytes (the item's JSON) an item may hold, so that its envelope stays under
/// the 64 KiB that the sync Worker and D1 accept (§6.2); the same budget as
/// [`AI_PART_MAX_BYTES`](crate::AI_PART_MAX_BYTES). Larger items are refused when saved.
pub const MAX_ITEM_PLAINTEXT_BYTES: usize = 40 * 1024;

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
            jump_host_id: None,
            note: String::new(),
            ai_notes: String::new(),
            updated_at: 0,
            env: Vec::new(),
        }
    }
}

/// A host group (one level of nesting in the MVP).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct Group {
    /// Display name.
    pub name: String,
    /// Parent group.
    pub parent_id: Option<String>,
    /// Sort position.
    pub sort: i64,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

/// Key algorithm of an [`SshKey`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAlgorithm {
    /// Ed25519.
    #[default]
    Ed25519,
    /// ECDSA.
    Ecdsa,
    /// RSA.
    Rsa,
}

/// A private key in the key store. The private key and passphrase are wiped on drop.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct SshKey {
    /// Display name.
    pub name: String,
    /// Key algorithm.
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
    /// Key comment.
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
    /// Port.
    pub port: u16,
    /// Key type, e.g. `ssh-ed25519`.
    pub key_type: String,
    /// Public key.
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
    /// Direction.
    #[zeroize(skip)]
    pub kind: ForwardKind,
    /// Address to bind.
    pub bind_address: String,
    /// Port to bind.
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
    /// Display name.
    pub name: String,
    /// The command text.
    pub command: String,
    /// Tags.
    pub tags: Vec<String>,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

/// Wire protocol of an [`AiProvider`] (spec §13.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProtocol {
    /// Anthropic Messages (`/v1/messages`).
    Anthropic,
    /// OpenAI-style Chat Completions. Also what a missing or unknown future value decodes to
    /// (`serde(other)` must be last).
    #[default]
    #[serde(other)]
    ChatCompletions,
}

impl AiProtocol {
    /// The `protocol` string as it appears in the plaintext JSON.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::ChatCompletions => "chat_completions",
        }
    }
}

/// Which header carries an Anthropic provider's key (spec §13.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AiAuthHeader {
    /// `Authorization: Bearer <key>`.
    Authorization,
    /// `x-api-key: <key>`. Also what a missing or unknown future value decodes to.
    #[default]
    #[serde(other)]
    XApiKey,
}

impl AiAuthHeader {
    /// The `auth_header` string as it appears in the plaintext JSON.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Authorization => "authorization",
            Self::XApiKey => "x-api-key",
        }
    }
}

/// A thinking level (AI-05), lowest first. Where one is optional, `None` is Default: the request
/// leaves the depth to the provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiEffort {
    /// `low`.
    Low,
    /// `medium`.
    Medium,
    /// `high`.
    High,
    /// `xhigh` (Extra High).
    Xhigh,
    /// `max`.
    Max,
}

impl AiEffort {
    /// Every level, lowest first.
    pub const ALL: [Self; 5] = [Self::Low, Self::Medium, Self::High, Self::Xhigh, Self::Max];

    /// The string as it appears in the plaintext JSON.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.as_str() == text)
    }
}

/// Lenient readers for thinking levels: a level a newer version added reads as unknown instead of
/// making the whole item unreadable.
mod effort_serde {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    use super::AiEffort;

    /// An optional level; anything that is not a known level reads as `None` (Default).
    pub(super) fn level<'de, D: Deserializer<'de>>(d: D) -> Result<Option<AiEffort>, D::Error> {
        let value = Option::<Value>::deserialize(d)?;
        Ok(value
            .as_ref()
            .and_then(Value::as_str)
            .and_then(AiEffort::parse))
    }

    /// A model's levels, lowest first, without the ones this version does not know; `None`
    /// (unknown) when the value is not a list.
    pub(super) fn levels<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Vec<AiEffort>>, D::Error> {
        let Some(Value::Array(items)) = Option::<Value>::deserialize(d)? else {
            return Ok(None);
        };
        let mut levels: Vec<AiEffort> = items
            .iter()
            .filter_map(|v| v.as_str().and_then(AiEffort::parse))
            .collect();
        levels.sort_unstable();
        levels.dedup();
        Ok(Some(levels))
    }
}

/// A model offered by an [`AiProvider`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct AiModel {
    /// Model id sent in requests.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Context window in tokens, `None` when unknown.
    pub context_window: Option<u64>,
    /// Output limit in tokens, `None` when unknown.
    pub max_output_tokens: Option<u64>,
    /// The thinking levels the model accepts, lowest first (AI-05): from Anthropic's model list
    /// or set by the user, empty when it accepts none, `None` (absent) when unknown.
    #[zeroize(skip)]
    #[serde(
        deserialize_with = "effort_serde::levels",
        skip_serializing_if = "Option::is_none"
    )]
    pub efforts: Option<Vec<AiEffort>>,
    /// Whether the model supports adaptive thinking, from Anthropic's model list; `None`
    /// (absent) when unknown.
    #[zeroize(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptive_thinking: Option<bool>,
}

/// An AI model provider (P1, spec §13.2). The API key is wiped on drop and never printed.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct AiProvider {
    /// Display name.
    pub name: String,
    /// Wire protocol.
    #[zeroize(skip)]
    pub protocol: AiProtocol,
    /// Base URL, e.g. `https://api.openai.com/v1` or `https://api.anthropic.com`.
    pub base_url: String,
    /// The API key (secret); empty when the server needs none. Never sent to the front-end.
    pub api_key: Zeroizing<String>,
    /// Header that carries the key (Anthropic protocol only).
    #[zeroize(skip)]
    pub auth_header: AiAuthHeader,
    /// Models the user can pick.
    pub models: Vec<AiModel>,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

impl fmt::Debug for AiProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiProvider")
            .field("name", &self.name)
            .field("protocol", &self.protocol)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("auth_header", &self.auth_header)
            .field("models", &self.models)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// Which web search service a [`SearchProvider`] talks to (spec §13.4).
///
/// Unlike the AI enums, an unknown future value is not mapped to a default: sending the key to
/// the wrong service would be worse than leaving the item unreadable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    /// Brave Search API.
    #[default]
    Brave,
    /// Tavily.
    Tavily,
    /// A SearXNG instance.
    Searxng,
}

impl SearchKind {
    /// The `kind` string as it appears in the plaintext JSON.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Brave => "brave",
            Self::Tavily => "tavily",
            Self::Searxng => "searxng",
        }
    }
}

/// The backend of the `web_search` tool (P1, spec §13.4). The API key is wiped on drop and never
/// printed.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct SearchProvider {
    /// Search service.
    #[zeroize(skip)]
    pub kind: SearchKind,
    /// The instance URL for SearXNG, `None` otherwise.
    pub base_url: Option<String>,
    /// The API key (secret); empty for SearXNG.
    pub api_key: Zeroizing<String>,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

impl fmt::Debug for SearchProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SearchProvider")
            .field("kind", &self.kind)
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// An AI conversation (P1, spec §13.7). Its entries live in [`AiMessage`] items.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct AiConversation {
    /// Title; starts as the first line of the first message, cut to 60 characters.
    pub title: String,
    /// The host the conversation last worked on.
    pub host_id: Option<String>,
    /// Pinned to the top of the history.
    pub pinned: bool,
    /// `entry_id` where the context sent to the model starts (AI-21).
    pub context_start: Option<String>,
    /// The thinking level its last message was sent with (AI-05); `None` (absent) is Default,
    /// which is also what conversations from before the level existed read as.
    #[zeroize(skip)]
    #[serde(
        deserialize_with = "effort_serde::level",
        skip_serializing_if = "Option::is_none"
    )]
    pub effort: Option<AiEffort>,
    /// Creation time, Unix ms.
    pub created_at: i64,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

/// One part of a conversation entry (P1, spec §13.7). Entries are written once and never change,
/// so they never conflict; they are ordered by `entry_id`.
///
/// `data` is this part's slice of the entry's JSON, so it is conversation content (SEC-04): it is
/// not printed by `Debug` and not kept in the item map (see [`Item::strip_message_data`]).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct AiMessage {
    /// The [`AiConversation`] the entry belongs to.
    pub conversation_id: String,
    /// UUIDv7 that orders the entries of a conversation.
    pub entry_id: String,
    /// 0-based index of this part.
    pub part: u32,
    /// Number of parts of the entry.
    pub part_count: u32,
    /// This part's slice of the entry's JSON (conversation content).
    pub data: String,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

impl fmt::Debug for AiMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiMessage")
            .field("conversation_id", &self.conversation_id)
            .field("entry_id", &self.entry_id)
            .field("part", &self.part)
            .field("part_count", &self.part_count)
            .field(
                "data",
                &format_args!("<{} bytes redacted>", self.data.len()),
            )
            .field("updated_at", &self.updated_at)
            .finish()
    }
}

/// A skill in the Agent Skills format (P2, spec §13.8); its text lives in [`SkillFile`] items.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct Skill {
    /// 1 to 64 lowercase letters, digits and hyphens.
    pub name: String,
    /// At most 1,024 characters.
    pub description: String,
    /// Other `SKILL.md` frontmatter fields, kept for export.
    #[zeroize(skip)]
    pub frontmatter: serde_json::Map<String, serde_json::Value>,
    /// Offered to the assistant.
    pub enabled: bool,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

/// One text file of a [`Skill`] (P2, spec §13.8).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct SkillFile {
    /// The [`Skill`] it belongs to.
    pub skill_id: String,
    /// `SKILL.md` (its body, without frontmatter) or a relative path such as `references/nginx.md`.
    pub path: String,
    /// UTF-8 text, at most 32 KB.
    pub content: String,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

/// How an [`McpServer`] is reached (spec §13.9). The environment and header values are secrets:
/// they are wiped on drop and never printed.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum McpTransport {
    /// A child process started on this device.
    Stdio {
        /// Executable, looked up on `PATH`.
        #[serde(default)]
        command: String,
        /// Arguments.
        #[serde(default)]
        args: Vec<String>,
        /// Environment variables (values are secrets).
        #[serde(default)]
        env: BTreeMap<String, Zeroizing<String>>,
    },
    /// A Streamable HTTP endpoint.
    Http {
        /// Endpoint URL.
        #[serde(default)]
        url: String,
        /// Request headers (values are secrets).
        #[serde(default)]
        headers: BTreeMap<String, Zeroizing<String>>,
    },
}

impl Default for McpTransport {
    fn default() -> Self {
        Self::Stdio {
            command: String::new(),
            args: Vec::new(),
            env: BTreeMap::new(),
        }
    }
}

impl McpTransport {
    /// The `kind` string as it appears in the plaintext JSON.
    #[must_use]
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Stdio { .. } => "stdio",
            Self::Http { .. } => "http",
        }
    }
}

impl Zeroize for McpTransport {
    fn zeroize(&mut self) {
        // `zeroize` has no impl for maps: taking the map out and dropping its entries one by one
        // wipes every `Zeroizing` value.
        fn wipe(map: &mut BTreeMap<String, Zeroizing<String>>) {
            for (mut name, value) in std::mem::take(map) {
                name.zeroize();
                drop(value);
            }
        }
        match self {
            Self::Stdio { command, args, env } => {
                command.zeroize();
                args.zeroize();
                wipe(env);
            }
            Self::Http { url, headers } => {
                url.zeroize();
                wipe(headers);
            }
        }
    }
}

impl fmt::Debug for McpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Names only: the values are what is secret.
        fn redacted(map: &BTreeMap<String, Zeroizing<String>>) -> Vec<(&String, &'static str)> {
            map.keys().map(|k| (k, "<redacted>")).collect()
        }
        match self {
            Self::Stdio { command, args, env } => f
                .debug_struct("Stdio")
                .field("command", command)
                .field("args", args)
                .field("env", &redacted(env))
                .finish(),
            Self::Http { url, headers } => f
                .debug_struct("Http")
                .field("url", url)
                .field("headers", &redacted(headers))
                .finish(),
        }
    }
}

/// An MCP server that adds tools to the assistant (P2, spec §13.9).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct McpServer {
    /// Display name; used in tool names.
    pub name: String,
    /// How to reach it.
    pub transport: McpTransport,
    /// Ask before every call, even in bypass mode (AI-31).
    pub always_ask: bool,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

/// Terminal colour theme preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    /// Light.
    Light,
    /// Dark.
    Dark,
    /// Follow the OS. Also what an unknown future value decodes to (`serde(other)` must be last).
    #[default]
    #[serde(other)]
    System,
}

/// Terminal cursor shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CursorStyle {
    /// Vertical bar.
    Bar,
    /// Underline.
    Underline,
    /// Filled block. Also what an unknown future value decodes to.
    #[default]
    #[serde(other)]
    Block,
}

/// What right-clicking in the terminal does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RightClick {
    /// Open a context menu.
    Menu,
    /// Copy the selection if there is one, otherwise paste (the PuTTY habit). Also what an
    /// unknown future value decodes to.
    #[default]
    #[serde(other)]
    CopyPaste,
}

/// Terminal appearance and behaviour settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct TerminalSettings {
    /// Font family.
    pub font_family: String,
    /// Font size in points.
    pub font_size: u16,
    /// Colour theme.
    #[zeroize(skip)]
    pub theme: ThemeMode,
    /// Cursor shape.
    #[zeroize(skip)]
    pub cursor_style: CursorStyle,
    /// Scrollback lines.
    pub scrollback: u32,
    /// Right-click behaviour. `None` until a value is recorded, which items written by builds
    /// that kept it device-local never have; read it with [`Self::right_click`].
    #[zeroize(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right_click: Option<RightClick>,
    /// Ask before pasting text with line breaks. `None` until a value is recorded; read it with
    /// [`Self::confirm_multiline_paste`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm_multiline_paste: Option<bool>,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_family: "Cascadia Mono".to_owned(),
            font_size: 13,
            theme: ThemeMode::Dark,
            cursor_style: CursorStyle::Block,
            scrollback: 10_000,
            right_click: None,
            confirm_multiline_paste: None,
        }
    }
}

impl TerminalSettings {
    /// The right-click behaviour; copy/paste while unset.
    #[must_use]
    pub fn right_click(&self) -> RightClick {
        self.right_click.unwrap_or_default()
    }

    /// Whether multi-line pastes need confirming; `true` while unset.
    #[must_use]
    pub fn confirm_multiline_paste(&self) -> bool {
        self.confirm_multiline_paste.unwrap_or(true)
    }

    /// Records the right-click behaviour. An unset field stays unset when `value` is the default.
    pub fn set_right_click(&mut self, value: RightClick) {
        record(&mut self.right_click, value, RightClick::default());
    }

    /// Records whether multi-line pastes need confirming. An unset field stays unset when `value`
    /// is the default.
    pub fn set_confirm_multiline_paste(&mut self, value: bool) {
        record(&mut self.confirm_multiline_paste, value, true);
    }
}

/// Stores `value` in a field that reads as `default` while unset, except that an unset field
/// stays unset when `value` is that default: the user has not chosen anything yet, and leaving
/// it open lets a device carrying over a different device-local value fill it in.
fn record<T: PartialEq>(field: &mut Option<T>, value: T, default: T) {
    if field.is_some() || value != default {
        *field = Some(value);
    }
}

/// A model picked by provider and model id.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct AiModelRef {
    /// Id of the [`AiProvider`] item.
    pub provider_id: String,
    /// Model id within that provider.
    pub model_id: String,
}

/// AI assistant settings that sync (spec §5.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct AiSettings {
    /// The model new conversations start with.
    pub default_model: Option<AiModelRef>,
    /// The thinking level new conversations start with (AI-05); `None` (absent) is Default.
    #[zeroize(skip)]
    #[serde(
        deserialize_with = "effort_serde::level",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_effort: Option<AiEffort>,
    /// Id of the [`SearchProvider`] item behind `web_search`.
    pub search_provider_id: Option<String>,
    /// The built-in `hatoba` skill is offered (AI-34). On by default, also for settings written
    /// before it existed.
    pub builtin_skill_enabled: bool,
    /// The user's instructions, sent with every request (AI-36), at most
    /// [`MAX_CUSTOM_INSTRUCTIONS_CHARS`] characters. Absent while empty.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub custom_instructions: String,
}

/// The longest [`AiSettings::custom_instructions`], in characters (AI-36).
pub const MAX_CUSTOM_INSTRUCTIONS_CHARS: usize = 4_000;

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            default_model: None,
            default_effort: None,
            search_provider_id: None,
            builtin_skill_enabled: true,
            custom_instructions: String::new(),
        }
    }
}

/// The user's synced settings. There is exactly one, with the fixed id [`SETTINGS_ID`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct Settings {
    /// Terminal appearance.
    pub terminal: TerminalSettings,
    /// Idle minutes before auto-lock (0 disables).
    pub auto_lock_minutes: u32,
    /// Disconnect SSH sessions when the vault locks.
    pub lock_disconnects_sessions: bool,
    /// AI assistant settings.
    pub ai: AiSettings,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            terminal: TerminalSettings::default(),
            auto_lock_minutes: 15,
            lock_disconnects_sessions: false,
            ai: AiSettings::default(),
            updated_at: 0,
        }
    }
}

/// Any syncable item. Serialises with `"type": "host" | "group" | "key" | "known_host" |
/// "forward" | "snippet" | "ai_provider" | "search_provider" | "ai_conversation" | "ai_message" |
/// "skill" | "skill_file" | "mcp_server" | "settings"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Item {
    /// An SSH host.
    Host(Host),
    /// A host group.
    Group(Group),
    /// A private key.
    Key(SshKey),
    /// A trusted host key.
    KnownHost(KnownHost),
    /// A saved port forward.
    Forward(PortForward),
    /// A command snippet.
    Snippet(Snippet),
    /// An AI model provider.
    AiProvider(AiProvider),
    /// The web search backend.
    SearchProvider(SearchProvider),
    /// An AI conversation.
    AiConversation(AiConversation),
    /// One part of a conversation entry.
    AiMessage(AiMessage),
    /// An assistant skill.
    Skill(Skill),
    /// A text file of a skill.
    SkillFile(SkillFile),
    /// An MCP server.
    McpServer(McpServer),
    /// The settings singleton.
    Settings(Settings),
}

impl Item {
    /// The `type` string as it appears in the plaintext JSON.
    #[must_use]
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Host(_) => "host",
            Self::Group(_) => "group",
            Self::Key(_) => "key",
            Self::KnownHost(_) => "known_host",
            Self::Forward(_) => "forward",
            Self::Snippet(_) => "snippet",
            Self::AiProvider(_) => "ai_provider",
            Self::SearchProvider(_) => "search_provider",
            Self::AiConversation(_) => "ai_conversation",
            Self::AiMessage(_) => "ai_message",
            Self::Skill(_) => "skill",
            Self::SkillFile(_) => "skill_file",
            Self::McpServer(_) => "mcp_server",
            Self::Settings(_) => "settings",
        }
    }

    /// Last modification time (Unix ms).
    #[must_use]
    pub fn updated_at(&self) -> i64 {
        match self {
            Self::Host(i) => i.updated_at,
            Self::Group(i) => i.updated_at,
            Self::Key(i) => i.updated_at,
            Self::KnownHost(i) => i.updated_at,
            Self::Forward(i) => i.updated_at,
            Self::Snippet(i) => i.updated_at,
            Self::AiProvider(i) => i.updated_at,
            Self::SearchProvider(i) => i.updated_at,
            Self::AiConversation(i) => i.updated_at,
            Self::AiMessage(i) => i.updated_at,
            Self::Skill(i) => i.updated_at,
            Self::SkillFile(i) => i.updated_at,
            Self::McpServer(i) => i.updated_at,
            Self::Settings(i) => i.updated_at,
        }
    }

    /// Sets the modification time (Unix ms).
    pub fn set_updated_at(&mut self, at: i64) {
        match self {
            Self::Host(i) => i.updated_at = at,
            Self::Group(i) => i.updated_at = at,
            Self::Key(i) => i.updated_at = at,
            Self::KnownHost(i) => i.updated_at = at,
            Self::Forward(i) => i.updated_at = at,
            Self::Snippet(i) => i.updated_at = at,
            Self::AiProvider(i) => i.updated_at = at,
            Self::SearchProvider(i) => i.updated_at = at,
            Self::AiConversation(i) => i.updated_at = at,
            Self::AiMessage(i) => i.updated_at = at,
            Self::Skill(i) => i.updated_at = at,
            Self::SkillFile(i) => i.updated_at = at,
            Self::McpServer(i) => i.updated_at = at,
            Self::Settings(i) => i.updated_at = at,
        }
    }

    /// A human-readable label for lists and conflict screens. Never a secret.
    #[must_use]
    pub fn display_name(&self) -> String {
        match self {
            Self::Host(i) => i.name.clone(),
            Self::Group(i) => i.name.clone(),
            Self::Key(i) => i.name.clone(),
            Self::KnownHost(i) => format!("{}:{}", i.host, i.port),
            Self::Forward(i) => format!("{}:{}", i.bind_address, i.bind_port),
            Self::Snippet(i) => i.name.clone(),
            Self::AiProvider(i) => i.name.clone(),
            Self::SearchProvider(i) => i.kind.as_str().to_owned(),
            Self::AiConversation(i) => i.title.clone(),
            Self::AiMessage(_) => "message".to_owned(),
            Self::Skill(i) => i.name.clone(),
            Self::SkillFile(i) => i.path.clone(),
            Self::McpServer(i) => i.name.clone(),
            Self::Settings(_) => "settings".to_owned(),
        }
    }

    /// Whether this is an SSH key (keys are never silently dropped by conflict resolution).
    #[must_use]
    pub fn is_key(&self) -> bool {
        matches!(self, Self::Key(_))
    }

    /// The host, if this is one.
    #[must_use]
    pub fn as_host(&self) -> Option<&Host> {
        if let Self::Host(h) = self {
            Some(h)
        } else {
            None
        }
    }

    /// The group, if this is one.
    #[must_use]
    pub fn as_group(&self) -> Option<&Group> {
        if let Self::Group(g) = self {
            Some(g)
        } else {
            None
        }
    }

    /// The key, if this is one.
    #[must_use]
    pub fn as_key(&self) -> Option<&SshKey> {
        if let Self::Key(k) = self {
            Some(k)
        } else {
            None
        }
    }

    /// The known host, if this is one.
    #[must_use]
    pub fn as_known_host(&self) -> Option<&KnownHost> {
        if let Self::KnownHost(k) = self {
            Some(k)
        } else {
            None
        }
    }

    /// The port forward, if this is one.
    #[must_use]
    pub fn as_forward(&self) -> Option<&PortForward> {
        if let Self::Forward(f) = self {
            Some(f)
        } else {
            None
        }
    }

    /// The AI provider, if this is one.
    #[must_use]
    pub fn as_ai_provider(&self) -> Option<&AiProvider> {
        if let Self::AiProvider(p) = self {
            Some(p)
        } else {
            None
        }
    }

    /// The search provider, if this is one.
    #[must_use]
    pub fn as_search_provider(&self) -> Option<&SearchProvider> {
        if let Self::SearchProvider(p) = self {
            Some(p)
        } else {
            None
        }
    }

    /// The AI conversation, if this is one.
    #[must_use]
    pub fn as_ai_conversation(&self) -> Option<&AiConversation> {
        if let Self::AiConversation(c) = self {
            Some(c)
        } else {
            None
        }
    }

    /// The conversation message part, if this is one.
    #[must_use]
    pub fn as_ai_message(&self) -> Option<&AiMessage> {
        if let Self::AiMessage(m) = self {
            Some(m)
        } else {
            None
        }
    }

    /// The skill, if this is one.
    #[must_use]
    pub fn as_skill(&self) -> Option<&Skill> {
        if let Self::Skill(s) = self {
            Some(s)
        } else {
            None
        }
    }

    /// The skill file, if this is one.
    #[must_use]
    pub fn as_skill_file(&self) -> Option<&SkillFile> {
        if let Self::SkillFile(f) = self {
            Some(f)
        } else {
            None
        }
    }

    /// The MCP server, if this is one.
    #[must_use]
    pub fn as_mcp_server(&self) -> Option<&McpServer> {
        if let Self::McpServer(m) = self {
            Some(m)
        } else {
            None
        }
    }

    /// The settings, if this is the settings item.
    #[must_use]
    pub fn as_settings(&self) -> Option<&Settings> {
        if let Self::Settings(s) = self {
            Some(s)
        } else {
            None
        }
    }

    /// Returns a copy whose name carries `suffix` (used for conflict copies). Types without a
    /// name field are returned unchanged.
    #[must_use]
    pub fn with_name_suffix(&self, suffix: &str) -> Self {
        let mut copy = self.clone();
        match &mut copy {
            Self::Host(i) => i.name.push_str(suffix),
            Self::Group(i) => i.name.push_str(suffix),
            Self::Key(i) => i.name.push_str(suffix),
            Self::Snippet(i) => i.name.push_str(suffix),
            Self::AiProvider(i) => i.name.push_str(suffix),
            Self::Skill(i) => i.name.push_str(suffix),
            Self::McpServer(i) => i.name.push_str(suffix),
            Self::KnownHost(_)
            | Self::Forward(_)
            | Self::SearchProvider(_)
            | Self::AiConversation(_)
            | Self::AiMessage(_)
            | Self::SkillFile(_)
            | Self::Settings(_) => {}
        }
        copy
    }

    /// Drops the `data` of an `ai_message`, wiping it first: the item map keeps only the header
    /// of a message part, because a long history must not stay in memory (spec §13.7). Items of
    /// every other type are left alone.
    pub fn strip_message_data(&mut self) {
        if let Self::AiMessage(m) = self {
            std::mem::take(&mut m.data).zeroize();
        }
    }

    /// The length of [`to_plaintext`](Self::to_plaintext), measured without building it.
    ///
    /// # Errors
    /// [`Error::Json`] if serialisation fails (cannot happen for these types in practice).
    pub fn plaintext_len(&self) -> Result<usize> {
        let mut counter = ByteCounter(0);
        serde_json::to_writer(&mut counter, self)?;
        Ok(counter.0)
    }

    /// Serialises to the plaintext JSON that gets sealed. The buffer is wiped on drop.
    ///
    /// # Errors
    /// [`Error::Json`] if serialisation fails (cannot happen for these types in practice).
    pub fn to_plaintext(&self) -> Result<Zeroizing<Vec<u8>>> {
        // Pre-size so typical items (even a 4096-bit RSA key) never reallocate, which would
        // leave an un-wiped copy of the plaintext in freed memory. A message part can be larger
        // than that and holds conversation content, so its exact size is measured first.
        let capacity = match self {
            Self::AiMessage(_) => {
                let mut counter = ByteCounter(0);
                serde_json::to_writer(&mut counter, self)?;
                counter.0
            }
            _ => 16 * 1024,
        };
        let mut buf = Zeroizing::new(Vec::with_capacity(capacity));
        serde_json::to_writer(&mut *buf, self)?;
        Ok(buf)
    }

    /// Parses plaintext JSON.
    ///
    /// # Errors
    /// [`Error::Format`] for invalid JSON or an unknown `type` (e.g. written by a newer app).
    pub fn from_plaintext(bytes: &[u8]) -> Result<Self> {
        serde_json::from_slice(bytes).map_err(|_| Error::Format("unreadable item".into()))
    }
}

/// An [`io::Write`](std::io::Write) that only counts.
struct ByteCounter(usize);

impl std::io::Write for ByteCounter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_host() -> Host {
        Host {
            name: "prod-api-tokyo".into(),
            address: "10.0.0.7".into(),
            username: "deploy".into(),
            auth: HostAuth::password("hunter2"),
            tags: vec!["production".into(), "tokyo".into()],
            favorite: true,
            updated_at: 1_700_000_000_000,
            ..Host::default()
        }
    }

    #[test]
    fn host_json_matches_the_spec_shape() {
        let value = serde_json::to_value(Item::Host(sample_host())).unwrap();
        assert_eq!(
            value,
            json!({
                "type": "host",
                "name": "prod-api-tokyo",
                "address": "10.0.0.7",
                "port": 22,
                "username": "deploy",
                "auth": { "kind": "password", "password": "hunter2" },
                "group_id": null,
                "tags": ["production", "tokyo"],
                "favorite": true,
                "jump_host_id": null,
                "note": "",
                "updated_at": 1_700_000_000_000_i64
            })
        );

        // AI-37: the AI notes are written only when there are some, and read back.
        let host = Host {
            ai_notes: "Debian 12; the app lives in /srv/api.".into(),
            ..sample_host()
        };
        let value = serde_json::to_value(Item::Host(host.clone())).unwrap();
        assert_eq!(value["ai_notes"], "Debian 12; the app lives in /srv/api.");
        let back = Item::from_plaintext(&Item::Host(host.clone()).to_plaintext().unwrap()).unwrap();
        assert_eq!(back, Item::Host(host));
        let old: Item = serde_json::from_value(json!({"type": "host", "name": "old"})).unwrap();
        assert_eq!(old.as_host().unwrap().ai_notes, "");
    }

    #[test]
    fn host_env_is_written_only_when_set_and_read_back() {
        // SSH-14: a host without variables keeps the old shape, so older versions read it as before.
        let value = serde_json::to_value(Item::Host(sample_host())).unwrap();
        assert!(value.get("env").is_none());

        let host = Host {
            env: vec![EnvVar::new("TZ", "Asia/Tokyo"), EnvVar::new("EMPTY", "")],
            ..sample_host()
        };
        let value = serde_json::to_value(Item::Host(host.clone())).unwrap();
        assert_eq!(
            value["env"],
            json!([{"name": "TZ", "value": "Asia/Tokyo"}, {"name": "EMPTY", "value": ""}])
        );
        let back = Item::from_plaintext(&Item::Host(host.clone()).to_plaintext().unwrap()).unwrap();
        assert_eq!(back, Item::Host(host));

        // An item from before SSH-14 has no variables; a newer one with extra fields still reads.
        let old: Item = serde_json::from_value(json!({"type": "host", "name": "old"})).unwrap();
        assert!(old.as_host().unwrap().env.is_empty());
        let newer: Item = serde_json::from_value(json!({
            "type": "host",
            "name": "newer",
            "env": [{"name": "A", "value": "1", "secret": false}, {"name": "B"}]
        }))
        .unwrap();
        assert_eq!(
            newer.as_host().unwrap().env,
            [EnvVar::new("A", "1"), EnvVar::new("B", "")]
        );
    }

    #[test]
    fn host_env_check_rules() {
        let ok = [
            EnvVar::new("LANG", "ja_JP.UTF-8"),
            EnvVar::new("_private", ""),
            EnvVar::new("LC_ALL2", "spaces and = signs"),
            EnvVar::new("lang", "names are case-sensitive"),
        ];
        assert_eq!(check_host_env(&ok), Ok(()));
        assert_eq!(check_host_env(&[]), Ok(()));

        let one = |name: &str, value: &str| check_host_env(&[EnvVar::new(name, value)]);
        assert_eq!(one("", "x"), Err(EnvVarError::NameMissing));
        for bad in ["1ST", "A-B", "A B", "A=B", "日本", "a.b", " A"] {
            assert_eq!(
                one(bad, "x"),
                Err(EnvVarError::NameInvalid(bad.into())),
                "{bad}"
            );
        }
        assert_eq!(one(&"N".repeat(MAX_ENV_NAME_CHARS), ""), Ok(()));
        assert_eq!(
            one(&"N".repeat(MAX_ENV_NAME_CHARS + 1), ""),
            Err(EnvVarError::NameTooLong)
        );
        for bad in ["a\nb", "a\rb", "a\0b", "a\x1bb", "a\x7fb", "a\u{85}b"] {
            assert_eq!(one("V", bad), Err(EnvVarError::ValueInvalid("V".into())));
        }
        assert_eq!(one("V", "\ttab is fine"), Ok(()));
        assert_eq!(one("V", &"値".repeat(MAX_ENV_VALUE_CHARS)), Ok(()));
        assert_eq!(
            one("V", &"値".repeat(MAX_ENV_VALUE_CHARS + 1)),
            Err(EnvVarError::ValueTooLong("V".into()))
        );

        assert_eq!(
            check_host_env(&[
                EnvVar::new("A", "1"),
                EnvVar::new("B", ""),
                EnvVar::new("A", "2")
            ]),
            Err(EnvVarError::NameDuplicate("A".into()))
        );
        let many: Vec<EnvVar> = (0..=MAX_HOST_ENV_VARS)
            .map(|i| EnvVar::new(format!("V{i}"), ""))
            .collect();
        assert_eq!(check_host_env(&many[..MAX_HOST_ENV_VARS]), Ok(()));
        assert_eq!(check_host_env(&many), Err(EnvVarError::TooMany));

        // Names and values together, in UTF-8 bytes.
        let full = MAX_ENV_VALUE_CHARS;
        let mut big = vec![
            EnvVar::new("A", "x".repeat(full)),
            EnvVar::new("B", "x".repeat(full)),
            EnvVar::new("C", "x".repeat(full)),
            EnvVar::new("D", "x".repeat(MAX_HOST_ENV_BYTES - 3 * full - 4)),
        ];
        assert_eq!(env_bytes(&big), MAX_HOST_ENV_BYTES);
        assert_eq!(check_host_env(&big), Ok(()));
        big[3].value.push('x');
        assert_eq!(check_host_env(&big), Err(EnvVarError::TooLarge));
        assert_eq!(
            check_host_env(&[EnvVar::new("V", "値".repeat(MAX_HOST_ENV_BYTES / 3))]),
            Err(EnvVarError::ValueTooLong("V".into()))
        );
    }

    #[test]
    fn the_largest_host_env_fits_an_item() {
        // SSH-14: a host whose variables use the whole budget, with the escaping that costs most
        // (every value byte a quote), still fits the plaintext a synced item may have.
        let quotes = "\"".repeat(MAX_ENV_VALUE_CHARS);
        let mut env: Vec<EnvVar> = (0..4)
            .map(|i| EnvVar::new(format!("N{i}"), quotes.clone()))
            .collect();
        env.extend((4..MAX_HOST_ENV_VARS).map(|i| EnvVar::new(format!("N{i}"), "")));
        while env_bytes(&env) > MAX_HOST_ENV_BYTES {
            env[3].value.pop();
        }
        assert_eq!(check_host_env(&env), Ok(()));
        let item = Item::Host(Host {
            ai_notes: "日".repeat(MAX_HOST_AI_NOTES_CHARS),
            env,
            ..sample_host()
        });
        let len = item.plaintext_len().unwrap();
        assert_eq!(len, item.to_plaintext().unwrap().len());
        assert!(len <= MAX_ITEM_PLAINTEXT_BYTES, "{len}");
    }

    #[test]
    fn every_type_tag_round_trips() {
        let items = [
            (Item::Host(Host::default()), "host"),
            (Item::Group(Group::default()), "group"),
            (Item::Key(SshKey::default()), "key"),
            (Item::KnownHost(KnownHost::default()), "known_host"),
            (Item::Forward(PortForward::default()), "forward"),
            (Item::Snippet(Snippet::default()), "snippet"),
            (Item::AiProvider(AiProvider::default()), "ai_provider"),
            (
                Item::SearchProvider(SearchProvider::default()),
                "search_provider",
            ),
            (
                Item::AiConversation(AiConversation::default()),
                "ai_conversation",
            ),
            (Item::AiMessage(AiMessage::default()), "ai_message"),
            (Item::Skill(Skill::default()), "skill"),
            (Item::SkillFile(SkillFile::default()), "skill_file"),
            (Item::McpServer(McpServer::default()), "mcp_server"),
            (Item::Settings(Settings::default()), "settings"),
        ];
        for (item, tag) in items {
            assert_eq!(item.kind_str(), tag);
            let value = serde_json::to_value(&item).unwrap();
            assert_eq!(value["type"], tag);
            let back = Item::from_plaintext(&item.to_plaintext().unwrap()).unwrap();
            assert_eq!(back, item);
        }
    }

    #[test]
    fn ai_items_match_the_spec_shape() {
        let provider = Item::AiProvider(AiProvider {
            name: "Anthropic".into(),
            protocol: AiProtocol::Anthropic,
            base_url: "https://api.anthropic.com".into(),
            api_key: Zeroizing::new("sk-ant-key".into()),
            auth_header: AiAuthHeader::Authorization,
            models: vec![
                AiModel {
                    id: "claude-x".into(),
                    name: "Claude X".into(),
                    context_window: Some(200_000),
                    max_output_tokens: None,
                    efforts: Some(vec![AiEffort::Low, AiEffort::High, AiEffort::Max]),
                    adaptive_thinking: Some(true),
                },
                AiModel {
                    efforts: Some(vec![]),
                    ..AiModel::default()
                },
                AiModel::default(),
            ],
            updated_at: 7,
        });
        assert_eq!(
            serde_json::to_value(&provider).unwrap(),
            json!({
                "type": "ai_provider",
                "name": "Anthropic",
                "protocol": "anthropic",
                "base_url": "https://api.anthropic.com",
                "api_key": "sk-ant-key",
                "auth_header": "authorization",
                "models": [
                    {"id": "claude-x", "name": "Claude X",
                     "context_window": 200_000, "max_output_tokens": null,
                     "efforts": ["low", "high", "max"], "adaptive_thinking": true},
                    {"id": "", "name": "", "context_window": null, "max_output_tokens": null,
                     "efforts": []},
                    {"id": "", "name": "", "context_window": null, "max_output_tokens": null}
                ],
                "updated_at": 7
            })
        );
        assert_eq!(
            serde_json::to_value(Item::AiProvider(AiProvider::default())).unwrap(),
            json!({
                "type": "ai_provider", "name": "", "protocol": "chat_completions",
                "base_url": "", "api_key": "", "auth_header": "x-api-key",
                "models": [], "updated_at": 0
            })
        );

        let search = Item::SearchProvider(SearchProvider {
            kind: SearchKind::Searxng,
            base_url: Some("https://searx.example.org".into()),
            api_key: Zeroizing::new(String::new()),
            updated_at: 8,
        });
        assert_eq!(
            serde_json::to_value(&search).unwrap(),
            json!({
                "type": "search_provider", "kind": "searxng",
                "base_url": "https://searx.example.org", "api_key": "", "updated_at": 8
            })
        );
        for (kind, text) in [
            (SearchKind::Brave, "brave"),
            (SearchKind::Tavily, "tavily"),
            (SearchKind::Searxng, "searxng"),
        ] {
            assert_eq!(serde_json::to_value(kind).unwrap(), text);
            assert_eq!(kind.as_str(), text);
        }

        let conversation = Item::AiConversation(AiConversation {
            title: "Why is nginx down".into(),
            host_id: Some("h1".into()),
            pinned: true,
            context_start: Some("e1".into()),
            effort: Some(AiEffort::Xhigh),
            created_at: 5,
            updated_at: 9,
        });
        assert_eq!(
            serde_json::to_value(&conversation).unwrap(),
            json!({
                "type": "ai_conversation", "title": "Why is nginx down", "host_id": "h1",
                "pinned": true, "context_start": "e1", "effort": "xhigh", "created_at": 5,
                "updated_at": 9
            })
        );
        assert_eq!(
            serde_json::to_value(Item::AiConversation(AiConversation::default())).unwrap(),
            json!({
                "type": "ai_conversation", "title": "", "host_id": null, "pinned": false,
                "context_start": null, "created_at": 0, "updated_at": 0
            })
        );

        let message = Item::AiMessage(AiMessage {
            conversation_id: "c1".into(),
            entry_id: "e1".into(),
            part: 1,
            part_count: 3,
            data: r#"{"role":"user"}"#.into(),
            updated_at: 10,
        });
        assert_eq!(
            serde_json::to_value(&message).unwrap(),
            json!({
                "type": "ai_message", "conversation_id": "c1", "entry_id": "e1",
                "part": 1, "part_count": 3, "data": r#"{"role":"user"}"#, "updated_at": 10
            })
        );

        let mut frontmatter = serde_json::Map::new();
        frontmatter.insert("license".into(), json!("MIT"));
        frontmatter.insert("metadata".into(), json!({"author": "x", "tags": [1, 2]}));
        let skill = Item::Skill(Skill {
            name: "nginx-ops".into(),
            description: "Operate nginx".into(),
            frontmatter,
            enabled: true,
            updated_at: 11,
        });
        assert_eq!(
            serde_json::to_value(&skill).unwrap(),
            json!({
                "type": "skill", "name": "nginx-ops", "description": "Operate nginx",
                "frontmatter": {"license": "MIT", "metadata": {"author": "x", "tags": [1, 2]}},
                "enabled": true, "updated_at": 11
            })
        );

        let file = Item::SkillFile(SkillFile {
            skill_id: "s1".into(),
            path: "references/nginx.md".into(),
            content: "# nginx".into(),
            updated_at: 12,
        });
        assert_eq!(
            serde_json::to_value(&file).unwrap(),
            json!({
                "type": "skill_file", "skill_id": "s1", "path": "references/nginx.md",
                "content": "# nginx", "updated_at": 12
            })
        );

        let stdio = Item::McpServer(McpServer {
            name: "fs".into(),
            transport: McpTransport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "server-fs".into()],
                env: BTreeMap::from([("TOKEN".to_owned(), Zeroizing::new("t0k".to_owned()))]),
            },
            always_ask: true,
            updated_at: 13,
        });
        assert_eq!(
            serde_json::to_value(&stdio).unwrap(),
            json!({
                "type": "mcp_server", "name": "fs",
                "transport": {"kind": "stdio", "command": "npx", "args": ["-y", "server-fs"],
                              "env": {"TOKEN": "t0k"}},
                "always_ask": true, "updated_at": 13
            })
        );
        let http = Item::McpServer(McpServer {
            name: "remote".into(),
            transport: McpTransport::Http {
                url: "https://mcp.example.org/mcp".into(),
                headers: BTreeMap::from([(
                    "Authorization".to_owned(),
                    Zeroizing::new("Bearer b".to_owned()),
                )]),
            },
            always_ask: false,
            updated_at: 14,
        });
        assert_eq!(
            serde_json::to_value(&http).unwrap(),
            json!({
                "type": "mcp_server", "name": "remote",
                "transport": {"kind": "http", "url": "https://mcp.example.org/mcp",
                              "headers": {"Authorization": "Bearer b"}},
                "always_ask": false, "updated_at": 14
            })
        );

        let mut settings = Settings::default();
        settings.ai.default_model = Some(AiModelRef {
            provider_id: "p1".into(),
            model_id: "m1".into(),
        });
        settings.ai.search_provider_id = Some("sp1".into());
        settings.ai.default_effort = Some(AiEffort::Medium);
        settings.ai.custom_instructions = "Answer in English.".into();
        let value = serde_json::to_value(Item::Settings(settings)).unwrap();
        assert_eq!(
            value["ai"],
            json!({
                "default_model": {"provider_id": "p1", "model_id": "m1"},
                "default_effort": "medium",
                "search_provider_id": "sp1",
                "builtin_skill_enabled": true,
                "custom_instructions": "Answer in English."
            })
        );
        assert_eq!(
            serde_json::to_value(Item::Settings(Settings::default())).unwrap()["ai"],
            json!({"default_model": null, "search_provider_id": null, "builtin_skill_enabled": true})
        );

        // Every shape reads back as written.
        for item in [
            provider,
            search,
            conversation,
            message,
            skill,
            file,
            stdio,
            http,
        ] {
            let back = Item::from_plaintext(&item.to_plaintext().unwrap()).unwrap();
            assert_eq!(back, item);
        }
    }

    #[test]
    fn ai_items_tolerate_unknown_and_missing_fields() {
        let read = |value: serde_json::Value| -> Item { serde_json::from_value(value).unwrap() };

        // Settings written before the AI assistant existed have no `ai`.
        let old = read(json!({"type": "settings", "auto_lock_minutes": 5}));
        assert_eq!(old.as_settings().unwrap().ai, AiSettings::default());
        // Those written before the built-in skill (AI-34) have it on.
        let before = read(json!({"type": "settings", "ai": {"search_provider_id": "s"}}));
        let ai = &before.as_settings().unwrap().ai;
        assert!(ai.builtin_skill_enabled);
        assert_eq!(ai.search_provider_id.as_deref(), Some("s"));
        let off = read(json!({"type": "settings", "ai": {"builtin_skill_enabled": false}}));
        assert!(!off.as_settings().unwrap().ai.builtin_skill_enabled);
        // AI-36: settings from before custom instructions have none.
        assert_eq!(ai.custom_instructions, "");
        let with = read(json!({
            "type": "settings",
            "ai": {"custom_instructions": "Reply in English.\nUse zsh."}
        }));
        assert_eq!(
            with.as_settings().unwrap().ai.custom_instructions,
            "Reply in English.\nUse zsh."
        );

        // AI-05: settings, conversations and models from before thinking levels read as Default
        // and unknown, and a level a newer version added does not make the item unreadable.
        assert_eq!(ai.default_effort, None);
        let newer = read(json!({"type": "settings", "ai": {"default_effort": "ultra"}}));
        assert_eq!(newer.as_settings().unwrap().ai.default_effort, None);
        let high = read(json!({"type": "settings", "ai": {"default_effort": "high"}}));
        assert_eq!(
            high.as_settings().unwrap().ai.default_effort,
            Some(AiEffort::High)
        );
        let old = read(json!({"type": "ai_conversation", "title": "t", "pinned": true}));
        assert_eq!(old.as_ai_conversation().unwrap().effort, None);
        for effort in [json!("ultra"), json!(3), json!(null)] {
            let c = read(json!({"type": "ai_conversation", "effort": effort}));
            assert_eq!(c.as_ai_conversation().unwrap().effort, None, "{effort}");
        }
        let levels = read(json!({
            "type": "ai_provider",
            "models": [
                {"id": "a", "efforts": ["max", "ultra", "low", "low"], "adaptive_thinking": false},
                {"id": "b", "efforts": "all"},
                {"id": "c"}
            ]
        }));
        let models = &levels.as_ai_provider().unwrap().models;
        assert_eq!(models[0].efforts, Some(vec![AiEffort::Low, AiEffort::Max]));
        assert_eq!(models[0].adaptive_thinking, Some(false));
        assert_eq!(models[1].efforts, None);
        assert_eq!(
            (models[2].efforts.clone(), models[2].adaptive_thinking),
            (None, None)
        );

        // A missing or unknown protocol / auth header reads as the default.
        let provider = read(json!({
            "type": "ai_provider", "name": "p", "protocol": "grpc", "auth_header": "cookie",
            "base_url": "https://x", "future": [1],
            "models": [{"id": "m", "extra": true}]
        }));
        let provider = provider.as_ai_provider().unwrap();
        assert_eq!(provider.protocol, AiProtocol::ChatCompletions);
        assert_eq!(provider.auth_header, AiAuthHeader::XApiKey);
        assert_eq!(provider.models[0].id, "m");
        assert_eq!(provider.models[0].context_window, None);
        assert_eq!(provider.api_key.as_str(), "");
        let bare = read(json!({"type": "ai_provider"}));
        assert_eq!(
            bare.as_ai_provider().unwrap().protocol,
            AiProtocol::ChatCompletions
        );

        // A search kind from the future is not guessed: the item is unreadable instead.
        assert!(matches!(
            Item::from_plaintext(br#"{"type":"search_provider","kind":"kagi"}"#),
            Err(Error::Format(_))
        ));
        assert_eq!(
            read(json!({"type": "search_provider"}))
                .as_search_provider()
                .unwrap()
                .kind,
            SearchKind::Brave
        );

        // MCP transports tolerate missing fields; their kind is required to be known.
        let mcp = read(json!({
            "type": "mcp_server", "name": "m", "extra": 1,
            "transport": {"kind": "http", "url": "https://m", "future": 1}
        }));
        assert_eq!(
            mcp.as_mcp_server().unwrap().transport,
            McpTransport::Http {
                url: "https://m".into(),
                headers: BTreeMap::new()
            }
        );
        let stdio = read(json!({"type": "mcp_server", "transport": {"kind": "stdio"}}));
        assert_eq!(
            stdio.as_mcp_server().unwrap().transport,
            McpTransport::default()
        );
        assert!(
            Item::from_plaintext(br#"{"type":"mcp_server","transport":{"kind":"carrier-pigeon"}}"#)
                .is_err()
        );
        let missing = read(json!({"type": "mcp_server"}));
        assert_eq!(
            missing.as_mcp_server().unwrap().transport.kind_str(),
            "stdio"
        );

        let skill = read(json!({"type": "skill", "name": "s", "unknown": {}}));
        assert!(skill.as_skill().unwrap().frontmatter.is_empty());
        assert!(!skill.as_skill().unwrap().enabled);
    }

    #[test]
    fn ai_item_names_and_accessors() {
        let provider = Item::AiProvider(AiProvider {
            name: "Anthropic".into(),
            updated_at: 3,
            ..AiProvider::default()
        });
        assert_eq!(provider.display_name(), "Anthropic");
        assert_eq!(provider.updated_at(), 3);
        assert_eq!(
            provider.with_name_suffix(" (copy)").display_name(),
            "Anthropic (copy)"
        );
        assert!(provider.as_ai_provider().is_some() && provider.as_host().is_none());
        assert!(!provider.is_key());

        let search = Item::SearchProvider(SearchProvider {
            kind: SearchKind::Tavily,
            ..SearchProvider::default()
        });
        assert_eq!(search.display_name(), "tavily");
        assert_eq!(search.with_name_suffix(" (copy)"), search);
        assert!(search.as_search_provider().is_some());

        let mut conversation = Item::AiConversation(AiConversation {
            title: "Disk full".into(),
            ..AiConversation::default()
        });
        assert_eq!(conversation.display_name(), "Disk full");
        conversation.set_updated_at(99);
        assert_eq!(conversation.updated_at(), 99);
        assert!(conversation.as_ai_conversation().is_some());

        let mut message = Item::AiMessage(AiMessage {
            data: "x".into(),
            ..AiMessage::default()
        });
        assert_eq!(message.display_name(), "message");
        message.set_updated_at(4);
        assert_eq!(message.updated_at(), 4);
        assert_eq!(message.as_ai_message().unwrap().data, "x");
        message.strip_message_data();
        assert_eq!(message.as_ai_message().unwrap().data, "");

        let mut skill = Item::Skill(Skill {
            name: "nginx".into(),
            ..Skill::default()
        });
        skill.strip_message_data(); // a no-op for every other type
        assert_eq!(skill.display_name(), "nginx");
        assert!(skill.as_skill().is_some());
        let file = Item::SkillFile(SkillFile {
            path: "SKILL.md".into(),
            ..SkillFile::default()
        });
        assert_eq!(file.display_name(), "SKILL.md");
        assert!(file.as_skill_file().is_some());
        let mcp = Item::McpServer(McpServer {
            name: "fs".into(),
            ..McpServer::default()
        });
        assert_eq!(mcp.display_name(), "fs");
        assert_eq!(mcp.with_name_suffix("!").display_name(), "fs!");
        assert!(mcp.as_mcp_server().is_some());
    }

    #[test]
    fn auth_variants_use_the_kind_tag() {
        let cases = [
            (
                HostAuth::password("p"),
                json!({"kind":"password","password":"p"}),
            ),
            (
                HostAuth::Key { key_id: "k".into() },
                json!({"kind":"key","key_id":"k"}),
            ),
            (HostAuth::Agent, json!({"kind":"agent"})),
            (HostAuth::Ask, json!({"kind":"ask"})),
        ];
        for (auth, expected) in cases {
            assert_eq!(serde_json::to_value(&auth).unwrap(), expected);
            assert_eq!(serde_json::from_value::<HostAuth>(expected).unwrap(), auth);
        }
    }

    #[test]
    fn settings_defaults_match_the_spec() {
        let s = Settings::default();
        assert_eq!(s.terminal.font_family, "Cascadia Mono");
        assert_eq!(s.terminal.font_size, 13);
        assert_eq!(s.terminal.theme, ThemeMode::Dark);
        assert_eq!(s.terminal.cursor_style, CursorStyle::Block);
        assert_eq!(s.terminal.scrollback, 10_000);
        assert_eq!(s.terminal.right_click(), RightClick::CopyPaste);
        assert!(s.terminal.confirm_multiline_paste());
        assert_eq!(s.auto_lock_minutes, 15);
        assert!(!s.lock_disconnects_sessions);
        let value = serde_json::to_value(Item::Settings(s)).unwrap();
        assert_eq!(value["terminal"]["theme"], "dark");
        assert_eq!(value["terminal"]["cursor_style"], "block");
        // Unset fields are left out, as builds that kept them device-local wrote them.
        assert!(value["terminal"].get("right_click").is_none());
        assert!(value["terminal"].get("confirm_multiline_paste").is_none());
    }

    #[test]
    fn terminal_behaviour_tells_unset_from_default() {
        let read = |terminal: serde_json::Value| -> TerminalSettings {
            let item: Item =
                serde_json::from_value(json!({"type":"settings","terminal":terminal})).unwrap();
            item.as_settings().unwrap().terminal.clone()
        };
        let unset = read(json!({"font_size": 14}));
        assert_eq!(unset.right_click, None);
        assert_eq!(unset.confirm_multiline_paste, None);
        let set = read(json!({"right_click":"copy_paste","confirm_multiline_paste":true}));
        assert_eq!(set.right_click, Some(RightClick::CopyPaste));
        assert_eq!(set.confirm_multiline_paste, Some(true));
        let set = read(json!({"right_click":"menu","confirm_multiline_paste":false}));
        assert_eq!(set.right_click(), RightClick::Menu);
        assert!(!set.confirm_multiline_paste());
        let value = serde_json::to_value(&set).unwrap();
        assert_eq!(value["right_click"], "menu");
        assert_eq!(value["confirm_multiline_paste"], false);
        // An unknown future value still counts as a choice the user made.
        assert_eq!(
            read(json!({"right_click":"middle_paste"})).right_click,
            Some(RightClick::CopyPaste)
        );
    }

    #[test]
    fn recording_the_default_leaves_an_unset_field_unset() {
        let mut t = TerminalSettings::default();
        t.set_right_click(RightClick::CopyPaste);
        t.set_confirm_multiline_paste(true);
        assert_eq!(t.right_click, None);
        assert_eq!(t.confirm_multiline_paste, None);

        t.set_right_click(RightClick::Menu);
        t.set_confirm_multiline_paste(false);
        assert_eq!(t.right_click, Some(RightClick::Menu));
        assert_eq!(t.confirm_multiline_paste, Some(false));

        // Once set, changing back to the default is recorded too.
        t.set_right_click(RightClick::CopyPaste);
        t.set_confirm_multiline_paste(true);
        assert_eq!(t.right_click, Some(RightClick::CopyPaste));
        assert_eq!(t.confirm_multiline_paste, Some(true));
    }

    #[test]
    fn tolerant_of_unknown_and_missing_fields() {
        let item: Item = serde_json::from_value(json!({
            "type": "host",
            "name": "x",
            "address": "example.org",
            "from_the_future": { "nested": [1, 2, 3] }
        }))
        .unwrap();
        let host = item.as_host().unwrap();
        assert_eq!(host.port, 22);
        assert_eq!(host.auth, HostAuth::Ask);
        assert!(host.tags.is_empty());
        assert_eq!(host.group_id, None);

        // Unknown enum values degrade gracefully instead of failing the whole item.
        let item: Item = serde_json::from_value(
            json!({"type":"settings","terminal":{"theme":"solarized","cursor_style":"beam"}}),
        )
        .unwrap();
        let s = item.as_settings().unwrap();
        assert_eq!(s.terminal.theme, ThemeMode::System);
        assert_eq!(s.terminal.cursor_style, CursorStyle::Block);
        assert_eq!(s.auto_lock_minutes, 15);
    }

    #[test]
    fn unknown_type_is_a_format_error() {
        let err = Item::from_plaintext(br#"{"type":"teleporter","name":"x"}"#);
        assert!(matches!(err, Err(Error::Format(_))));
        assert!(matches!(Item::from_plaintext(b"{}"), Err(Error::Format(_))));
    }

    #[test]
    fn updated_at_and_names() {
        let mut item = Item::Host(sample_host());
        assert_eq!(item.updated_at(), 1_700_000_000_000);
        item.set_updated_at(5);
        assert_eq!(item.updated_at(), 5);
        assert_eq!(item.display_name(), "prod-api-tokyo");
        let key = Item::Key(SshKey {
            name: "laptop".into(),
            ..SshKey::default()
        });
        assert_eq!(
            key.with_name_suffix(" (copy)").display_name(),
            "laptop (copy)"
        );
        assert!(key.is_key());
        let kh = Item::KnownHost(KnownHost {
            host: "h".into(),
            port: 2222,
            ..KnownHost::default()
        });
        assert_eq!(kh.display_name(), "h:2222");
    }

    #[test]
    fn ids_are_uuid_v7_and_time_ordered() {
        let a = new_id();
        let b = new_id();
        let parsed = uuid::Uuid::parse_str(&a).unwrap();
        assert_eq!(parsed.get_version_num(), 7);
        assert_ne!(a, b);
    }

    #[test]
    fn debug_never_prints_secrets() {
        let host = Item::Host(sample_host());
        assert!(!format!("{host:?}").contains("hunter2"));
        let key = Item::Key(SshKey {
            private_key: Zeroizing::new("-----BEGIN OPENSSH PRIVATE KEY-----SECRETBODY".into()),
            passphrase: Some(Zeroizing::new("pass-phrase-secret".into())),
            ..SshKey::default()
        });
        let dbg = format!("{key:?}");
        assert!(!dbg.contains("SECRETBODY"));
        assert!(!dbg.contains("pass-phrase-secret"));

        let provider = Item::AiProvider(AiProvider {
            name: "openai".into(),
            api_key: Zeroizing::new("sk-provider-secret".into()),
            ..AiProvider::default()
        });
        let search = Item::SearchProvider(SearchProvider {
            api_key: Zeroizing::new("search-api-secret".into()),
            ..SearchProvider::default()
        });
        let stdio = Item::McpServer(McpServer {
            name: "fs".into(),
            transport: McpTransport::Stdio {
                command: "npx".into(),
                args: vec!["server".into()],
                env: BTreeMap::from([(
                    "API_TOKEN".to_owned(),
                    Zeroizing::new("env-secret".to_owned()),
                )]),
            },
            ..McpServer::default()
        });
        let http = Item::McpServer(McpServer {
            transport: McpTransport::Http {
                url: "https://mcp.example.org".into(),
                headers: BTreeMap::from([(
                    "Authorization".to_owned(),
                    Zeroizing::new("header-secret".to_owned()),
                )]),
            },
            ..McpServer::default()
        });
        let message = Item::AiMessage(AiMessage {
            entry_id: "e1".into(),
            data: "conversation-secret".into(),
            ..AiMessage::default()
        });
        for (item, secret) in [
            (&provider, "sk-provider-secret"),
            (&search, "search-api-secret"),
            (&stdio, "env-secret"),
            (&http, "header-secret"),
            (&message, "conversation-secret"),
        ] {
            let dbg = format!("{item:?}");
            assert!(!dbg.contains(secret), "{dbg}");
            let pretty = format!("{item:#?}");
            assert!(!pretty.contains(secret), "{pretty}");
        }
        // What is not secret stays readable.
        assert!(format!("{provider:?}").contains("openai"));
        assert!(format!("{stdio:?}").contains("API_TOKEN"));
        assert!(format!("{http:?}").contains("https://mcp.example.org"));
        assert!(format!("{message:?}").contains("e1"));
    }

    #[test]
    fn zeroize_wipes_secret_fields() {
        let mut item = Item::Key(SshKey {
            private_key: Zeroizing::new("PRIVATE".into()),
            passphrase: Some(Zeroizing::new("PASS".into())),
            name: "n".into(),
            ..SshKey::default()
        });
        item.zeroize();
        let key = item.as_key().unwrap();
        assert!(key.private_key.is_empty());
        assert!(key.passphrase.is_none());
        assert!(key.name.is_empty());

        let mut provider = Item::AiProvider(AiProvider {
            name: "n".into(),
            api_key: Zeroizing::new("KEY".into()),
            models: vec![AiModel {
                id: "m".into(),
                ..AiModel::default()
            }],
            ..AiProvider::default()
        });
        provider.zeroize();
        let provider = provider.as_ai_provider().unwrap();
        assert!(provider.api_key.is_empty() && provider.name.is_empty());
        assert!(provider.models.is_empty());

        let mut search = Item::SearchProvider(SearchProvider {
            base_url: Some("https://s".into()),
            api_key: Zeroizing::new("KEY".into()),
            ..SearchProvider::default()
        });
        search.zeroize();
        let search = search.as_search_provider().unwrap();
        assert!(search.api_key.is_empty() && search.base_url.is_none());

        let mut mcp = Item::McpServer(McpServer {
            name: "n".into(),
            transport: McpTransport::Stdio {
                command: "cmd".into(),
                args: vec!["a".into()],
                env: BTreeMap::from([("K".to_owned(), Zeroizing::new("V".to_owned()))]),
            },
            ..McpServer::default()
        });
        mcp.zeroize();
        let mcp = mcp.as_mcp_server().unwrap();
        assert!(mcp.name.is_empty());
        assert_eq!(mcp.transport, McpTransport::default());
        let mut http = McpTransport::Http {
            url: "https://u".into(),
            headers: BTreeMap::from([("H".to_owned(), Zeroizing::new("V".to_owned()))]),
        };
        http.zeroize();
        assert_eq!(
            http,
            McpTransport::Http {
                url: String::new(),
                headers: BTreeMap::new()
            }
        );

        let mut message = Item::AiMessage(AiMessage {
            data: "DATA".into(),
            conversation_id: "c".into(),
            ..AiMessage::default()
        });
        message.zeroize();
        let message = message.as_ai_message().unwrap();
        assert!(message.data.is_empty() && message.conversation_id.is_empty());
    }
}
