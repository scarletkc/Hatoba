//! Stored conversation entries (spec §13.7).
//!
//! An entry is written once and never changes. Its JSON is exactly what the desktop shell stores
//! in `ai_message` items, and every request is built from these entries, so continuing a
//! conversation on another device sends the same context. Every struct tolerates missing fields
//! so devices on different app versions can read each other's entries.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

/// One stored entry: `{ "created_at": …, "role": …, … }`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AiEntry {
    /// Creation time, Unix ms.
    #[serde(default)]
    pub created_at: i64,
    /// The role-tagged body, flattened next to `created_at`.
    #[serde(flatten)]
    pub body: EntryBody,
}

/// The role-specific part of an entry, tagged by `role`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "snake_case")]
pub enum EntryBody {
    /// A message the user sent.
    User {
        /// The message text.
        #[serde(default)]
        text: String,
    },
    /// A model response.
    Assistant(AssistantEntry),
    /// The result of one tool call.
    Tool {
        /// Id of the [`ToolCall`] this answers.
        #[serde(default)]
        tool_call_id: String,
        /// How the call ended.
        #[serde(default)]
        status: ToolStatus,
        /// The result text, already shortened with
        /// [`truncate_result`](crate::tools::truncate_result) where that applies. For
        /// [`ToolStatus::Rejected`] it is the user's reason (may be empty); the adapter adds the
        /// wording that tells the model the call was rejected or cancelled.
        #[serde(default)]
        content: String,
    },
    /// A summary written by Compact (AI-21); the context starts here afterwards.
    Summary {
        /// The summary text.
        #[serde(default)]
        text: String,
    },
}

/// A model response as stored (the `assistant` entry).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AssistantEntry {
    /// The provider item id that produced it. With `model_id`, decides whether `raw` is replayed.
    pub provider_id: String,
    /// The model id that produced it.
    pub model_id: String,
    /// The answer text (all text blocks, in order).
    pub text: String,
    /// Reasoning the response carried, for display only (AI-06). It goes back to the model only
    /// inside `raw`.
    pub reasoning: Option<String>,
    /// Tool calls, in the order the model made them.
    pub tool_calls: Vec<ToolCall>,
    /// How the response ended.
    pub finish: Finish,
    /// Token usage of this response (AI-20).
    pub usage: Option<Usage>,
    /// The assistant message as the provider returned it, every field kept: the Chat Completions
    /// message object, or the Anthropic content block array.
    pub raw: serde_json::Value,
}

/// A tool call made by the model.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolCall {
    /// The provider's call id; the `tool` entry answers it.
    pub id: String,
    /// Tool name, e.g. `run_command`.
    pub name: String,
    /// The arguments as JSON text (usually an object). May be invalid JSON when the response was
    /// cut off.
    pub arguments: String,
}

/// How a response ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    /// The model finished its answer.
    #[default]
    Stop,
    /// The model wants tool results before going on.
    ToolCalls,
    /// Cut off at the output limit (or the context window).
    Length,
    /// Declined by the model or the provider's filter.
    Refused,
}

/// Token usage of one response (AI-20).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    /// Input tokens, cached ones included.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// True when estimated from text length because the provider reported no usage.
    pub estimated: bool,
    /// Of the input tokens, those read from the provider's prompt cache, when it said so.
    /// Stored only when not zero.
    #[serde(skip_serializing_if = "is_zero")]
    pub cache_read_tokens: u64,
    /// Of the input tokens, those written to the provider's prompt cache, when it said so.
    /// Stored only when not zero.
    #[serde(skip_serializing_if = "is_zero")]
    pub cache_write_tokens: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// How a tool call ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    /// The tool ran and returned a result.
    #[default]
    Ok,
    /// The tool failed; `content` says why.
    Error,
    /// The user rejected the call; `content` is the user's reason, possibly empty.
    Rejected,
    /// The call was stopped or never ran (stop, lock, crash, merged entries).
    Cancelled,
}

impl AiEntry {
    /// A `user` entry.
    #[must_use]
    pub fn user(created_at: i64, text: impl Into<String>) -> Self {
        Self {
            created_at,
            body: EntryBody::User { text: text.into() },
        }
    }

