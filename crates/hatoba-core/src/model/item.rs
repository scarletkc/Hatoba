use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::{
    AiConversation, AiMemory, AiMessage, AiProvider, Group, Host, KnownHost, McpServer,
    PortForward, Proxy, SearchProvider, Settings, Skill, SkillFile, Snippet, SshKey, TelnetHost,
};
use crate::error::{Error, Result};

/// A fresh UUIDv7 item id (time-ordered, so the item map iterates in creation order).
#[must_use]
pub fn new_id() -> String {
    uuid::Uuid::now_v7().to_string()
}

/// The most plaintext bytes (the item's JSON) an item may hold, so that its envelope stays under
/// the 64 KiB that the sync Worker and D1 accept (§6.2); the same budget as
/// [`AI_PART_MAX_BYTES`](crate::AI_PART_MAX_BYTES). Larger items are refused when saved.
pub const MAX_ITEM_PLAINTEXT_BYTES: usize = 40 * 1024;

/// Any syncable item. Serialises with `"type": "host" | "telnet_host" | "group" | "key" |
/// "known_host" | "forward" | "snippet" | "proxy" | "ai_provider" | "search_provider" |
/// "ai_conversation" | "ai_message" | "skill" | "skill_file" | "mcp_server" | "ai_memory" |
/// "settings"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Zeroize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Item {
    Host(Host),
    TelnetHost(TelnetHost),
    Group(Group),
    /// A private key.
    Key(SshKey),
    /// A trusted host key.
    KnownHost(KnownHost),
    Forward(PortForward),
    Snippet(Snippet),
    /// A SOCKS5 or HTTP proxy.
    Proxy(Proxy),
    AiProvider(AiProvider),
    SearchProvider(SearchProvider),
    AiConversation(AiConversation),
    /// One part of a conversation entry.
    AiMessage(AiMessage),
    Skill(Skill),
    /// A text file of a skill.
    SkillFile(SkillFile),
    McpServer(McpServer),
    /// A memory of the AI assistant.
    AiMemory(AiMemory),
    /// The settings singleton.
    Settings(Settings),
}

impl Item {
    /// The `type` string as it appears in the plaintext JSON.
    #[must_use]
    pub fn kind_str(&self) -> &'static str {
        match self {
            Self::Host(_) => "host",
            Self::TelnetHost(_) => "telnet_host",
            Self::Group(_) => "group",
            Self::Key(_) => "key",
            Self::KnownHost(_) => "known_host",
            Self::Forward(_) => "forward",
            Self::Snippet(_) => "snippet",
            Self::Proxy(_) => "proxy",
            Self::AiProvider(_) => "ai_provider",
            Self::SearchProvider(_) => "search_provider",
            Self::AiConversation(_) => "ai_conversation",
            Self::AiMessage(_) => "ai_message",
            Self::Skill(_) => "skill",
            Self::SkillFile(_) => "skill_file",
            Self::McpServer(_) => "mcp_server",
            Self::AiMemory(_) => "ai_memory",
            Self::Settings(_) => "settings",
        }
    }

    /// Last modification time (Unix ms).
    #[must_use]
    pub fn updated_at(&self) -> i64 {
        match self {
            Self::Host(i) => i.updated_at,
            Self::TelnetHost(i) => i.updated_at,
            Self::Group(i) => i.updated_at,
            Self::Key(i) => i.updated_at,
            Self::KnownHost(i) => i.updated_at,
            Self::Forward(i) => i.updated_at,
            Self::Snippet(i) => i.updated_at,
            Self::Proxy(i) => i.updated_at,
            Self::AiProvider(i) => i.updated_at,
            Self::SearchProvider(i) => i.updated_at,
            Self::AiConversation(i) => i.updated_at,
            Self::AiMessage(i) => i.updated_at,
            Self::Skill(i) => i.updated_at,
            Self::SkillFile(i) => i.updated_at,
            Self::McpServer(i) => i.updated_at,
            Self::AiMemory(i) => i.updated_at,
            Self::Settings(i) => i.updated_at,
        }
    }

    /// Sets the modification time (Unix ms).
    pub fn set_updated_at(&mut self, at: i64) {
        match self {
            Self::Host(i) => i.updated_at = at,
            Self::TelnetHost(i) => i.updated_at = at,
            Self::Group(i) => i.updated_at = at,
            Self::Key(i) => i.updated_at = at,
            Self::KnownHost(i) => i.updated_at = at,
            Self::Forward(i) => i.updated_at = at,
            Self::Snippet(i) => i.updated_at = at,
            Self::Proxy(i) => i.updated_at = at,
            Self::AiProvider(i) => i.updated_at = at,
            Self::SearchProvider(i) => i.updated_at = at,
            Self::AiConversation(i) => i.updated_at = at,
            Self::AiMessage(i) => i.updated_at = at,
            Self::Skill(i) => i.updated_at = at,
            Self::SkillFile(i) => i.updated_at = at,
            Self::McpServer(i) => i.updated_at = at,
            Self::AiMemory(i) => i.updated_at = at,
            Self::Settings(i) => i.updated_at = at,
        }
    }

    /// A human-readable label for lists and conflict screens. Never a secret.
    #[must_use]
    pub fn display_name(&self) -> String {
        match self {
            Self::Host(i) => i.name.clone(),
            Self::TelnetHost(i) => i.name.clone(),
            Self::Group(i) => i.name.clone(),
            Self::Key(i) => i.name.clone(),
            Self::KnownHost(i) => format!("{}:{}", i.host, i.port),
            Self::Forward(i) if i.name.is_empty() => format!("{}:{}", i.bind_address, i.bind_port),
            Self::Forward(i) => i.name.clone(),
            Self::Snippet(i) => i.name.clone(),
            Self::Proxy(i) => i.name.clone(),
            Self::AiProvider(i) => i.name.clone(),
            Self::SearchProvider(i) => i.kind.as_str().to_owned(),
            Self::AiConversation(i) => i.title.clone(),
            Self::AiMessage(_) => "message".to_owned(),
            Self::Skill(i) => i.name.clone(),
            Self::SkillFile(i) => i.path.clone(),
            Self::McpServer(i) => i.name.clone(),
            Self::AiMemory(i) => i.name.clone(),
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

    /// The Telnet host, if this is one.
    #[must_use]
    pub fn as_telnet_host(&self) -> Option<&TelnetHost> {
        if let Self::TelnetHost(h) = self {
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

    /// The proxy, if this is one.
    #[must_use]
    pub fn as_proxy(&self) -> Option<&Proxy> {
        if let Self::Proxy(p) = self {
            Some(p)
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

    /// The AI memory, if this is one.
    #[must_use]
    pub fn as_ai_memory(&self) -> Option<&AiMemory> {
        if let Self::AiMemory(m) = self {
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
            Self::TelnetHost(i) => i.name.push_str(suffix),
            Self::Group(i) => i.name.push_str(suffix),
            Self::Key(i) => i.name.push_str(suffix),
            Self::Snippet(i) => i.name.push_str(suffix),
            Self::Proxy(i) => i.name.push_str(suffix),
            Self::AiProvider(i) => i.name.push_str(suffix),
            Self::Skill(i) => i.name.push_str(suffix),
            Self::McpServer(i) => i.name.push_str(suffix),
            Self::AiMemory(i) => i.name.push_str(suffix),
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
