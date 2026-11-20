//! `From` conversions between `hatoba-assistant`'s DTO enums and their `hatoba-core` / `hatoba-ai`
//! counterparts.

use hatoba_ipc::mirror_enum;

use super::{
    AiAuthHeader, AiChatgptStatus, AiEffort, AiFileChangeState, AiFinish, AiJudgeDecision,
    AiPermissionMode, AiProtocol, AiTestFailure, AiToolStatus, SearchKind,
};

mirror_enum!(
    AiProtocol <=> hatoba_core::model::AiProtocol:
    ChatCompletions,
    Anthropic,
    Responses,
    SystemOne,
);
mirror_enum!(
    AiProtocol <=> hatoba_ai::provider::Protocol:
    ChatCompletions,
    Anthropic,
    Responses,
    SystemOne,
);
mirror_enum!(
    AiAuthHeader <=> hatoba_core::model::AiAuthHeader:
    XApiKey,
    Authorization,
);
mirror_enum!(
    AiAuthHeader => hatoba_ai::provider::AuthHeader:
    XApiKey,
    Authorization,
);
mirror_enum!(
    AiEffort <=> hatoba_core::model::AiEffort:
    Low,
    Medium,
    High,
    Xhigh,
    Max,
);
mirror_enum!(
    AiEffort <=> hatoba_ai::provider::Effort:
    Low,
    Medium,
    High,
    Xhigh,
    Max,
);
mirror_enum!(
    SearchKind <=> hatoba_core::model::SearchKind:
    Brave,
    Tavily,
    Searxng,
);
mirror_enum!(
    SearchKind => hatoba_ai::web::SearchKind:
    Brave,
    Tavily,
    Searxng,
);
mirror_enum!(
    AiPermissionMode => hatoba_ai::tools::PermissionMode:
    Manual,
    Auto,
    Bypass,
);
mirror_enum!(
    hatoba_ai::entry::AutoDecision => AiJudgeDecision:
    Run,
    Ask,
);
mirror_enum!(
    hatoba_ai::entry::Finish => AiFinish:
    Stop,
    ToolCalls,
    Length,
    Refused,
);
mirror_enum!(
    AiToolStatus <=> hatoba_ai::entry::ToolStatus:
    Ok,
    Error,
    Rejected,
    Cancelled,
);
mirror_enum!(
    hatoba_core::SnapshotState => AiFileChangeState:
    Unfinished,
    Written,
    Reverted,
    Dropped,
);
mirror_enum!(
    hatoba_ai::chatgpt::SessionStatus => AiChatgptStatus:
    SignedIn,
    PlanDisabled,
    Ended,
    SignedOut,
);
mirror_enum!(
    hatoba_ai::models::TestFailure => AiTestFailure:
    Auth,
    Network,
    UnknownModel,
    InvalidUrl,
    Other,
);
