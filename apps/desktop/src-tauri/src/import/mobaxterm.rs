//! MobaXterm sessions (SSH-15), in `MobaXterm.ini` (in `%APPDATA%\MobaXterm`, or beside the
//! portable edition's program) or in a `.mxtsessions` export, which holds the same sections. Each
//! `[Bookmarks]` or `[Bookmarks_N]` section is a folder, `SubRep` its path, and each other line a
//! session: `name=` then `#`-separated parts, the third of which holds the connection's settings
//! separated by `%`. The file is in the Windows ANSI code page. The passwords MobaXterm saves are
//! encrypted with its master password, so they are not imported.

use std::path::{Path, PathBuf};

use hatoba_core::model::ProxyKind;
use zeroize::Zeroizing;

use super::text;
use super::{Entry, Gateway, Jump, Proxy, Source, non_empty, port_or};
use crate::error::AppResult;

pub(super) fn read(path: Option<&Path>) -> AppResult<Source> {
    let Some(path) = path.map(Path::to_path_buf).or_else(default_file) else {
        return Ok(Source::default());
    };
    let text = text::decode(&text::read_file(&path)?);
    Ok(Source {
        location: Some(path.to_string_lossy().into_owned()),
        encrypted: false,
        entries: parse(&text, &Places::of_this_device()),
        ..Source::default()
    })
}

/// The installed edition's `MobaXterm.ini`.
fn default_file() -> Option<PathBuf> {
    dirs::config_dir()
        .map(|d| d.join("MobaXterm").join("MobaXterm.ini"))
        .filter(|p| p.is_file())
}

/// What MobaXterm's placeholders in paths stand for.
struct Places {
    /// `_CurrentDrive_`: the drive letter MobaXterm runs from.
    drive: String,
    /// `_ProfileDir_`: the user's profile folder.
    profile: Option<String>,
    /// `_MyDocuments_`: the user's documents folder.
    documents: Option<String>,
}

impl Places {
    fn of_this_device() -> Self {
        let drive = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        Self {
            drive: drive.trim_end_matches(':').to_owned(),
            profile: dirs::home_dir().map(|p| p.to_string_lossy().into_owned()),
            documents: dirs::document_dir().map(|p| p.to_string_lossy().into_owned()),
        }
    }

    fn expand(&self, path: &str) -> String {
        let mut path = path.replace("_CurrentDrive_", &self.drive);
        if let Some(profile) = &self.profile {
            path = path.replace("_ProfileDir_", profile);
        }
        if let Some(documents) = &self.documents {
            path = path.replace("_MyDocuments_", documents);
        }
        path
    }
}

/// The settings of an SSH session, by their index among the `%`-separated ones.
mod field {
    pub const TYPE: usize = 0;
    pub const HOST: usize = 1;
    pub const PORT: usize = 2;
    pub const USER: usize = 3;
    pub const GATEWAY_HOSTS: usize = 8;
    pub const GATEWAY_PORTS: usize = 9;
    pub const GATEWAY_USERS: usize = 10;
    pub const KEY: usize = 14;
    pub const PROXY_TYPE: usize = 19;
    pub const PROXY_HOST: usize = 20;
    pub const PROXY_PORT: usize = 21;
    pub const PROXY_USER: usize = 22;
    pub const PROXY_COMMAND: usize = 26;
}

/// The session type of SSH; others are RDP, VNC, Telnet, SFTP and so on.
const SSH: &str = "0";
/// What separates the gateways of a list.
const PIPE: &str = "__PIPE__";

fn parse(text: &str, places: &Places) -> Vec<Entry> {
    let mut entries = Vec::new();
    for section in text::ini(text) {
        let Some(suffix) = section.name.strip_prefix("Bookmarks") else {
            continue;
        };
        if !(suffix.is_empty() || suffix.starts_with('_')) {
            continue;
        }
        let folder: Vec<String> = section
            .get("SubRep")
            .unwrap_or_default()
            .split('\\')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        for (name, value) in &section.entries {
            if name.eq_ignore_ascii_case("SubRep") || name.eq_ignore_ascii_case("ImgNum") {
                continue;
            }
            entries.extend(session(name, value, &folder, places));
        }
    }
    entries
}

