//! `~/.ssh/config` (`%USERPROFILE%\.ssh\config` on Windows), SSH-11.

use hatoba_ssh::SshConfigHost;

use super::{Entry, Jump, Source};
use crate::error::{AppError, AppResult};

pub(super) fn read() -> AppResult<Source> {
    let home = dirs::home_dir().ok_or_else(|| AppError::not_found("home directory"))?;
    let path = home.join(".ssh").join("config");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(AppError::io(e)),
    };
    let home = home.to_string_lossy();
    Ok(Source {
        location: Some(path.to_string_lossy().into_owned()),
        encrypted: false,
        entries: hatoba_ssh::config::parse_ssh_config_with_home(&text, Some(&home))
            .into_iter()
            .map(entry)
            .collect(),
        ..Source::default()
    })
}

fn entry(host: SshConfigHost) -> Entry {
    Entry {
        id: host.alias.clone(),
        address: host.hostname.unwrap_or_else(|| host.alias.clone()),
        name: host.alias,
        port: host.port.unwrap_or(22),
        username: host.user,
        identity_files: host.identity_files,
        jump: host.proxy_jump.map(Jump::Named),
        skipped_proxy: host.proxy_command.map(|c| format!("ProxyCommand {c}")),
        set_env: host.set_env,
        ..Entry::default()
    }
}
