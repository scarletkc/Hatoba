//! Tauri commands (spec §10.1). Every command returns `AppResult`, serialized as `AppError`.

pub mod ai;
pub mod app;
pub mod deploy;
pub mod forwards;
pub mod hosts;
pub mod keys;
pub mod mcp;
pub mod settings;
pub mod sftp;
pub mod skills;
pub mod ssh;
pub mod sync;
pub mod vault;
