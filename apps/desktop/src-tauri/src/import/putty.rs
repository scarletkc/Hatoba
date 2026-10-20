//! PuTTY's saved sessions (SSH-12): on Windows the registry key
//! `HKCU\Software\SimonTatham\PuTTY\Sessions`, with a subkey per session; on macOS and Linux a
//! file per session in `~/.putty/sessions` (or `~/.config/putty/sessions`). A `.reg` file exported
//! from the registry key brings Windows sessions to another device. Session names are escaped with
//! `%XX`, and PuTTY does not save passwords.

use std::collections::HashMap;
use std::path::Path;

use hatoba_core::model::ProxyKind;
use zeroize::{Zeroize, Zeroizing};

use super::text;
use super::{Entry, Gateway, Jump, Proxy, Source, non_empty, port_or, split_user};
use crate::error::AppResult;

/// Where PuTTY on Windows keeps its sessions, under `HKEY_CURRENT_USER`.
#[cfg(windows)]
const SESSIONS_KEY: &str = r"Software\SimonTatham\PuTTY\Sessions";
/// The session that holds PuTTY's defaults rather than a server.
const DEFAULTS: &str = "Default Settings";

/// One setting of a session: a string, or a number (a `REG_DWORD` in the registry).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Setting {
    Text(String),
    Number(u32),
}

/// A session's settings by name.
#[derive(Debug, Default)]
struct Settings(HashMap<String, Setting>);

/// The settings hold the proxy's password, so they are wiped when dropped.
impl Drop for Settings {
    fn drop(&mut self) {
        for setting in self.0.values_mut() {
            if let Setting::Text(text) = setting {
                text.zeroize();
            }
        }
    }
}

/// Saved sessions by name, and where they were read from.
type Found = Option<(String, Vec<(String, Settings)>)>;

impl Settings {
    fn text(&self, name: &str) -> Option<&str> {
        match self.0.get(name)? {
            Setting::Text(text) => Some(text),
            Setting::Number(_) => None,
        }
    }

    /// A number; the session files of macOS and Linux write numbers as text.
    fn number(&self, name: &str) -> Option<u32> {
        match self.0.get(name)? {
            Setting::Number(n) => Some(*n),
            Setting::Text(text) => text.trim().parse().ok(),
        }
    }
}

pub(super) fn read(path: Option<&Path>) -> AppResult<Source> {
    let (location, sessions) = match path {
        Some(path) if path.is_dir() => {
            let sessions = path.join("sessions");
            let dir = if sessions.is_dir() {
                sessions
            } else {
                path.to_path_buf()
            };
            (dir.to_string_lossy().into_owned(), read_folder(&dir)?)
        }
        Some(path) => (
            path.to_string_lossy().into_owned(),
            parse_reg(&read_text(path)?),
        ),
        None => match read_default()? {
            Some(found) => found,
            None => return Ok(Source::default()),
        },
    };
    Ok(Source {
        location: Some(location),
        encrypted: false,
        entries: entries(sessions),
    })
}

#[cfg(windows)]
fn read_default() -> AppResult<Found> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, REG_DWORD, REG_EXPAND_SZ, REG_SZ};
    use winreg::types::FromRegValue;

    let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(SESSIONS_KEY) else {
        return Ok(None);
    };
    let mut sessions = Vec::new();
    for escaped in key.enum_keys().filter_map(Result::ok) {
        let Ok(session) = key.open_subkey(&escaped) else {
            continue;
        };
        let mut settings = Settings::default();
        for (name, value) in session.enum_values().filter_map(Result::ok) {
            let setting = match value.vtype {
                REG_SZ | REG_EXPAND_SZ => String::from_reg_value(&value).ok().map(Setting::Text),
                REG_DWORD => u32::from_reg_value(&value).ok().map(Setting::Number),
                _ => None,
            };
            if let std::borrow::Cow::Owned(mut bytes) = value.bytes {
                bytes.zeroize();
            }
            if let Some(setting) = setting {
                settings.0.insert(name, setting);
            }
        }
        sessions.push((unescape(&escaped), settings));
    }
    Ok(Some((
        format!(r"HKEY_CURRENT_USER\{SESSIONS_KEY}"),
        sessions,
    )))
}

