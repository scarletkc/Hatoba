//! Local terminal tabs (spec §7.5): the shells of this computer, each in a PTY, by session id.
//! Their ids are UUIDv7 like the SSH sessions', so one id never names both.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use hatoba_local::{LocalPty, LocalTools, Shell};

use crate::error::{AppError, AppResult};

/// A local terminal tab's shell.
pub struct LocalSession {
    pub pty: Arc<LocalPty>,
    pub shell: Shell,
    /// How the AI assistant's tools run on the tab (AI-12, AI-38…41, AI-48, AI-49).
    pub tools: LocalTools,
    closed: AtomicBool,
}

impl LocalSession {
    pub fn new(pty: Arc<LocalPty>, shell: Shell, tools: LocalTools) -> Self {
        Self {
            pty,
            shell,
            tools,
            closed: AtomicBool::new(false),
        }
    }

    /// Ends the shell and the console programs it started.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.pty.close();
    }

    /// `true` once the shell has ended, so the tab's tools return the disconnected result.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

#[derive(Default)]
pub struct LocalManager {
    sessions: Mutex<HashMap<String, Arc<LocalSession>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl LocalManager {
    pub fn insert(&self, id: String, session: Arc<LocalSession>) {
        lock(&self.sessions).insert(id, session);
    }

    pub fn get(&self, id: &str) -> AppResult<Arc<LocalSession>> {
        lock(&self.sessions)
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::not_found("session"))
    }

    /// The session with `id`, when one is open.
    pub fn find(&self, id: &str) -> Option<Arc<LocalSession>> {
        lock(&self.sessions).get(id).cloned()
    }

    /// Removes the session only if it is still the given instance.
    pub fn remove_if_same(&self, id: &str, live: &Arc<LocalSession>) {
        let mut sessions = lock(&self.sessions);
        if sessions.get(id).is_some_and(|s| Arc::ptr_eq(s, live)) {
            sessions.remove(id);
        }
    }

    pub fn take(&self, id: &str) -> Option<Arc<LocalSession>> {
        lock(&self.sessions).remove(id)
    }

    /// SEC-03 "lock disconnects sessions", and quitting: takes every session out, by id.
    pub fn take_all(&self) -> Vec<(String, Arc<LocalSession>)> {
        lock(&self.sessions).drain().collect()
    }
}

/// The variables the AI's tools on a local tab start their `sh` with (macOS and Linux): on macOS
/// the login shell's `PATH`, which an app started from the Dock lacks, as MCP servers get it.
pub fn tools_env() -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    crate::platform::shell_path::default_path()
        .map(|path| vec![("PATH".into(), path.to_owned())])
        .unwrap_or_default()
}

/// The variables a local shell starts with, on top of Hatoba's own environment: the terminal
/// type xterm.js answers to, and on macOS and Linux a UTF-8 `LANG` when none is set, since an app
/// started from the Dock or a launcher may have none and the shell would then show no Chinese or
/// Japanese text.
pub fn shell_env(version: &str) -> Vec<(String, String)> {
    let mut env = vec![
        ("TERM".to_owned(), "xterm-256color".to_owned()),
        ("COLORTERM".to_owned(), "truecolor".to_owned()),
        ("TERM_PROGRAM".to_owned(), "Hatoba".to_owned()),
        ("TERM_PROGRAM_VERSION".to_owned(), version.to_owned()),
    ];
    if !cfg!(windows)
        && ["LANG", "LC_ALL", "LC_CTYPE"]
            .iter()
            .all(|name| std::env::var_os(name).is_none_or(|v| v.is_empty()))
    {
        env.push((
            "LANG".to_owned(),
            utf8_locale(&crate::platform::system::locale()),
        ));
    }
    env
}

/// A POSIX UTF-8 locale for the user's language as the system names it, such as `zh-hans-cn`
/// (`zh_CN.UTF-8`) or `en_us.utf-8` (`en_US.UTF-8`).
fn utf8_locale(language: &str) -> String {
    let base = language.split(['.', '@']).next().unwrap_or_default();
    let parts: Vec<&str> = base.split(['-', '_']).filter(|p| !p.is_empty()).collect();
    let Some(lang) = parts
        .first()
        .filter(|l| l.len() == 2 && l.chars().all(|c| c.is_ascii_alphabetic()))
    else {
        return "en_US.UTF-8".to_owned();
    };
    let lang = lang.to_ascii_lowercase();
    let region = parts
        .iter()
        .skip(1)
        .rev()
        .find(|p| p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic()))
        .map(|r| r.to_ascii_uppercase())
        .unwrap_or_else(|| match lang.as_str() {
            "en" => "US".to_owned(),
            "ja" => "JP".to_owned(),
            "zh" => "CN".to_owned(),
            "ko" => "KR".to_owned(),
            other => other.to_ascii_uppercase(),
        });
    format!("{lang}_{region}.UTF-8")
}

#[cfg(test)]
mod tests {
    use super::utf8_locale;

    #[test]
    fn the_system_language_becomes_a_utf8_locale() {
        assert_eq!(utf8_locale("zh-hans-cn"), "zh_CN.UTF-8");
        assert_eq!(utf8_locale("en_us.utf-8"), "en_US.UTF-8");
        assert_eq!(utf8_locale("ja"), "ja_JP.UTF-8");
        assert_eq!(utf8_locale("de"), "de_DE.UTF-8");
        assert_eq!(utf8_locale(""), "en_US.UTF-8");
        assert_eq!(utf8_locale("c"), "en_US.UTF-8");
    }
}
