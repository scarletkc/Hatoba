use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

/// Wire protocol of an [`AiProvider`] (spec §13.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AiProtocol {
    /// Anthropic Messages (`/v1/messages`).
    Anthropic,
    /// OpenAI Responses (`/responses`). A version without it reads such a provider as Chat
    /// Completions, which the same base URL also serves at OpenAI.
    Responses,
    /// TypeSafe's System One decision format, which the judge of Auto mode can use (AI-47). Such
    /// a provider answers decision questions and cannot chat: `base_url` is the endpoint's base,
    /// such as `https://api.typesafe.ai/v1`, and `models` are its model ids, such as `jev-latest`
    /// or `@cf/cloudflare/clef-flash`.
    SystemOne,
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
            Self::Responses => "responses",
            Self::SystemOne => "system_one",
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
pub(super) mod effort_serde {
    use serde::{Deserialize, Deserializer};
    use serde_json::Value;

    use super::AiEffort;

    /// An optional level; anything that is not a known level reads as `None` (Default).
    pub(in crate::model) fn level<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<AiEffort>, D::Error> {
        let value = Option::<Value>::deserialize(d)?;
        Ok(value
            .as_ref()
            .and_then(Value::as_str)
            .and_then(AiEffort::parse))
    }

    /// A model's levels, lowest first, without the ones this version does not know; `None`
    /// (unknown) when the value is not a list.
    pub(in crate::model) fn levels<'de, D: Deserializer<'de>>(
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
    /// Whether the model supports adaptive thinking, from Anthropic's model list or models.dev
    /// (AI-03); `None` (absent) when unknown.
    #[zeroize(skip)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptive_thinking: Option<bool>,
}

/// An AI model provider (P1, spec §13.2). The API key is wiped on drop and never printed.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct AiProvider {
    pub name: String,
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
    /// The ChatGPT account the provider signs in with instead of a key (AI-52), absent for a key
    /// provider. Its tokens never sync: each device signs in itself and keeps its session
    /// sealed in its own vault.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chatgpt: Option<AiChatgptAccount>,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

/// The ChatGPT account of an [`AiProvider`] (AI-52): which registration of Hatoba a sign-in on
/// any device reuses. No secrets, but wiped with the provider like the rest of the item.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct AiChatgptAccount {
    /// The client id OpenAI issued at the first sign-in (`oaiapp_…`).
    pub client_id: String,
    /// The account's verified `sub`.
    pub subject: String,
    /// The account's email, shown in the settings and sent as a sign-in hint.
    pub email: String,
}

impl fmt::Debug for AiChatgptAccount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AiChatgptAccount")
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
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
            .field("chatgpt", &self.chatgpt)
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
    /// The quick-connect target (HOST-12) it last worked on, while `host_id` is `None`: a message
    /// in a tab on a saved host or another target starts with a `host_change` note from it
    /// (AI-09). `None` (absent) is none, as for conversations from before it was recorded, which
    /// get no note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quick_target: Option<AiQuickTarget>,
    /// AI-43: for a fork that records neither a host nor a target, the name a `host_change` note
    /// gives the server its copies came from (AI-09), when the copies tell the name but not which
    /// saved host or target it is (empty for a host that no longer exists). The next message that
    /// moves the fork starts with a note from it, and the move clears it. `None` (absent) is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// §7.5: the name of the computer whose local terminal tab it last worked on, while it has
    /// neither a host nor a target. `None` (absent) is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local: Option<String>,
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

/// A quick-connect target (HOST-12) as a conversation records it (AI-09): the address in
/// lowercase and without brackets, so one server has one form and targets compare as HOST-12
/// compares them. Build it with [`AiQuickTarget::new`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct AiQuickTarget {
    /// The user name it connects as.
    pub username: String,
    /// DNS name or IP address, in lowercase, IPv6 without brackets.
    pub address: String,
    pub port: u16,
}

impl AiQuickTarget {
    /// The recorded form of `username@address:port`.
    #[must_use]
    pub fn new(username: &str, address: &str, port: u16) -> Self {
        let address = address
            .strip_prefix('[')
            .and_then(|a| a.strip_suffix(']'))
            .unwrap_or(address);
        Self {
            username: username.to_owned(),
            address: address.to_lowercase(),
            port,
        }
    }
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

/// A note the AI assistant keeps across conversations (AI-53, spec §13.12). A global memory, with
/// neither `host_id` nor `local`, shows in every conversation; a host memory only in those on its
/// saved host, and a local one only in those on its computer's local terminal (§7.5).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(default)]
pub struct AiMemory {
    /// One line, unique among the memories of its scope.
    pub name: String,
    /// The text: a few lines on one subject.
    pub content: String,
    /// The saved host it belongs to. `None` (absent) is none, as for every memory from before
    /// host memories, which are global.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_id: Option<String>,
    /// The name of the computer whose local terminal it belongs to, while `host_id` is `None`.
    /// `None` (absent) is none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub local: Option<String>,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}

impl AiMemory {
    /// The UTF-8 bytes it takes of the limit on all memories together (AI-53): its name and its
    /// content.
    #[must_use]
    pub fn size(&self) -> usize {
        self.name.len() + self.content.len()
    }
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
        #[serde(default)]
        args: Vec<String>,
        /// Environment variables (values are secrets).
        #[serde(default)]
        env: BTreeMap<String, Zeroizing<String>>,
    },
    /// A Streamable HTTP endpoint.
    Http {
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
    pub transport: McpTransport,
    /// Ask before every call, even in bypass mode (AI-31).
    pub always_ask: bool,
    /// Last modification, Unix ms.
    pub updated_at: i64,
}