    /// An `assistant` entry.
    #[must_use]
    pub fn assistant(created_at: i64, entry: AssistantEntry) -> Self {
        Self {
            created_at,
            body: EntryBody::Assistant(entry),
        }
    }

    /// A `tool` entry.
    #[must_use]
    pub fn tool(
        created_at: i64,
        tool_call_id: impl Into<String>,
        status: ToolStatus,
        content: impl Into<String>,
    ) -> Self {
        Self {
            created_at,
            body: EntryBody::Tool {
                tool_call_id: tool_call_id.into(),
                status,
                content: content.into(),
            },
        }
    }

    /// A `summary` entry.
    #[must_use]
    pub fn summary(created_at: i64, text: impl Into<String>) -> Self {
        Self {
            created_at,
            body: EntryBody::Summary { text: text.into() },
        }
    }

    /// The `cancelled` result stored for a call that has none (spec §13.1). Its content is empty;
    /// the adapter tells the model the call was cancelled.
    #[must_use]
    pub fn cancelled_result(created_at: i64, tool_call_id: impl Into<String>) -> Self {
        Self::tool(
            created_at,
            tool_call_id,
            ToolStatus::Cancelled,
            String::new(),
        )
    }
}

/// The ids of tool calls that have no `tool` entry yet, in conversation order (spec §13.1).
///
/// The desktop shell stores an [`AiEntry::cancelled_result`] for each before every request, e.g.
/// after a crash, or after entries from two devices merged. A call is answered by the first
/// result with its id stored after it that no earlier call took, so a server that reuses call
/// ids across responses is handled; such an id is listed once per unanswered call.
#[must_use]
pub fn fix_up_missing_results(entries: &[AiEntry]) -> Vec<String> {
    let pairs = pair_results(entries);
    let mut missing = Vec::new();
    for (i, entry) in entries.iter().enumerate() {
        if let EntryBody::Assistant(a) = &entry.body {
            for (c, call) in a.tool_calls.iter().enumerate() {
                if !pairs.contains_key(&(i, c)) {
                    missing.push(call.id.clone());
                }
            }
        }
    }
    missing
}

