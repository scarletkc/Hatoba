//! Conversations, entries and turn events as the WebView gets them, and failures as the `error`
//! event and the logs name them.

use hatoba_ai::AiError;
use hatoba_ai::chat::StreamEvent;
use hatoba_ai::entry::{AiEntry, EntryBody, Finish, ToolStatus};
use hatoba_core::Vault;
use hatoba_core::model::AiConversation;

use super::judge::auto_view;
use super::request::effort_view;
use crate::dto::{
    AiConversationView, AiEntryView, AiFinish, AiToolCall, AiToolStatus, AiTurnEvent, AiUsage,
    QuickTarget,
};

pub fn conversation_view(v: &Vault, id: &str, c: &AiConversation) -> AiConversationView {
    conversation_view_at(id, c, v.ai_last_activity(id))
}

/// [`conversation_view`] with the conversation's last activity read already, for lists
/// (`Vault::ai_last_activities` reads every conversation's in one pass).
pub fn conversation_view_at(
    id: &str,
    c: &AiConversation,
    last_activity: i64,
) -> AiConversationView {
    AiConversationView {
        id: id.to_owned(),
        title: c.title.clone(),
        host_id: c.host_id.clone(),
        quick_target: c.quick_target.as_ref().map(|t| QuickTarget {
            address: t.address.clone(),
            port: t.port,
            username: t.username.clone(),
        }),
        origin: c.origin.clone(),
        local: c.local.clone(),
        pinned: c.pinned,
        context_start: c.context_start.clone(),
        effort: c.effort.map(effort_view),
        created_at: c.created_at,
        updated_at: c.updated_at,
        last_activity,
    }
}

/// An entry for the WebView: everything but the provider's raw message.
pub fn entry_view(entry_id: &str, entry: &AiEntry) -> AiEntryView {
    let entry_id = entry_id.to_owned();
    let created_at = entry.created_at;
    match &entry.body {
        EntryBody::User { text, .. } => AiEntryView::User {
            entry_id,
            created_at,
            text: text.clone(),
        },
        EntryBody::Assistant(a) => AiEntryView::Assistant {
            entry_id,
            created_at,
            provider_id: a.provider_id.clone(),
            model_id: a.model_id.clone(),
            text: a.text.clone(),
            reasoning: a.reasoning.clone(),
            tool_calls: a
                .tool_calls
                .iter()
                .map(|c| AiToolCall {
                    id: c.id.clone(),
                    name: c.name.clone(),
                    arguments: c.arguments.clone(),
                })
                .collect(),
            finish: finish_view(a.finish),
            usage: a.usage.map(|u| AiUsage {
                input_tokens: u.input_tokens,
                output_tokens: u.output_tokens,
                estimated: u.estimated,
            }),
        },
        EntryBody::Tool {
            tool_call_id,
            status,
            content,
            auto,
        } => AiEntryView::Tool {
            entry_id,
            created_at,
            tool_call_id: tool_call_id.clone(),
            status: status_view(*status),
            content: content.clone(),
            auto: auto.as_ref().map(auto_view),
        },
        EntryBody::Summary { text } => AiEntryView::Summary {
            entry_id,
            created_at,
            text: text.clone(),
        },
    }
}

pub(super) fn finish_view(finish: Finish) -> AiFinish {
    match finish {
        Finish::Stop => AiFinish::Stop,
        Finish::ToolCalls => AiFinish::ToolCalls,
        Finish::Length => AiFinish::Length,
        Finish::Refused => AiFinish::Refused,
    }
}

fn status_view(status: ToolStatus) -> AiToolStatus {
    match status {
        ToolStatus::Ok => AiToolStatus::Ok,
        ToolStatus::Error => AiToolStatus::Error,
        ToolStatus::Rejected => AiToolStatus::Rejected,
        ToolStatus::Cancelled => AiToolStatus::Cancelled,
    }
}

pub(super) fn core_status(status: AiToolStatus) -> ToolStatus {
    match status {
        AiToolStatus::Ok => ToolStatus::Ok,
        AiToolStatus::Error => ToolStatus::Error,
        AiToolStatus::Rejected => ToolStatus::Rejected,
        AiToolStatus::Cancelled => ToolStatus::Cancelled,
    }
}

pub(super) fn stream_event(event: StreamEvent) -> AiTurnEvent {
    match event {
        StreamEvent::Text(delta) => AiTurnEvent::Text { delta },
        StreamEvent::Reasoning(delta) => AiTurnEvent::Reasoning { delta },
        StreamEvent::Thinking => AiTurnEvent::Thinking,
        StreamEvent::ToolCall(call) => AiTurnEvent::ToolCall {
            id: call.id,
            name: call.name,
            arguments: call.arguments,
        },
        StreamEvent::Usage(usage) => AiTurnEvent::Usage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            estimated: usage.estimated,
        },
        StreamEvent::EffortIgnored => AiTurnEvent::EffortIgnored,
    }
}

/// The `error` event's message: the provider's own message for an HTTP error (its status goes
/// in `status`), otherwise the failure.
pub(super) fn error_text(e: &AiError) -> String {
    match e {
        AiError::Http { message, .. } => message.clone(),
        other => other.to_string(),
    }
}

/// The kind of a failure, for logs (never its message, which may quote the provider).
pub fn error_kind(e: &AiError) -> &'static str {
    match e {
        AiError::InvalidUrl(_) => "invalid_url",
        AiError::Config(_) => "config",
        AiError::Http { .. } => "http",
        AiError::Network(_) => "network",
        AiError::Protocol(_) => "protocol",
        AiError::Blocked(_) => "blocked",
        AiError::UnsupportedContentType(_) => "content_type",
        AiError::SignIn(_) => "sign_in",
        AiError::Cancelled => "cancelled",
    }
}