fn session(name: &str, value: &str, folder: &[String], places: &Places) -> Option<Entry> {
    let parts: Vec<&str> = value.split('#').collect();
    let settings: Vec<String> = parts.get(2)?.split('%').map(unescape).collect();
    let get = |i: usize| settings.get(i).map(String::as_str).unwrap_or_default();
    if get(field::TYPE) != SSH {
        return None;
    }
    let address = non_empty(Some(get(field::HOST)))?;
    let username = non_empty(Some(get(field::USER))).filter(|u| u != "<default>");
    let key = non_empty(Some(get(field::KEY))).map(|k| places.expand(&k));
    let (proxy, skipped_proxy) = proxy(&get);
    let id = folder
        .iter()
        .map(String::as_str)
        .chain([name])
        .collect::<Vec<_>>()
        .join("\\");
    Some(Entry {
        id,
        name: name.to_owned(),
        address,
        port: port_or(Some(get(field::PORT)), 22),
        username,
        folder: folder.to_vec(),
        identity_files: key.into_iter().collect(),
        jump: gateways(&get).map(Jump::Gateways),
        proxy,
        skipped_proxy,
        note: parts.get(5).map(|c| unescape(c.trim())).unwrap_or_default(),
        ..Entry::default()
    })
}

/// What MobaXterm writes in place of the characters that separate its settings.
fn unescape(value: &str) -> String {
    value.replace("__DIEZE__", "#").replace("__PTVIRG__", ";")
}

/// The SSH gateways, outermost first; their own private keys are not imported.
fn gateways<'a>(get: &impl Fn(usize) -> &'a str) -> Option<Vec<Gateway>> {
    let hosts: Vec<&str> = get(field::GATEWAY_HOSTS).split(PIPE).collect();
    let ports: Vec<&str> = get(field::GATEWAY_PORTS).split(PIPE).collect();
    let users: Vec<&str> = get(field::GATEWAY_USERS).split(PIPE).collect();
    let list: Vec<Gateway> = hosts
        .iter()
        .enumerate()
        .filter_map(|(i, host)| {
            Some(Gateway {
                address: non_empty(Some(host))?,
                port: port_or(ports.get(i).copied(), 22),
                username: non_empty(users.get(i).copied()),
            })
        })
        .collect();
    (!list.is_empty()).then_some(list)
}