/// For each tool call, keyed by `(assistant entry index, call index)`, the index of the `tool`
/// entry that answers it: the first result with that id stored after the call and not taken by
/// an earlier call.
pub(crate) fn pair_results(entries: &[AiEntry]) -> HashMap<(usize, usize), usize> {
    let mut results: HashMap<&str, VecDeque<usize>> = HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        if let EntryBody::Tool { tool_call_id, .. } = &entry.body {
            results
                .entry(tool_call_id.as_str())
                .or_default()
                .push_back(i);
        }
    }
    let mut pairs = HashMap::new();
    for (i, entry) in entries.iter().enumerate() {
        let EntryBody::Assistant(a) = &entry.body else {
            continue;
        };
        for (c, call) in a.tool_calls.iter().enumerate() {
            let Some(queue) = results.get_mut(call.id.as_str()) else {
                continue;
            };
            while queue.front().is_some_and(|&pos| pos < i) {
                queue.pop_front();
            }
            if let Some(pos) = queue.pop_front() {
                pairs.insert((i, c), pos);
            }
        }
    }
    pairs
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn wire_shape_matches_the_spec() {
        let entry = AiEntry::assistant(
            7,
            AssistantEntry {
                provider_id: "p".into(),
                model_id: "m".into(),
                text: "hi".into(),
                reasoning: Some("think".into()),
                tool_calls: vec![ToolCall {
                    id: "c1".into(),
                    name: "run_command".into(),
                    arguments: r#"{"command":"ls"}"#.into(),
                }],
                finish: Finish::ToolCalls,
                usage: Some(Usage {
                    input_tokens: 10,
                    output_tokens: 2,
                    estimated: false,
                    ..Usage::default()
                }),
                raw: json!({"role": "assistant"}),
            },
        );
        let value = serde_json::to_value(&entry).unwrap();
        assert_eq!(
            value,
            json!({
                "created_at": 7,
                "role": "assistant",
                "provider_id": "p",
                "model_id": "m",
                "text": "hi",
                "reasoning": "think",
                "tool_calls": [{"id": "c1", "name": "run_command", "arguments": "{\"command\":\"ls\"}"}],
                "finish": "tool_calls",
                "usage": {"input_tokens": 10, "output_tokens": 2, "estimated": false},
                "raw": {"role": "assistant"}
            })
        );
        let back: AiEntry = serde_json::from_value(value).unwrap();
        assert_eq!(back, entry);

        // Cache counts are stored only when the provider reported some.
        let cached = Usage {
            input_tokens: 10,
            output_tokens: 2,
            estimated: false,
            cache_read_tokens: 8,
            cache_write_tokens: 0,
        };
        let value = serde_json::to_value(cached).unwrap();
        assert_eq!(
            value,
            json!({"input_tokens": 10, "output_tokens": 2, "estimated": false, "cache_read_tokens": 8})
        );
        assert_eq!(serde_json::from_value::<Usage>(value).unwrap(), cached);

        for (entry, expected) in [
            (
                AiEntry::user(1, "hello"),
                json!({"created_at": 1, "role": "user", "text": "hello"}),
            ),
            (
                AiEntry::tool(2, "c1", ToolStatus::Rejected, "no"),
                json!({"created_at": 2, "role": "tool", "tool_call_id": "c1", "status": "rejected", "content": "no"}),
            ),
            (
                AiEntry::summary(3, "s"),
                json!({"created_at": 3, "role": "summary", "text": "s"}),
            ),
        ] {
            assert_eq!(serde_json::to_value(&entry).unwrap(), expected);
            assert_eq!(serde_json::from_value::<AiEntry>(expected).unwrap(), entry);
        }
    }

    #[test]
    fn missing_fields_and_unknown_fields_are_tolerated() {
        let entry: AiEntry =
            serde_json::from_value(json!({"role": "assistant", "future": 1})).unwrap();
        assert_eq!(entry.created_at, 0);
        let EntryBody::Assistant(a) = entry.body else {
            panic!("expected an assistant entry");
        };
        assert_eq!(a.finish, Finish::Stop);
        assert!(a.raw.is_null() && a.usage.is_none() && a.tool_calls.is_empty());

        let tool: AiEntry =
            serde_json::from_value(json!({"created_at": 1, "role": "tool", "tool_call_id": "x"}))
                .unwrap();
        assert_eq!(tool, AiEntry::tool(1, "x", ToolStatus::Ok, ""));
        assert!(serde_json::from_value::<AiEntry>(json!({"role": "system"})).is_err());
    }

    #[test]
    fn calls_without_results_are_listed_in_order() {
        let call = |id: &str| ToolCall {
            id: id.into(),
            name: "run_command".into(),
            arguments: "{}".into(),
        };
        let assistant = |calls: Vec<ToolCall>| {
            AiEntry::assistant(
                0,
                AssistantEntry {
                    tool_calls: calls,
                    finish: Finish::ToolCalls,
                    ..AssistantEntry::default()
                },
            )
        };
        let entries = vec![
            AiEntry::user(0, "go"),
            assistant(vec![call("a"), call("b")]),
            AiEntry::tool(0, "a", ToolStatus::Ok, "done"),
            assistant(vec![call("c"), call("b")]),
        ];
        // Both calls named "b" lack a result.
        assert_eq!(fix_up_missing_results(&entries), vec!["b", "c", "b"]);
        let mut fixed = entries.clone();
        fixed.extend(
            fix_up_missing_results(&entries)
                .into_iter()
                .map(|id| AiEntry::cancelled_result(1, id)),
        );
        assert!(fix_up_missing_results(&fixed).is_empty());

        // A server that reuses ids: the old result does not answer the new call.
        let reused = vec![
            assistant(vec![call("call_0")]),
            AiEntry::tool(0, "call_0", ToolStatus::Ok, "first"),
            AiEntry::user(0, "again"),
            assistant(vec![call("call_0")]),
        ];
        assert_eq!(fix_up_missing_results(&reused), vec!["call_0"]);
        // A result stored before its call (out of order) never pairs with it.
        let early = vec![
            AiEntry::tool(0, "x", ToolStatus::Ok, "orphan"),
            assistant(vec![call("x")]),
        ];
        assert_eq!(fix_up_missing_results(&early), vec!["x"]);
    }
}
