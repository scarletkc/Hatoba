//! The AI assistant's data transfer objects (spec §10, §13): what the `ai_*`, skill and MCP
//! commands exchange with the WebView.
//!
//! The desktop app re-exports them from its own `dto` module, and tauri-specta exports them, with
//! their doc comments, to `apps/desktop/src/ipc/bindings.ts`, the frontend's only definition of
//! them. **No DTO carries a secret**; inputs may carry secrets one way (WebView → Rust), and they
//! are never echoed back.

pub use hatoba_ipc::dto::{LocalShellKind, QuickTarget};

mod mcp;
mod providers;
mod skills;
mod turns;

pub use mcp::*;
pub use providers::*;
pub use skills::*;
pub use turns::*;
