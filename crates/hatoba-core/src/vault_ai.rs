//! AI assistant items in the vault (spec §5.1, §13.7).
//!
//! Providers, search providers, skills and MCP servers are plain items with typed accessors. A
//! conversation is an `ai_conversation` item plus its entries, which are stored as `ai_message`
//! items:
//!
//! * An entry is written once and never changes, so entries never conflict: two devices that
//!   append to one conversation merge on sync, ordered by `entry_id` (a UUIDv7 that
//!   [`Vault::ai_append_entry`] makes greater than every id already in the conversation).
//! * An entry's JSON is split across as many parts as it takes to keep each part's item plaintext
//!   within [`AI_PART_MAX_BYTES`], so every envelope stays under the Worker's 64 KB limit (§6.2).
//! * The item map keeps the header of a part (`conversation_id`, `entry_id`, `part`,
//!   `part_count`, `updated_at`) and not its `data`. [`Vault::ai_entries`] reads a conversation
//!   back from the local database and decrypts it again.
//! * Deleting a conversation or a skill tombstones everything that belongs to it (§6.5), and
//!   editing a message to send it again (AI-26) tombstones that entry and every later one.

use std::collections::BTreeMap;

use uuid::{Builder, Uuid};
use zeroize::{Zeroize, Zeroizing};

use crate::crypto::{item_aad, random_bytes, seal};
use crate::error::{Error, Result};
use crate::model::{
    AiConversation, AiMessage, AiProvider, Item, McpServer, SearchProvider, Skill, SkillFile,
    new_id,
};
use crate::store::{ItemRow, StoreOps};
use crate::vault::{Vault, decode_envelope};

/// The most plaintext bytes one `ai_message` item may hold: the whole item JSON, with the
/// escaped `data`. The envelope adds 16 bytes of tag and base64, about 55 KB in all.
pub const AI_PART_MAX_BYTES: usize = 40 * 1024;

/// A conversation entry as stored: its id and its JSON (spec §13.7), joined from its parts. The
/// JSON is conversation content and is wiped when dropped.
#[derive(Clone, PartialEq, Eq)]
pub struct StoredEntry {
    /// UUIDv7 that orders the entries of a conversation.
    pub entry_id: String,
    /// The entry's JSON.
    pub json: Zeroizing<String>,
}

impl std::fmt::Debug for StoredEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoredEntry")
            .field("entry_id", &self.entry_id)
            .field(
                "json",
                &format_args!("<{} bytes redacted>", self.json.len()),
            )
            .finish()
    }
}

impl Vault {
    // ---- typed accessors ----

    /// All AI model providers.
    #[must_use]
    pub fn ai_providers(&self) -> Vec<(String, AiProvider)> {
        self.collect(Item::as_ai_provider)
    }

    /// All search providers.
    #[must_use]
    pub fn search_providers(&self) -> Vec<(String, SearchProvider)> {
        self.collect(Item::as_search_provider)
    }

    /// All AI conversations (their entries are read with [`ai_entries`](Self::ai_entries)).
    #[must_use]
    pub fn ai_conversations(&self) -> Vec<(String, AiConversation)> {
        self.collect(Item::as_ai_conversation)
    }

    /// All skills.
    #[must_use]
    pub fn skills(&self) -> Vec<(String, Skill)> {
        self.collect(Item::as_skill)
    }

    /// The files of one skill.
    #[must_use]
    pub fn skill_files(&self, skill_id: &str) -> Vec<(String, SkillFile)> {
        self.items()
            .filter_map(|(id, item)| {
                item.as_skill_file()
                    .filter(|f| f.skill_id == skill_id)
                    .map(|f| (id.to_owned(), f.clone()))
            })
            .collect()
    }

    /// All MCP servers.
    #[must_use]
    pub fn mcp_servers(&self) -> Vec<(String, McpServer)> {
        self.collect(Item::as_mcp_server)
    }

    // ---- conversation entries ----

