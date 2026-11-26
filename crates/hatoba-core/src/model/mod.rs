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
//! [`Proxy`] holds a proxy password and has a redacting `Debug` too (SSH-13).
//!
//! The AI assistant adds eight item types (spec §5.1, §13): [`AiProvider`] and [`SearchProvider`]
//! hold API keys, [`McpServer`] holds environment and header values, and [`AiMessage`] holds a
//! slice of conversation content (SEC-04). All four have a redacting `Debug`. An `ai_message`'s
//! `data` is only held while an entry is being read or written: the in-memory item map keeps the
//! header alone (see [`Item::strip_message_data`]).

mod ai;
mod host;
mod item;
mod settings;
mod ssh;

#[cfg(test)]
mod tests;

pub use ai::*;
pub use host::*;
pub use item::*;
pub use settings::*;
pub use ssh::*;
