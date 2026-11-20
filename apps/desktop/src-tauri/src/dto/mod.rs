//! Data transfer objects exchanged with the WebView (spec §10).
//!
//! tauri-specta exports them, with their doc comments, to `apps/desktop/src/ipc/bindings.ts`, the
//! frontend's only definition of them.
//! **No DTO carries a secret**: hosts expose `has_password`, keys expose only public material.
//! Inputs may carry secrets one way (WebView → Rust), e.g. a new host password; they are never
//! echoed back.

pub use hatoba_assistant::dto::*;

mod app;
mod convert;
mod hosts;
mod sessions;
mod settings;
mod sync;

#[cfg(test)]
mod tests;

pub use app::*;
pub use hosts::*;
pub use sessions::*;
pub use settings::*;
pub use sync::*;
