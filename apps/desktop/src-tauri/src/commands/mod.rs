//! Tauri commands (spec §10.1). Every command returns `AppResult`, serialized as `AppError`.

pub mod app;
pub mod deploy;
pub mod forwards;
pub mod hosts;
pub mod keys;
pub mod settings;
pub mod sftp;
pub mod ssh;
pub mod sync;
pub mod vault;