/// The sessions folder PuTTY uses on macOS and Linux, if it has one.
#[cfg(not(windows))]
fn read_default() -> AppResult<Found> {
    let dir = match std::env::var_os("PUTTYDIR") {
        Some(dir) => std::path::PathBuf::from(dir).join("sessions"),
        None => {
            let Some(home) = dirs::home_dir() else {
                return Ok(None);
            };
            let config = home.join(".config").join("putty");
            let base = if config.is_dir() {
                config
            } else {
                home.join(".putty")
            };
            base.join("sessions")
        }
    };
    if !dir.is_dir() {
        return Ok(None);
    }
    Ok(Some((
        dir.to_string_lossy().into_owned(),
        read_folder(&dir)?,
    )))
}

/// A file's text, which holds a proxy password, wiped when dropped along with its bytes.
fn read_text(path: &Path) -> AppResult<Zeroizing<String>> {
    let bytes = Zeroizing::new(text::read_file(path)?);
    Ok(Zeroizing::new(text::decode(&bytes)))
}

/// A folder of session files, one `Key=value` line per setting.
fn read_folder(dir: &Path) -> AppResult<Vec<(String, Settings)>> {
    let mut sessions = Vec::new();
    for (path, _) in text::files_under(dir, 0)? {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let mut settings = Settings::default();
        for line in read_text(&path)?.lines() {
            if let Some((key, value)) = line.split_once('=') {
                settings
                    .0
                    .insert(key.to_owned(), Setting::Text(value.to_owned()));
            }
        }
        sessions.push((unescape(&name), settings));
    }
    Ok(sessions)
}

/// The sessions in a `.reg` file of the registry key, as Registry Editor exports it.
fn parse_reg(text: &str) -> Vec<(String, Settings)> {
    let marker = r"\SimonTatham\PuTTY\Sessions\";
    let mut sessions: Vec<(String, Settings)> = Vec::new();
    let mut in_session = false;
    for line in text.lines() {
        let line = line.trim();
        if let Some(key) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let escaped = key
                .find(marker)
                .map(|at| &key[at + marker.len()..])
                .filter(|name| !name.is_empty() && !name.contains('\\'));
            in_session = escaped.is_some();
            if let Some(escaped) = escaped {
                sessions.push((unescape(escaped), Settings::default()));
            }
            continue;
        }
        let Some((_, settings)) = sessions.last_mut().filter(|_| in_session) else {
            continue;
        };
        if let Some((name, setting)) = reg_value(line) {
            settings.0.insert(name, setting);
        }
    }
    sessions
}

/// `"Name"="text"` or `"Name"=dword:0000abcd`; other types are left out.
fn reg_value(line: &str) -> Option<(String, Setting)> {
    let (name, rest) = reg_string(line)?;
    let rest = rest.strip_prefix('=')?;
    let setting = if let Some(hex) = rest.strip_prefix("dword:") {
        Setting::Number(u32::from_str_radix(hex.trim(), 16).ok()?)
    } else {
        let (value, tail) = reg_string(rest)?;
        if !tail.trim().is_empty() {
            return None;
        }
        Setting::Text(value)
    };
    Some((name, setting))
}

/// A quoted string with `\\` and `\"` escapes at the start of `text`, and what follows it.
fn reg_string(text: &str) -> Option<(String, &str)> {
    let mut chars = text.strip_prefix('"')?.char_indices();
    let mut value = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '"' => return Some((value, &text[i + 2..])),
            '\\' => value.push(chars.next()?.1),
            c => value.push(c),
        }
    }
    None
}

/// A session name from its registry key or file name, where PuTTY writes `%XX` for a byte that
/// can't appear there. The bytes are in the Windows ANSI code page.
fn unescape(escaped: &str) -> String {
    let bytes = escaped.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%')
            .then(|| escaped.get(i + 1..i + 3))
            .flatten()
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    text::decode(&out)
}

fn entries(sessions: Vec<(String, Settings)>) -> Vec<Entry> {
    let names: Vec<String> = sessions.iter().map(|(name, _)| name.clone()).collect();
    sessions
        .into_iter()
        .filter(|(name, _)| name != DEFAULTS)
        .filter_map(|(name, settings)| entry(name, &settings, &names))
        .collect()
}

/// `ProxyMethod` (PuTTY 0.68 and later), or else the older `ProxyType`.
#[derive(Debug, PartialEq, Eq)]
enum ProxyMethod {
    None,
    Socks4,
    Socks5,
    Http,
    Telnet,
    Command,
    /// SSH to the proxy and forward a port: a jump host (PuTTY 0.77 and later).
    SshForward,
    /// SSH to the proxy and run a command or a subsystem there.
    SshOther,
}

