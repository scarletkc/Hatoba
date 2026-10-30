//! What the desktop app's crates share over IPC (spec §10): [`error::AppError`], the error every
//! command returns, and the data transfer objects that more than one of them uses. Independent of
//! Tauri, so `hatoba-assistant` builds and tests without it.

pub mod dto;
pub mod error;