/// The proxy to import, or the description of one Hatoba can't use.
fn proxy<'a>(get: &impl Fn(usize) -> &'a str) -> (Option<Proxy>, Option<String>) {
    let host = get(field::PROXY_HOST).trim();
    let port = port_or(Some(get(field::PROXY_PORT)), 1080);
    let kind = match get(field::PROXY_TYPE) {
        "2" => ProxyKind::Socks5,
        "3" => ProxyKind::Http,
        "1" => return (None, Some(format!("SOCKS4 proxy {host}:{port}"))),
        "4" => return (None, Some(format!("Telnet proxy {host}:{port}"))),
        "5" => {
            return (
                None,
                Some(format!("local proxy command {}", get(field::PROXY_COMMAND))),
            );
        }
        _ => return (None, None),
    };
    if host.is_empty() {
        return (None, None);
    }
    let proxy = Proxy {
        kind,
        address: host.to_owned(),
        port,
        username: get(field::PROXY_USER).trim().to_owned(),
        password: Zeroizing::new(String::new()),
    };
    (Some(proxy), None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn places() -> Places {
        Places {
            drive: "D".into(),
            profile: Some("C:\\Users\\ops".into()),
            documents: Some("C:\\Users\\ops\\Documents".into()),
        }
    }

    /// An export as MobaXterm 24 writes it, with a session line per setting it tests.
    const EXPORT: &str = "[Bookmarks]\r\n\
        SubRep=\r\n\
        ImgNum=42\r\n\
        web= #109#0%web.example.org%22%<default>%%-1%-1%%%%%0%-1%0%%%-1%0%0%0%%1080%%0%0%1%%0%%%%0%-1%-1%0#MobaFont%10%0%0%-1%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%0%_Std_Colors_0_%80%24%0%1%-1%<none>%%0%0%-1%-1#0# #-1\r\n\
        desktop= #91#4%10.0.0.9%3389%admin%0%0%0%0%-1%0%0%0%%%%%0%0%%-1%%-1%0%0%0%0%%1080%%0%0%0%0%-1%0%0%-1%0%0%0%0#MobaFont%10%0%0%-1%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%0%_Std_Colors_0_%80%24%0%1%-1%<none>%%0%0%-1%-1#0# #-1\r\n\
        [Bookmarks_1]\r\n\
        SubRep=生产\\数据库\r\n\
        ImgNum=41\r\n\
        pg 主库= #109#0%10.0.0.2%2222%postgres%%-1%-1%%bastion.example.org__PIPE__10.0.0.1%22__PIPE__2200%jump__PIPE__%0%0%0%_ProfileDir_\\.ssh\\id_ed25519%%-1%0%0%2%socks.example.org%1081%proxyuser%0%0%1%%0%%%%0%-1%-1%0#MobaFont%10%0%0%-1%15%236,236,236%30,30,30%180,180,192%0%-1%0%%xterm%-1%0%_Std_Colors_0_%80%24%0%1%-1%<none>%%0%0%-1%-1#0#主库 __DIEZE__1#-1\r\n\
        legacy= #109#0%10.0.0.3%22%root%%-1%-1%%%%%0%0%0%_CurrentDrive_:\\keys\\legacy.ppk%%-1%0%0%5%%1080%%0%0%1%nc __PTVIRG__ proxy 8080#MobaFont\r\n\
        [Bookmarks_2]\r\n\
        SubRep=Lab\r\n\
        ImgNum=41\r\n\
        old=#109#0%10.0.0.4%22%root%%-1%-1%%%%%0%0%0%%%-1%0%0%1%socks4.example.org%1080%%0\r\n\
        [Misc]\r\n\
        other= #109#0%10.0.0.5%22%root\r\n";

    #[test]
    fn ssh_sessions_parse_with_their_folders_keys_gateways_and_proxies() {
        let entries = parse(EXPORT, &places());
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["web", "pg 主库", "legacy", "old"],
            "RDP and other sections are left out"
        );

        let web = &entries[0];
        assert_eq!(web.id, "web");
        assert_eq!((web.address.as_str(), web.port), ("web.example.org", 22));
        assert_eq!(web.username, None, "<default> is MobaXterm's default login");
        assert!(web.folder.is_empty() && web.identity_files.is_empty());
        assert!(web.jump.is_none() && web.proxy.is_none() && web.skipped_proxy.is_none());

        let pg = &entries[1];
        assert_eq!(pg.id, "生产\\数据库\\pg 主库");
        assert_eq!(pg.folder, ["生产", "数据库"]);
        assert_eq!((pg.port, pg.username.as_deref()), (2222, Some("postgres")));
        assert_eq!(pg.identity_files, ["C:\\Users\\ops\\.ssh\\id_ed25519"]);
        assert_eq!(
            pg.jump,
            Some(Jump::Gateways(vec![
                Gateway {
                    address: "bastion.example.org".into(),
                    port: 22,
                    username: Some("jump".into()),
                },
                Gateway {
                    address: "10.0.0.1".into(),
                    port: 2200,
                    username: None,
                },
            ]))
        );
        let proxy = pg.proxy.as_ref().unwrap();
        assert_eq!(proxy.kind, ProxyKind::Socks5);
        assert_eq!(proxy.label(), "SOCKS5 socks.example.org:1081");
        assert_eq!(proxy.username, "proxyuser");
        assert_eq!(pg.note, "主库 #1");

        let legacy = &entries[2];
        assert_eq!(legacy.identity_files, ["D:\\keys\\legacy.ppk"]);
        assert_eq!(
            legacy.skipped_proxy.as_deref(),
            Some("local proxy command nc ; proxy 8080")
        );
        assert_eq!(web.note, "", "an empty comment");

        let old = &entries[3];
        assert_eq!(old.folder, ["Lab"]);
        assert_eq!(
            old.skipped_proxy.as_deref(),
            Some("SOCKS4 proxy socks4.example.org:1080")
        );
    }
}