fn proxy_method(s: &Settings) -> ProxyMethod {
    match s.number("ProxyMethod") {
        Some(1) => ProxyMethod::Socks4,
        Some(2) => ProxyMethod::Socks5,
        Some(3) => ProxyMethod::Http,
        Some(4) => ProxyMethod::Telnet,
        Some(5) => ProxyMethod::Command,
        Some(6) => ProxyMethod::SshForward,
        Some(7 | 8) => ProxyMethod::SshOther,
        Some(_) => ProxyMethod::None,
        None => match s.number("ProxyType") {
            Some(1) => ProxyMethod::Http,
            Some(2) if s.number("ProxySOCKSVersion") == Some(4) => ProxyMethod::Socks4,
            Some(2) => ProxyMethod::Socks5,
            Some(3) => ProxyMethod::Telnet,
            Some(4) => ProxyMethod::Command,
            _ => ProxyMethod::None,
        },
    }
}

/// A session, or `None` for one that is not SSH or has no host name. `names` are those of all
/// the sessions, for an SSH proxy that names one.
fn entry(name: String, s: &Settings, names: &[String]) -> Option<Entry> {
    if !s
        .text("Protocol")
        .is_none_or(|p| p.eq_ignore_ascii_case("ssh"))
    {
        return None;
    }
    let (user, host) = split_user(s.text("HostName")?.trim());
    // PuTTY drops a `:port` after a host name, but not the colons of an IPv6 address.
    let host = match host.split_once(':') {
        Some((before, after)) if !after.contains(':') => before,
        _ => host,
    };
    let address = non_empty(Some(host))?;
    let proxy_host = s.text("ProxyHost").unwrap_or("proxy").trim();
    let proxy_port = s.number("ProxyPort").and_then(|p| u16::try_from(p).ok());
    let proxy_label = |kind: &str| format!("{kind} {proxy_host}:{}", proxy_port.unwrap_or(80));
    let mut entry = Entry {
        id: name.clone(),
        name,
        address,
        port: port_or(s.number("PortNumber").map(|p| p.to_string()).as_deref(), 22),
        username: non_empty(s.text("UserName")).or_else(|| user.map(str::to_owned)),
        identity_files: non_empty(s.text("PublicKeyFile")).into_iter().collect(),
        ..Entry::default()
    };
    match proxy_method(s) {
        ProxyMethod::None => {}
        method @ (ProxyMethod::Socks5 | ProxyMethod::Http) if !proxy_host.is_empty() => {
            entry.proxy = Some(Proxy {
                kind: if method == ProxyMethod::Http {
                    ProxyKind::Http
                } else {
                    ProxyKind::Socks5
                },
                address: proxy_host.to_owned(),
                port: proxy_port.filter(|p| *p != 0).unwrap_or(80),
                username: s.text("ProxyUsername").unwrap_or_default().to_owned(),
                password: Zeroizing::new(s.text("ProxyPassword").unwrap_or_default().to_owned()),
            });
        }
        ProxyMethod::Socks5 | ProxyMethod::Http => {}
        ProxyMethod::Socks4 => entry.skipped_proxy = Some(proxy_label("SOCKS4 proxy")),
        ProxyMethod::Telnet => entry.skipped_proxy = Some(proxy_label("Telnet proxy")),
        ProxyMethod::Command => {
            let command = s.text("ProxyTelnetCommand").unwrap_or_default();
            entry.skipped_proxy = Some(format!("local proxy command {command}"));
        }
        ProxyMethod::SshOther => entry.skipped_proxy = Some(proxy_label("SSH proxy")),
        // The proxy host can name a saved session.
        ProxyMethod::SshForward if names.iter().any(|n| n == proxy_host) => {
            entry.jump = Some(Jump::Entry(proxy_host.to_owned()));
        }
        ProxyMethod::SshForward => {
            let (user, host) = split_user(proxy_host);
            if let Some(address) = non_empty(Some(host)) {
                entry.jump = Some(Jump::Gateways(vec![Gateway {
                    address,
                    port: proxy_port.filter(|p| *p != 0).unwrap_or(22),
                    username: non_empty(s.text("ProxyUsername"))
                        .or_else(|| user.map(str::to_owned)),
                }]));
            }
        }
    }
    Some(entry)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `.reg` export of PuTTY 0.83's sessions, as Registry Editor writes it (UTF-16 there).
    const EXPORT: &str = r#"Windows Registry Editor Version 5.00

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions]

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Default%20Settings]
"HostName"=""
"Protocol"="ssh"

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\bastion]
"Present"=dword:00000001
"HostName"="jump@bastion.example.org"
"Protocol"="ssh"
"PortNumber"=dword:00000016

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\web%20%E7%94%9F%E4%BA%A7]
"HostName"="web.example.org:8022"
"UserName"="deploy"
"Protocol"="ssh"
"PortNumber"=dword:00000822
"PublicKeyFile"="C:\\Users\\ops\\keys\\web \"prod\".ppk"
"ProxyMethod"=dword:00000002
"ProxyHost"="socks.example.org"
"ProxyPort"=dword:00000438
"ProxyUsername"="proxyuser"
"ProxyPassword"="proxy secret"
"TerminalModes"=hex(7):43,00,00,00

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\db]
"HostName"="10.0.0.2"
"Protocol"="ssh"
"PortNumber"=dword:00000016
"ProxyMethod"=dword:00000006
"ProxyHost"="bastion"
"ProxyPort"=dword:00000016

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\cache]
"HostName"="10.0.0.3"
"Protocol"="ssh"
"ProxyMethod"=dword:00000006
"ProxyHost"="ops@10.0.0.1"
"ProxyPort"=dword:00000050

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\old]
"HostName"="[::1]"
"ProxyType"=dword:00000002
"ProxySOCKSVersion"=dword:00000004
"ProxyHost"="socks4.example.org"
"ProxyPort"=dword:00000438

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\router]
"HostName"="192.168.1.1"
"Protocol"="telnet"
"PortNumber"=dword:00000017

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\SshHostKeys]
"ssh-ed25519@22:bastion.example.org"="0x0"
"#;

    #[test]
    fn a_registry_export_parses_ssh_sessions() {
        let entries = entries(parse_reg(EXPORT));
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["bastion", "web 生产", "db", "cache", "old"]);

        let bastion = &entries[0];
        assert_eq!(
            (
                bastion.address.as_str(),
                bastion.port,
                bastion.username.as_deref()
            ),
            ("bastion.example.org", 22, Some("jump"))
        );

        let web = &entries[1];
        assert_eq!(
            (web.address.as_str(), web.port, web.username.as_deref()),
            ("web.example.org", 2082, Some("deploy"))
        );
        assert_eq!(web.identity_files, [r#"C:\Users\ops\keys\web "prod".ppk"#]);
        let proxy = web.proxy.as_ref().unwrap();
        assert_eq!(proxy.label(), "SOCKS5 socks.example.org:1080");
        assert_eq!(proxy.username, "proxyuser");
        assert_eq!(proxy.password.as_str(), "proxy secret");

        // An SSH proxy that names a saved session, and one that is an address.
        assert_eq!(entries[2].jump, Some(Jump::Entry("bastion".into())));
        assert_eq!(
            entries[3].jump,
            Some(Jump::Gateways(vec![Gateway {
                address: "10.0.0.1".into(),
                port: 80,
                username: Some("ops".into()),
            }]))
        );

        let old = &entries[4];
        assert_eq!(old.address, "[::1]", "no port is cut from an IPv6 address");
        assert_eq!(
            old.skipped_proxy.as_deref(),
            Some("SOCKS4 proxy socks4.example.org:1080")
        );
        assert!(old.proxy.is_none());
    }

    #[test]
    fn session_files_of_macos_and_linux_parse_too() {
        let dir =
            std::env::temp_dir().join(format!("hatoba-import-putty-{}", uuid::Uuid::now_v7()));
        let sessions = dir.join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        std::fs::write(
            sessions.join("my%20server"),
            "HostName=10.0.0.9\nPortNumber=2200\nProtocol=ssh\nUserName=root\nPublicKeyFile=/home/ops/.ssh/id.ppk\nProxyMethod=3\nProxyHost=proxy.example.org\nProxyPort=3128\n",
        )
        .unwrap();
        std::fs::write(sessions.join("Default%20Settings"), "HostName=\n").unwrap();

        let source = read(Some(&dir)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(
            source.location,
            Some(sessions.to_string_lossy().into_owned())
        );
        assert_eq!(source.entries.len(), 1);
        let entry = &source.entries[0];
        assert_eq!(
            (entry.name.as_str(), entry.address.as_str(), entry.port),
            ("my server", "10.0.0.9", 2200)
        );
        assert_eq!(entry.identity_files, ["/home/ops/.ssh/id.ppk"]);
        assert_eq!(
            entry.proxy.as_ref().unwrap().label(),
            "HTTP proxy.example.org:3128"
        );
    }

    #[test]
    fn escaped_names_decode() {
        assert_eq!(unescape("a%20b%25c%2E"), "a b%c.");
        assert_eq!(unescape("%E7%94%9F%E4%BA%A7"), "生产");
        assert_eq!(unescape("bad%zz%"), "bad%zz%");
    }
}
