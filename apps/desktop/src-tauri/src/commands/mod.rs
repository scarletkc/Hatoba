//! Tauri commands (spec §10.1). Every command returns `AppResult`, serialized as `AppError`.

pub mod ai;
pub mod allowances;
pub mod app;
pub mod cli;
pub mod dashboard;
pub mod deploy;
pub mod forwards;
pub mod hosts;
pub mod import;
pub mod keys;
pub mod local;
pub mod mcp;
pub mod memory;
pub mod proxies;
pub mod quick;
pub mod settings;
pub mod sftp;
pub mod skills;
pub mod ssh;
pub mod sync;
pub mod vault;