    /// The cached headers of one conversation's message parts, as `(item id, header)`.
    fn message_headers<'a>(
        &'a self,
        conversation_id: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a AiMessage)> + 'a {
        self.items().filter_map(move |(id, item)| {
            item.as_ai_message()
                .filter(|m| m.conversation_id == conversation_id)
                .map(|m| (id, m))
        })
    }

    /// The entries of a conversation, oldest first (by `entry_id`).
    ///
    /// The parts are read from the local database and decrypted again, because the item map
    /// holds no message data. An entry whose parts have not all arrived yet (sync delivers items
    /// in any order) or whose parts cannot be decrypted is left out; the log carries the item id
    /// only. The JSON is a [`Zeroizing`] string, wiped when dropped.
    ///
    /// # Errors
    /// [`Error::Locked`]; storage errors.
    pub fn ai_entries(&self, conversation_id: &str) -> Result<Vec<StoredEntry>> {
        let unlocked = self.unlocked.as_ref().ok_or(Error::Locked)?;

        // Group the cached headers by entry. `parts` maps a part number to the item holding it
        // (the first by item id if two items claim the same part).
        struct Pending<'a> {
            part_count: u32,
            consistent: bool,
            parts: BTreeMap<u32, &'a str>,
        }
        let mut pending: BTreeMap<&str, Pending<'_>> = BTreeMap::new();
        for (item_id, header) in self.message_headers(conversation_id) {
            let entry = pending
                .entry(header.entry_id.as_str())
                .or_insert_with(|| Pending {
                    part_count: header.part_count,
                    consistent: true,
                    parts: BTreeMap::new(),
                });
            entry.consistent &= entry.part_count == header.part_count;
            entry.parts.entry(header.part).or_insert(item_id);
        }

        let mut out = Vec::with_capacity(pending.len());
        'entries: for (entry_id, entry) in pending {
            // Distinct part numbers, as many as `part_count`, none above `part_count - 1`:
            // exactly 0..part_count.
            let complete = entry.consistent
                && entry.part_count > 0
                && usize::try_from(entry.part_count).is_ok_and(|n| n == entry.parts.len())
                && entry.parts.keys().next_back() == Some(&(entry.part_count - 1));
            if !complete {
                tracing::debug!(
                    conversation_id,
                    entry_id,
                    "conversation entry is incomplete; skipping"
                );
                continue;
            }
            let mut pieces: Vec<Zeroizing<String>> = Vec::with_capacity(entry.parts.len());
            for (&part, &item_id) in &entry.parts {
                let readable = self
                    .store
                    .item_row(item_id)?
                    .filter(|row| !row.deleted)
                    .and_then(|row| row.envelope)
                    .and_then(|env| decode_envelope(&unlocked.vault_key, item_id, &env).ok());
                match readable {
                    Some(Item::AiMessage(mut m))
                        if m.conversation_id == conversation_id
                            && m.entry_id == entry_id
                            && m.part == part
                            && m.part_count == entry.part_count =>
                    {
                        pieces.push(Zeroizing::new(std::mem::take(&mut m.data)));
                    }
                    other => {
                        // `pieces` wipes itself.
                        if let Some(mut item) = other {
                            item.zeroize();
                        }
                        tracing::warn!(
                            item_id,
                            "conversation entry part could not be read; skipping entry"
                        );
                        continue 'entries;
                    }
                }
            }
            // One allocation of the final size, so no un-wiped copy is left behind.
            let mut json =
                Zeroizing::new(String::with_capacity(pieces.iter().map(|p| p.len()).sum()));
            for piece in &pieces {
                json.push_str(piece);
            }
            out.push(StoredEntry {
                entry_id: entry_id.to_owned(),
                json,
            });
        }
        Ok(out)
    }

    /// Appends an entry to a conversation and returns its `entry_id`.
    ///
    /// The id is a fresh UUIDv7 greater than every entry id already in the conversation, even
    /// when this device's clock is behind the device that wrote the last one. `json` is split on
    /// character boundaries into as many `ai_message` items as it takes to keep each item's
    /// plaintext (the whole item JSON, escaped `data` included) within [`AI_PART_MAX_BYTES`].
    /// All parts are stored in one transaction, dirty, so they sync together. The conversation
    /// item itself is not touched; see [`ai_last_activity`](Self::ai_last_activity).
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::ItemNotFound`] if the conversation does not exist; storage
    /// errors.
    pub fn ai_append_entry(&mut self, conversation_id: &str, json: &str) -> Result<String> {
        if !self.is_unlocked() {
            return Err(Error::Locked);
        }
        if !matches!(self.get(conversation_id), Some(Item::AiConversation(_))) {
            return Err(Error::ItemNotFound(conversation_id.to_owned()));
        }
        let now = self.clock.now_ms();
        let last = self
            .message_headers(conversation_id)
            .map(|(_, m)| m.entry_id.as_str())
            .max();
        let entry_id = next_entry_id(now, last)?;
        let slices = split_entry(conversation_id, &entry_id, json)?;
        let part_count = u32::try_from(slices.len())
            .map_err(|_| Error::InvalidItem("the entry is too large".into()))?;

        let Self {
            store, unlocked, ..
        } = self;
        let unlocked = unlocked.as_mut().ok_or(Error::Locked)?;
        let key = &unlocked.vault_key;
        let headers = store.transaction(|tx| {
            let mut headers = Vec::with_capacity(slices.len());
            for (part, slice) in (0..part_count).zip(&slices) {
                let id = new_id();
                let mut item = Item::AiMessage(AiMessage {
                    conversation_id: conversation_id.to_owned(),
                    entry_id: entry_id.clone(),
                    part,
                    part_count,
                    data: (*slice).to_owned(),
                    updated_at: now,
                });
                let plain = item.to_plaintext()?;
                if plain.len() > AI_PART_MAX_BYTES {
                    return Err(Error::InvalidItem(
                        "an entry part exceeds the size limit".into(),
                    ));
                }
                let envelope = seal(key, item_aad(&id).as_bytes(), &plain)?;
                tx.upsert_item_row(&ItemRow {
                    id: id.clone(),
                    envelope: Some(envelope.to_json()),
                    revision: 0,
                    deleted: false,
                    dirty: true,
                    updated_at: now,
                })?;
                item.strip_message_data();
                headers.push((id, item));
            }
            Ok(headers)
        })?;
        unlocked.items.extend(headers);
        Ok(entry_id)
    }

    /// When a conversation last saw activity: the latest `updated_at` of the conversation item
    /// and of its message parts, from memory (no decryption). 0 for an unknown conversation.
    #[must_use]
    pub fn ai_last_activity(&self, conversation_id: &str) -> i64 {
        let conversation = match self.get(conversation_id) {
            Some(Item::AiConversation(c)) => c.updated_at,
            _ => 0,
        };
        self.message_headers(conversation_id)
            .map(|(_, m)| m.updated_at)
            .fold(conversation, i64::max)
    }

    /// Deletes a conversation and all of its message parts, leaving tombstones that sync.
    ///
    /// Parts without a conversation item (it was deleted on another device while this one added
    /// entries) are deleted too.
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::ItemNotFound`] if there is neither a conversation nor parts;
    /// [`Error::InvalidItem`] if the id belongs to another type of item.
    pub fn ai_delete_conversation(&mut self, conversation_id: &str) -> Result<()> {
        if !self.is_unlocked() {
            return Err(Error::Locked);
        }
        let mut ids: Vec<String> = match self.get(conversation_id) {
            Some(Item::AiConversation(_)) => vec![conversation_id.to_owned()],
            Some(_) => return Err(Error::InvalidItem("not a conversation".into())),
            None => Vec::new(),
        };
        ids.extend(
            self.message_headers(conversation_id)
                .map(|(id, _)| id.to_owned()),
        );
        if ids.is_empty() {
            return Err(Error::ItemNotFound(conversation_id.to_owned()));
        }
        self.tombstone(&ids)
    }

    /// Deletes the entry `entry_id` of a conversation and every later entry (AI-26), leaving
    /// tombstones that sync, in one transaction. Returns how many entries were deleted.
    ///
    /// Entries are ordered by `entry_id`, so "later" means a greater id, wherever it was written.
    /// The conversation item is not touched; a `context_start` that pointed at a deleted entry is
    /// the caller's to move.
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::ItemNotFound`] if there is no such conversation or it has no
    /// entry `entry_id`; storage errors.
    pub fn ai_delete_entries_from(
        &mut self,
        conversation_id: &str,
        entry_id: &str,
    ) -> Result<usize> {
        if !self.is_unlocked() {
            return Err(Error::Locked);
        }
        if !matches!(self.get(conversation_id), Some(Item::AiConversation(_))) {
            return Err(Error::ItemNotFound(conversation_id.to_owned()));
        }
        let mut entries = std::collections::BTreeSet::new();
        let mut ids = Vec::new();
        let mut found = false;
        for (id, header) in self.message_headers(conversation_id) {
            if header.entry_id.as_str() >= entry_id {
                found |= header.entry_id == entry_id;
                entries.insert(header.entry_id.clone());
                ids.push(id.to_owned());
            }
        }
        if !found {
            return Err(Error::ItemNotFound(entry_id.to_owned()));
        }
        self.tombstone(&ids)?;
        Ok(entries.len())
    }

    /// Deletes a skill and its files, leaving tombstones that sync.
    ///
    /// # Errors
    /// [`Error::Locked`]; [`Error::ItemNotFound`] if there is no such skill;
    /// [`Error::InvalidItem`] if the id belongs to another type of item.
    pub fn skill_delete(&mut self, skill_id: &str) -> Result<()> {
        if !self.is_unlocked() {
            return Err(Error::Locked);
        }
        match self.get(skill_id) {
            Some(Item::Skill(_)) => {}
            Some(_) => return Err(Error::InvalidItem("not a skill".into())),
            None => return Err(Error::ItemNotFound(skill_id.to_owned())),
        }
        let mut ids = vec![skill_id.to_owned()];
        ids.extend(self.skill_files(skill_id).into_iter().map(|(id, _)| id));
        self.tombstone(&ids)
    }

    /// Tombstones the given items in one transaction and drops them (wiped) from the item map.
    fn tombstone(&mut self, ids: &[String]) -> Result<()> {
        let now = self.clock.now_ms();
        let Self {
            store, unlocked, ..
        } = self;
        let unlocked = unlocked.as_mut().ok_or(Error::Locked)?;
        store.transaction(|tx| {
            for id in ids {
                let Some(row) = tx.item_row(id)?.filter(|r| !r.deleted) else {
                    continue;
                };
                tx.upsert_item_row(&ItemRow {
                    id: id.clone(),
                    envelope: None,
                    revision: row.revision,
                    deleted: true,
                    dirty: true,
                    updated_at: now.max(row.updated_at.saturating_add(1)),
                })?;
            }
            Ok(())
        })?;
        for id in ids {
            if let Some(mut item) = unlocked.items.remove(id) {
                item.zeroize();
            }
        }
        Ok(())
    }
}

// ---- entry ids ----------------------------------------------------------------------------

/// A UUIDv7 for the clock reading `now_ms`, strictly greater than `after` when that is a UUID.
///
/// The random bits of a fresh id may fall below the previous id of the same millisecond (or the
/// previous id may come from a device whose clock is ahead), so the id is then derived from
/// `after` instead.
fn next_entry_id(now_ms: i64, after: Option<&str>) -> Result<String> {
    let millis = u64::try_from(now_ms).unwrap_or(0);
    let fresh = Builder::from_unix_timestamp_millis(millis, &random_bytes::<10>()?).into_uuid();
    let id = match after.and_then(|s| Uuid::parse_str(s).ok()) {
        Some(prev) if prev >= fresh => successor(prev),
        _ => fresh,
    };
    Ok(id.to_string())
}

/// The next UUIDv7 after `prev`: the 74 bits after the timestamp count up, carrying into the
/// timestamp (the version and variant bits stay put). Anything that is not a UUIDv7 of this
/// shape gets the plain numeric successor, which is still greater.
fn successor(prev: Uuid) -> Uuid {
    const RAND_B_BITS: u32 = 62;
    const COUNTER_BITS: u32 = 74;
    let v = prev.as_u128();
    let mut millis = v >> 80;
    let rand_a = (v >> 64) & 0xfff;
    let rand_b = v & ((1 << RAND_B_BITS) - 1);
    let mut counter = ((rand_a << RAND_B_BITS) | rand_b) + 1;
    if counter >> COUNTER_BITS != 0 {
        counter = 0;
        millis += 1;
    }
    let next = Uuid::from_u128(
        (millis << 80)
            | (0x7 << 76)
            | ((counter >> RAND_B_BITS) << 64)
            | (0b10 << 62)
            | (counter & ((1 << RAND_B_BITS) - 1)),
    );
    if next > prev {
        next
    } else {
        Uuid::from_u128(prev.as_u128().saturating_add(1))
    }
}

// ---- splitting ----------------------------------------------------------------------------

/// How many bytes `c` takes inside a JSON string as `serde_json` writes it.
fn escaped_len(c: char) -> usize {
    match c {
        '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
        c if c < ' ' => 6,
        c => c.len_utf8(),
    }
}

/// Cuts `json` into slices (on character boundaries, never empty unless `json` is) such that each
/// one, as the `data` of an `ai_message` item of this conversation and entry, serialises to at
/// most [`AI_PART_MAX_BYTES`].
fn split_entry<'a>(conversation_id: &str, entry_id: &str, json: &'a str) -> Result<Vec<&'a str>> {
    // The item without data, with the widest numbers it can carry.
    let template = Item::AiMessage(AiMessage {
        conversation_id: conversation_id.to_owned(),
        entry_id: entry_id.to_owned(),
        part: u32::MAX,
        part_count: u32::MAX,
        data: String::new(),
        updated_at: i64::MIN,
    });
    // `data` is written between quotes, and those are part of the template already.
    let budget = AI_PART_MAX_BYTES.saturating_sub(template.to_plaintext()?.len());
    if budget < 8 {
        return Err(Error::InvalidItem("malformed conversation id".into()));
    }
    let mut slices = Vec::new();
    let (mut start, mut used) = (0, 0);
    for (at, c) in json.char_indices() {
        let width = escaped_len(c);
        if used + width > budget {
            slices.push(&json[start..at]);
            start = at;
            used = 0;
        }
        used += width;
    }
    slices.push(&json[start..]);
    Ok(slices)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;

    use super::*;
    use crate::crypto::{KdfParams, open_json};
    use crate::model::{AiAuthHeader, AiModel, AiProtocol, McpTransport, SETTINGS_ID, SearchKind};
    use crate::vault::ManualClock;

    const PW: &str = "ai-vault-test-password";

    fn vault() -> (Vault, Arc<ManualClock>) {
        let clock = Arc::new(ManualClock::new(1_700_000_000_000));
        let mut vault = Vault::open_in_memory().unwrap().with_clock(clock.clone());
        vault
            .create_with_params(PW, KdfParams::for_tests())
            .unwrap();
        (vault, clock)
    }

    fn conversation(vault: &mut Vault, title: &str) -> String {
        vault
            .put(
                None,
                Item::AiConversation(AiConversation {
                    title: title.into(),
                    ..AiConversation::default()
                }),
            )
            .unwrap()
    }

    /// Every message item the map holds.
    fn cached_messages(vault: &Vault) -> Vec<AiMessage> {
        vault
            .items()
            .filter_map(|(_, i)| i.as_ai_message().cloned())
            .collect()
    }

    /// A string heavy on quotes, backslashes, control characters and multi-byte characters.
    fn nasty(chars: usize) -> String {
        const ALPHABET: [char; 12] = [
            '"', '\\', '\n', '\t', '\u{1}', 'a', '7', '日', '本', 'é', '😀', ' ',
        ];
        (0..chars)
            .map(|i| ALPHABET[(i * 7 + i / 5) % ALPHABET.len()])
            .collect()
    }

    #[test]
    fn escaped_len_matches_serde_json() {
        let mut chars: Vec<char> = (0u32..0x80).filter_map(char::from_u32).collect();
        chars.extend([
            'é',
            '日',
            '😀',
            '\u{7f}',
            '\u{ff}',
            '\u{2028}',
            '\u{10ffff}',
        ]);
        for c in chars {
            let encoded = serde_json::to_string(&c.to_string()).unwrap();
            assert_eq!(escaped_len(c), encoded.len() - 2, "char {:?}", c);
        }
    }

    #[test]
    fn small_entries_are_one_part() {
        let (mut vault, _clock) = vault();
        let conv = conversation(&mut vault, "t");
        let json = r#"{"created_at":1,"role":"user","text":"hi"}"#;
        let id = vault.ai_append_entry(&conv, json).unwrap();
        let entries = vault.ai_entries(&conv).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entry_id, id);
        assert_eq!(entries[0].json.as_str(), json);
        let headers = cached_messages(&vault);
        assert_eq!((headers[0].part, headers[0].part_count), (0, 1));
        // Empty JSON still makes an entry.
        let empty = vault.ai_append_entry(&conv, "").unwrap();
        assert_eq!(
            vault.ai_entries(&conv).unwrap().last().unwrap().entry_id,
            empty
        );
        assert!(
            vault
                .ai_entries(&conv)
                .unwrap()
                .last()
                .unwrap()
                .json
                .is_empty()
        );
    }

    #[test]
    fn a_large_entry_splits_within_the_limits_and_reassembles() {
        let (mut vault, _clock) = vault();
        let conv = conversation(&mut vault, "big");
        // About 300 KB of text; much of it escapes to two or six bytes, the rest is multi-byte.
        let body = nasty(110_000);
        let json = serde_json::to_string(&json!({"role":"tool","content":body})).unwrap();
        assert!(json.len() > 300 * 1024, "{} bytes", json.len());
        let id = vault.ai_append_entry(&conv, &json).unwrap();

        // Every stored part: plaintext at most 40 KB, envelope well under the 64 KB limit.
        let mut parts = 0;
        for row in vault.store.item_rows().unwrap() {
            if row.id == SETTINGS_ID || row.id == conv {
                continue;
            }
            let env = row.envelope.as_deref().unwrap();
            let plain = open_json(
                &vault.unlocked.as_ref().unwrap().vault_key,
                item_aad(&row.id).as_bytes(),
                env,
            )
            .unwrap();
            let part = Item::from_plaintext(&plain).unwrap();
            let part = part.as_ai_message().unwrap();
            parts += 1;
            assert!(
                plain.len() <= AI_PART_MAX_BYTES,
                "{} byte plaintext",
                plain.len()
            );
            if part.part + 1 < part.part_count {
                assert!(plain.len() > AI_PART_MAX_BYTES - 64, "parts are filled up");
            }
            assert!(env.len() < 56 * 1024, "{} byte envelope", env.len());
        }
        assert!(parts >= 8, "{parts} parts");
        let headers = cached_messages(&vault);
        assert_eq!(headers.len(), parts);
        assert!(headers.iter().all(|h| h.entry_id == id
            && usize::try_from(h.part_count).unwrap() == parts
            && h.data.is_empty()));

        // Byte-identical after reassembly, also after a lock and unlock.
        let entries = vault.ai_entries(&conv).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].json.as_str(), json);
        vault.lock();
        vault.unlock(PW).unwrap();
        assert_eq!(vault.ai_entries(&conv).unwrap()[0].json.as_str(), json);
        assert!(cached_messages(&vault).iter().all(|h| h.data.is_empty()));
    }

    #[test]
    fn the_split_respects_the_limit_for_every_alignment() {
        // Multi-byte and escaped characters straddling the budget must never overflow it.
        let (mut vault, _clock) = vault();
        let conv = conversation(&mut vault, "t");
        let entry_id = "0192f0aa-1111-7000-8000-000000000001";
        for lead in 0..8 {
            for filler in ['日', '😀', '"', '\u{1}', 'x'] {
                let json: String = std::iter::repeat_n('x', lead)
                    .chain(std::iter::repeat_n(filler, 30_000))
                    .collect();
                let slices = split_entry(&conv, entry_id, &json).unwrap();
                assert_eq!(slices.concat(), json);
                for (part, slice) in slices.iter().enumerate() {
                    let item = Item::AiMessage(AiMessage {
                        conversation_id: conv.clone(),
                        entry_id: entry_id.into(),
                        part: u32::try_from(part).unwrap(),
                        part_count: u32::try_from(slices.len()).unwrap(),
                        data: (*slice).into(),
                        updated_at: i64::MAX,
                    });
                    assert!(item.to_plaintext().unwrap().len() <= AI_PART_MAX_BYTES);
                }
            }
        }
    }

    #[test]
    fn memory_holds_no_message_data() {
        let (mut vault, _clock) = vault();
        let conv = conversation(&mut vault, "t");
        let secret = "the-conversation-secret-text";
        vault
            .ai_append_entry(&conv, &format!(r#"{{"role":"user","text":"{secret}"}}"#))
            .unwrap();
        // After put (including a direct put of a message item).
        let direct = vault
            .put(
                None,
                Item::AiMessage(AiMessage {
                    conversation_id: conv.clone(),
                    entry_id: "x".into(),
                    part_count: 1,
                    data: secret.into(),
                    ..AiMessage::default()
                }),
            )
            .unwrap();
        for (_, item) in vault.items() {
            assert!(!format!("{item:?}").contains(secret));
            assert!(!serde_json::to_string(item).unwrap().contains(secret));
        }
        assert_eq!(
            vault.get(&direct).unwrap().as_ai_message().unwrap().data,
            ""
        );
        // After unlock.
        vault.lock();
        vault.unlock(PW).unwrap();
        assert_eq!(cached_messages(&vault).len(), 2);
        for (_, item) in vault.items() {
            assert!(!serde_json::to_string(item).unwrap().contains(secret));
        }
        // The data is still in the store, readable.
        assert!(vault.ai_entries(&conv).unwrap()[0].json.contains(secret));
    }

    #[test]
    fn entry_ids_increase_even_within_one_millisecond() {
        let (mut vault, _clock) = vault(); // the manual clock never moves
        let conv = conversation(&mut vault, "t");
        let mut previous = String::new();
        for i in 0..500 {
            let id = vault
                .ai_append_entry(&conv, &format!(r#"{{"n":{i}}}"#))
                .unwrap();
            assert!(id > previous, "{id} <= {previous}");
            assert_eq!(Uuid::parse_str(&id).unwrap().get_version_num(), 7);
            previous = id;
        }
        let entries = vault.ai_entries(&conv).unwrap();
        assert_eq!(entries.len(), 500);
        for (i, e) in entries.iter().enumerate() {
            assert_eq!(e.json.as_str(), format!(r#"{{"n":{i}}}"#));
        }
    }

    #[test]
    fn entry_ids_stay_above_ids_from_a_clock_that_runs_ahead() {
        let (mut vault, clock) = vault();
        let conv = conversation(&mut vault, "t");
        let ahead = vault.ai_append_entry(&conv, "{}").unwrap();
        // This device's clock now reads an earlier time than the entry above carries.
        clock.advance(-86_400_000);
        let next = vault.ai_append_entry(&conv, "{}").unwrap();
        assert!(next > ahead);
        assert_eq!(Uuid::parse_str(&next).unwrap().get_version_num(), 7);
        let all = vault.ai_entries(&conv).unwrap();
        assert_eq!(
            all.iter().map(|e| e.entry_id.as_str()).collect::<Vec<_>>(),
            [ahead.as_str(), next.as_str()]
        );
    }

    #[test]
    fn successor_counts_up_and_carries() {
        let at = |counter: u128, millis: u128| {
            Uuid::from_u128(
                (millis << 80)
                    | (0x7 << 76)
                    | ((counter >> 62) << 64)
                    | (0b10 << 62)
                    | (counter & ((1 << 62) - 1)),
            )
        };
        let one = successor(at(41, 1000));
        assert_eq!(one, at(42, 1000));
        // The low field overflows into the high one.
        assert_eq!(successor(at((1 << 62) - 1, 1000)), at(1 << 62, 1000));
        // The whole counter overflows into the timestamp.
        assert_eq!(successor(at((1 << 74) - 1, 1000)), at(0, 1001));
        for u in [one, successor(at((1 << 74) - 1, 1000))] {
            assert_eq!(u.get_version_num(), 7);
            assert_eq!(u.get_variant(), uuid::Variant::RFC4122);
        }
        // Not a v7: still greater.
        let v4 = Uuid::parse_str("f81d4fae-7dec-41d0-a765-00a0c91e6bf6").unwrap();
        assert!(successor(v4) > v4);
        // A negative clock reading does not panic.
        assert!(next_entry_id(-5, None).is_ok());
    }

    #[test]
    fn incomplete_entries_are_skipped_until_the_rest_arrives() {
        let (mut vault, _clock) = vault();
        let conv = conversation(&mut vault, "t");
        let json = serde_json::to_string(&json!({"text": nasty(60_000)})).unwrap();
        let first = vault.ai_append_entry(&conv, r#"{"n":1}"#).unwrap();
        let big = vault.ai_append_entry(&conv, &json).unwrap();
        let last = vault.ai_append_entry(&conv, r#"{"n":3}"#).unwrap();
        let ids = |v: &Vault| -> Vec<String> {
            v.ai_entries(&conv)
                .unwrap()
                .into_iter()
                .map(|e| e.entry_id)
                .collect()
        };
        assert_eq!(ids(&vault), [first.clone(), big.clone(), last.clone()]);

        // Hide the second part of the big entry, as if it had not been synced yet.
        let (hidden_id, hidden) = {
            let (id, item) = vault
                .items()
                .find(|(_, i)| {
                    i.as_ai_message()
                        .is_some_and(|m| m.entry_id == big && m.part == 1)
                })
                .unwrap();
            (id.to_owned(), item.clone())
        };
        vault.unlocked.as_mut().unwrap().items.remove(&hidden_id);
        assert_eq!(ids(&vault), [first.clone(), last.clone()]);

        // It arrives: the entry appears, in order.
        vault
            .unlocked
            .as_mut()
            .unwrap()
            .items
            .insert(hidden_id, hidden);
        assert_eq!(ids(&vault), [first.clone(), big.clone(), last.clone()]);
        assert_eq!(vault.ai_entries(&conv).unwrap()[1].json.as_str(), json);

        // A part that cannot be read (here: its row is gone) skips the entry without failing.
        let other = {
            let (id, _) = vault
                .items()
                .find(|(_, i)| {
                    i.as_ai_message()
                        .is_some_and(|m| m.entry_id == big && m.part == 0)
                })
                .unwrap();
            id.to_owned()
        };
        vault
            .store
            .upsert_item_row(&ItemRow {
                id: other,
                envelope: Some(
                    r#"{"v":1,"n":"AAAAAAAAAAAAAAAA","c":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"}"#
                        .into(),
                ),
                revision: 0,
                deleted: false,
                dirty: false,
                updated_at: 1,
            })
            .unwrap();
        assert_eq!(ids(&vault), [first, last]);
    }

    #[test]
    fn inconsistent_or_duplicated_parts_do_not_confuse_the_join() {
        let (mut vault, _clock) = vault();
        let conv = conversation(&mut vault, "t");
        let id = vault.ai_append_entry(&conv, r#"{"n":1}"#).unwrap();
        // Two items claim to be part 0 of a two-part entry, and part 1 is missing.
        for _ in 0..2 {
            vault
                .put(
                    None,
                    Item::AiMessage(AiMessage {
                        conversation_id: conv.clone(),
                        entry_id: "zzz-two-part".into(),
                        part: 0,
                        part_count: 2,
                        data: "x".into(),
                        ..AiMessage::default()
                    }),
                )
                .unwrap();
        }
        // A part that disagrees about the part count.
        vault
            .put(
                None,
                Item::AiMessage(AiMessage {
                    conversation_id: conv.clone(),
                    entry_id: "zzz-liar".into(),
                    part: 0,
                    part_count: 1,
                    data: "x".into(),
                    ..AiMessage::default()
                }),
            )
            .unwrap();
        vault
            .put(
                None,
                Item::AiMessage(AiMessage {
                    conversation_id: conv.clone(),
                    entry_id: "zzz-liar".into(),
                    part: 1,
                    part_count: 2,
                    data: "y".into(),
                    ..AiMessage::default()
                }),
            )
            .unwrap();
        // A zero part count.
        vault
            .put(
                None,
                Item::AiMessage(AiMessage {
                    conversation_id: conv.clone(),
                    entry_id: "zzz-zero".into(),
                    ..AiMessage::default()
                }),
            )
            .unwrap();
        let entries = vault.ai_entries(&conv).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].entry_id, id);
    }

    #[test]
    fn append_needs_a_conversation_and_an_unlocked_vault() {
        let (mut vault, _clock) = vault();
        assert!(matches!(
            vault.ai_append_entry("missing", "{}"),
            Err(Error::ItemNotFound(_))
        ));
        let host = vault
            .put(None, Item::Host(crate::model::Host::default()))
            .unwrap();
        assert!(matches!(
            vault.ai_append_entry(&host, "{}"),
            Err(Error::ItemNotFound(_))
        ));
        let conv = conversation(&mut vault, "t");
        let pending = vault.pending_count();
        vault.lock();
        assert!(matches!(
            vault.ai_append_entry(&conv, "{}"),
            Err(Error::Locked)
        ));
        assert!(matches!(vault.ai_entries(&conv), Err(Error::Locked)));
        assert!(matches!(
            vault.ai_delete_conversation(&conv),
            Err(Error::Locked)
        ));
        assert!(matches!(vault.skill_delete("x"), Err(Error::Locked)));
        assert_eq!(vault.ai_last_activity(&conv), 0);
        vault.unlock(PW).unwrap();
        assert_eq!(vault.pending_count(), pending);
    }

    #[test]
    fn appended_parts_are_dirty_and_last_activity_follows_them() {
        let (mut vault, clock) = vault();
        let conv = conversation(&mut vault, "t");
        let created = vault.get(&conv).unwrap().updated_at();
        assert_eq!(vault.ai_last_activity(&conv), created);
        assert_eq!(vault.ai_last_activity("unknown"), 0);

        let before = vault.pending_count();
        clock.advance(5_000);
        vault.ai_append_entry(&conv, r#"{"n":1}"#).unwrap();
        assert_eq!(vault.pending_count(), before + 1);
        assert_eq!(vault.ai_last_activity(&conv), created + 5_000);

        // Another conversation's entries do not count.
        let other = conversation(&mut vault, "other");
        clock.advance(5_000);
        vault.ai_append_entry(&other, r#"{"n":2}"#).unwrap();
        assert_eq!(vault.ai_last_activity(&conv), created + 5_000);
        assert_eq!(vault.ai_entries(&conv).unwrap().len(), 1);
        assert_eq!(vault.ai_entries(&other).unwrap().len(), 1);

        // The conversation item itself later than its entries.
        clock.advance(5_000);
        let mut item = vault.get(&conv).unwrap().clone();
        if let Item::AiConversation(c) = &mut item {
            c.pinned = true;
        }
        vault.put(Some(&conv), item).unwrap();
        assert_eq!(vault.ai_last_activity(&conv), created + 15_000);
    }

    #[test]
    fn deleting_a_conversation_tombstones_every_part() {
        let (mut vault, clock) = vault();
        let conv = conversation(&mut vault, "doomed");
        let keep = conversation(&mut vault, "kept");
        let json = serde_json::to_string(&json!({"text": nasty(70_000)})).unwrap();
        vault.ai_append_entry(&conv, &json).unwrap();
        vault.ai_append_entry(&conv, r#"{"n":2}"#).unwrap();
        vault.ai_append_entry(&keep, r#"{"n":3}"#).unwrap();
        let part_ids: Vec<String> = vault
            .items()
            .filter(|(_, i)| i.as_ai_message().is_some_and(|m| m.conversation_id == conv))
            .map(|(id, _)| id.to_owned())
            .collect();
        assert!(part_ids.len() >= 4, "{} parts", part_ids.len());

        clock.advance(1_000);
        vault.ai_delete_conversation(&conv).unwrap();
        assert!(vault.get(&conv).is_none());
        assert!(vault.ai_entries(&conv).unwrap().is_empty());
        assert_eq!(vault.ai_last_activity(&conv), 0);
        for id in part_ids.iter().chain([&conv]) {
            assert!(vault.get(id).is_none());
            let row = vault.store.item_row(id).unwrap().unwrap();
            assert!(row.deleted && row.dirty && row.envelope.is_none(), "{id}");
        }
        // Nothing else was touched.
        assert!(vault.get(&keep).is_some());
        assert_eq!(vault.ai_entries(&keep).unwrap().len(), 1);
        // A second delete finds nothing; so does an id of another type.
        assert!(matches!(
            vault.ai_delete_conversation(&conv),
            Err(Error::ItemNotFound(_))
        ));
        let host = vault
            .put(None, Item::Host(crate::model::Host::default()))
            .unwrap();
        assert!(matches!(
            vault.ai_delete_conversation(&host),
            Err(Error::InvalidItem(_))
        ));
        assert!(vault.get(&host).is_some());
    }

    #[test]
    fn deleting_entries_from_one_removes_it_and_every_later_entry() {
        let (mut vault, clock) = vault();
        let conv = conversation(&mut vault, "edit");
        let other = conversation(&mut vault, "other");
        let first = vault.ai_append_entry(&conv, r#"{"n":1}"#).unwrap();
        let json = serde_json::to_string(&json!({"text": nasty(70_000)})).unwrap();
        let second = vault.ai_append_entry(&conv, &json).unwrap();
        vault.ai_append_entry(&conv, r#"{"n":3}"#).unwrap();
        vault.ai_append_entry(&other, r#"{"n":4}"#).unwrap();
        let parts_of = |vault: &Vault, entry: &str| -> Vec<String> {
            vault
                .items()
                .filter(|(_, i)| i.as_ai_message().is_some_and(|m| m.entry_id == entry))
                .map(|(id, _)| id.to_owned())
                .collect()
        };
        let second_parts = parts_of(&vault, &second);
        assert!(second_parts.len() >= 2);

        clock.advance(1_000);
        assert_eq!(vault.ai_delete_entries_from(&conv, &second).unwrap(), 2);
        let left = vault.ai_entries(&conv).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].entry_id, first);
        for id in &second_parts {
            let row = vault.store.item_row(id).unwrap().unwrap();
            assert!(row.deleted && row.dirty && row.envelope.is_none(), "{id}");
        }
        assert_eq!(vault.ai_entries(&other).unwrap().len(), 1);
        assert!(vault.get(&conv).is_some());

        // A new entry still sorts after everything the conversation ever had.
        let next = vault.ai_append_entry(&conv, r#"{"n":5}"#).unwrap();
        assert!(next > second);

        assert!(matches!(
            vault.ai_delete_entries_from(&conv, &second),
            Err(Error::ItemNotFound(_))
        ));
        assert!(matches!(
            vault.ai_delete_entries_from("missing", &first),
            Err(Error::ItemNotFound(_))
        ));
        vault.lock();
        assert!(matches!(
            vault.ai_delete_entries_from(&conv, &first),
            Err(Error::Locked)
        ));
    }

    #[test]
    fn orphaned_parts_are_deleted_too() {
        let (mut vault, _clock) = vault();
        let conv = conversation(&mut vault, "t");
        vault.ai_append_entry(&conv, "{}").unwrap();
        // The conversation item disappears (deleted elsewhere), the part stays.
        vault.delete(&conv).unwrap();
        assert!(vault.ai_last_activity(&conv) > 0);
        vault.ai_delete_conversation(&conv).unwrap();
        assert_eq!(vault.ai_last_activity(&conv), 0);
        assert!(matches!(
            vault.ai_delete_conversation(&conv),
            Err(Error::ItemNotFound(_))
        ));
    }

    #[test]
    fn deleting_a_skill_deletes_its_files() {
        let (mut vault, _clock) = vault();
        let skill = vault
            .put(
                None,
                Item::Skill(Skill {
                    name: "nginx".into(),
                    description: "d".into(),
                    enabled: true,
                    ..Skill::default()
                }),
            )
            .unwrap();
        let other = vault
            .put(
                None,
                Item::Skill(Skill {
                    name: "other".into(),
                    ..Skill::default()
                }),
            )
            .unwrap();
        let file = |skill_id: &str, path: &str| {
            Item::SkillFile(SkillFile {
                skill_id: skill_id.into(),
                path: path.into(),
                content: "text".into(),
                ..SkillFile::default()
            })
        };
        let f1 = vault.put(None, file(&skill, "SKILL.md")).unwrap();
        let f2 = vault.put(None, file(&skill, "references/a.md")).unwrap();
        let f3 = vault.put(None, file(&other, "SKILL.md")).unwrap();
        assert_eq!(vault.skills().len(), 2);
        assert_eq!(vault.skill_files(&skill).len(), 2);

        vault.skill_delete(&skill).unwrap();
        assert_eq!(vault.skills().len(), 1);
        assert!(vault.skill_files(&skill).is_empty());
        assert_eq!(vault.skill_files(&other).len(), 1);
        for id in [&skill, &f1, &f2] {
            assert!(vault.get(id).is_none());
            assert!(vault.store.item_row(id).unwrap().unwrap().deleted);
        }
        assert!(vault.get(&f3).is_some());
        assert!(matches!(
            vault.skill_delete(&skill),
            Err(Error::ItemNotFound(_))
        ));
        assert!(matches!(
            vault.skill_delete(&f3),
            Err(Error::InvalidItem(_))
        ));
    }

    #[test]
    fn typed_accessors_return_their_own_kind() {
        let (mut vault, _clock) = vault();
        vault
            .put(
                None,
                Item::AiProvider(AiProvider {
                    name: "p".into(),
                    models: vec![AiModel {
                        id: "m".into(),
                        ..AiModel::default()
                    }],
                    ..AiProvider::default()
                }),
            )
            .unwrap();
        vault
            .put(None, Item::SearchProvider(SearchProvider::default()))
            .unwrap();
        vault
            .put(
                None,
                Item::McpServer(McpServer {
                    name: "m".into(),
                    ..McpServer::default()
                }),
            )
            .unwrap();
        conversation(&mut vault, "c");
        assert_eq!(vault.ai_providers().len(), 1);
        assert_eq!(vault.ai_providers()[0].1.models[0].id, "m");
        assert_eq!(vault.search_providers().len(), 1);
        assert_eq!(vault.mcp_servers().len(), 1);
        assert_eq!(vault.ai_conversations().len(), 1);
        assert!(vault.skills().is_empty());
        assert_eq!(
            (
                AiProtocol::default(),
                AiAuthHeader::default(),
                SearchKind::default()
            ),
            (
                AiProtocol::ChatCompletions,
                AiAuthHeader::XApiKey,
                SearchKind::Brave
            )
        );
        assert_eq!(McpTransport::default().kind_str(), "stdio");
    }

    #[test]
    fn a_providers_api_key_never_appears_in_debug_or_the_store() {
        let (mut vault, _clock) = vault();
        let key = "sk-test-0123456789-very-secret";
        let id = vault
            .put(
                None,
                Item::AiProvider(AiProvider {
                    name: "openai".into(),
                    api_key: Zeroizing::new(key.into()),
                    ..AiProvider::default()
                }),
            )
            .unwrap();
        let provider = vault.ai_providers().remove(0).1;
        for text in [
            format!("{provider:?}"),
            format!("{:?}", Item::AiProvider(provider.clone())),
            format!("{:?}", vault.ai_providers()),
            format!("{:?}", vault.get(&id)),
        ] {
            assert!(!text.contains(key), "{text}");
            assert!(text.contains("<redacted>"));
        }
        assert_eq!(provider.api_key.as_str(), key);
        let row = vault.store.item_row(&id).unwrap().unwrap();
        assert!(!row.envelope.unwrap().contains("sk-test"));
    }
}
