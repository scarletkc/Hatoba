//! Xshell sessions (SSH-15): one `.xsh` file per session, an INI file in UTF-16, in the sessions
//! folder and its subfolders, which are the session folders. **File → Export** writes them all to
//! an `.xts` file, a zip of the same files. Xshell keeps its private keys and proxies itself, and
//! the passwords it saves are encrypted for the Windows account, so none of them are imported.

use std::io::{Read, Seek};
use std::path::{Path, PathBuf};

use super::text::{self, MAX_FILE_BYTES, MAX_FILES};
use super::{Entry, Source, non_empty, port_or};
use crate::error::{AppError, AppResult};

pub(super) fn read(path: Option<&Path>) -> AppResult<Source> {
    let (location, entries) = match path {
        Some(path) if path.is_dir() => {
            let folder = sessions_folder(path);
            let entries = read_folder(&folder)?;
            (folder, entries)
        }
        Some(path) => (path.to_path_buf(), read_export(path)?),
        None => match default_folder() {
            Some(folder) => {
                let entries = read_folder(&folder)?;
                (folder, entries)
            }
            None => return Ok(Source::default()),
        },
    };
    Ok(Source {
        location: Some(location.to_string_lossy().into_owned()),
        encrypted: false,
        entries,
    })
}

/// The sessions folder for a chosen folder: the folder itself, or `Xshell/Sessions` in the user
/// data folder, or `Sessions` in the `Xshell` folder. A session group may also be called
/// `Sessions`, so a sessions folder is not looked into.
fn sessions_folder(chosen: &Path) -> PathBuf {
    let in_data = chosen.join("Xshell").join("Sessions");
    let in_xshell = chosen.join("Sessions");
    if in_data.is_dir() {
        in_data
    } else if chosen
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("Xshell"))
        && in_xshell.is_dir()
    {
        in_xshell
    } else {
        chosen.to_path_buf()
    }
}

/// The sessions folder of the newest Xshell that has one, in its default place.
fn default_folder() -> Option<PathBuf> {
    let docs = dirs::document_dir()?;
    let netsarang = docs.join("NetSarang Computer");
    ["9", "8", "7", "6"]
        .into_iter()
        .map(|version| netsarang.join(version).join("Xshell").join("Sessions"))
        // Xshell 5.
        .chain([docs.join("NetSarang").join("Xshell").join("Sessions")])
        .find(|p| p.is_dir())
}

fn read_folder(root: &Path) -> AppResult<Vec<Entry>> {
    let mut entries = Vec::new();
    for (path, folder) in text::files_under(root, text::MAX_DEPTH)? {
        if !is_xsh(&path.to_string_lossy()) {
            continue;
        }
        let Some(name) = path.file_stem().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        let id = folder
            .iter()
            .map(String::as_str)
            .chain([name.as_str()])
            .collect::<Vec<_>>()
            .join("/");
        let bytes = text::read_file(&path)?;
        entries.extend(parse_xsh(&text::decode(&bytes), id, name, folder));
    }
    Ok(entries)
}

fn read_export(path: &Path) -> AppResult<Vec<Entry>> {
    let file = std::fs::File::open(path).map_err(AppError::io)?;
    read_zip(file)
}

/// The sessions in an `.xts` export. Its paths start with `Xshell/Sessions/`; on Chinese and
/// Japanese Windows the names are in the ANSI code page rather than UTF-8.
fn read_zip(file: impl Read + Seek) -> AppResult<Vec<Entry>> {
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| AppError::invalid("path", format!("not an Xshell export: {e}")))?;
    if archive.len() > MAX_FILES {
        return Err(AppError::io("the export holds too many files"));
    }
    let mut entries = Vec::new();
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| AppError::invalid("path", e.to_string()))?;
        if !file.is_file() {
            continue;
        }
        let path = zip_name(file.name_raw());
        if !is_xsh(&path) {
            continue;
        }
        let mut parts: Vec<String> = path
            .split(['/', '\\'])
            .filter(|p| !p.is_empty())
            .map(str::to_owned)
            .collect();
        if let Some(at) = parts
            .iter()
            .position(|p| p.eq_ignore_ascii_case("Sessions"))
        {
            parts.drain(..=at);
        } else if parts
            .first()
            .is_some_and(|p| p.eq_ignore_ascii_case("Xshell"))
        {
            parts.remove(0);
        }
        let Some(file_name) = parts.pop() else {
            continue;
        };
        let name = file_name[..file_name.len() - ".xsh".len()].to_owned();
        let mut bytes = Vec::new();
        (&mut file)
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(AppError::io)?;
        total += bytes.len() as u64;
        if bytes.len() as u64 > MAX_FILE_BYTES || total > 4 * MAX_FILE_BYTES {
            return Err(AppError::io("the export is too large"));
        }
        let id = parts
            .iter()
            .map(String::as_str)
            .chain([name.as_str()])
            .collect::<Vec<_>>()
            .join("/");
        entries.extend(parse_xsh(&text::decode(&bytes), id, name, parts));
    }
    Ok(entries)
}

/// A name in a zip file: UTF-8, or else the ANSI code page of the Windows that wrote it. On
/// Windows that is this device's own; elsewhere GBK is tried first, as most Xshell users are
/// Chinese.
fn zip_name(raw: &[u8]) -> String {
    if let Ok(name) = std::str::from_utf8(raw) {
        return name.to_owned();
    }
    gbk(raw).unwrap_or_else(|| text::decode(raw))
}

#[cfg(not(windows))]
fn gbk(raw: &[u8]) -> Option<String> {
    encoding_rs::GBK
        .decode_without_bom_handling_and_without_replacement(raw)
        .map(std::borrow::Cow::into_owned)
}

/// Windows decodes with its own ANSI code page instead.
#[cfg(windows)]
fn gbk(_: &[u8]) -> Option<String> {
    None
}

fn is_xsh(path: &str) -> bool {
    path.len() > ".xsh".len()
        && path
            .get(path.len() - ".xsh".len()..)
            .is_some_and(|ext| ext.eq_ignore_ascii_case(".xsh"))
}

/// One `.xsh` file; `None` for a session that is not SSH, such as Telnet, serial or SFTP.
fn parse_xsh(text: &str, id: String, name: String, folder: Vec<String>) -> Option<Entry> {
    let sections = text::ini(text);
    let connection = text::section(&sections, "CONNECTION")?;
    if !connection
        .get("Protocol")
        .is_none_or(|p| p.eq_ignore_ascii_case("SSH"))
    {
        return None;
    }
    let address = non_empty(connection.get("Host"))?;
    let auth = text::section(&sections, "CONNECTION:AUTHENTICATION");
    let proxy =
        text::section(&sections, "CONNECTION:PROXY").and_then(|s| non_empty(s.get("Proxy")));
    Some(Entry {
        id,
        name,
        address,
        port: port_or(connection.get("Port"), 22),
        username: non_empty(auth.and_then(|a| a.get("UserName"))),
        folder,
        kept_key: non_empty(auth.and_then(|a| a.get("UserKey"))),
        skipped_proxy: proxy.map(|p| format!("Xshell proxy {p}")),
        note: connection.get("Description").unwrap_or_default().to_owned(),
        ..Entry::default()
    })
}

#[cfg(test)]
mod tests {
    use std::io::{Cursor, Write};

    use super::*;

    /// A session file as Xshell 7 writes it: UTF-16LE with a byte order mark, CRLF lines.
    fn xsh(host: &str, extra: &str) -> Vec<u8> {
        let text = format!(
            "[SessionInfo]\r\nVersion=7.0\r\nDescription=Xshell session file\r\n\
             [CONNECTION]\r\nHost={host}\r\nPort=2222\r\nProtocol=SSH\r\nDescription=主数据库\r\n\
             [CONNECTION:AUTHENTICATION]\r\nUserName=deploy\r\nMethod=0\r\nPassword=c2VjcmV0\r\n{extra}"
        );
        [0xFF, 0xFE]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
            .collect()
    }

    #[test]
    fn sessions_parse_with_their_folders_and_skip_other_protocols() {
        let entry = parse_xsh(
            &text::decode(&xsh(
                "db.example.org",
                "UserKey=id_rsa_2048\r\n[CONNECTION:PROXY]\r\nProxy=office\r\n",
            )),
            "生产/db".into(),
            "db".into(),
            vec!["生产".into()],
        )
        .unwrap();
        assert_eq!(entry.name, "db");
        assert_eq!(entry.address, "db.example.org");
        assert_eq!(entry.port, 2222);
        assert_eq!(entry.username.as_deref(), Some("deploy"));
        assert_eq!(entry.folder, ["生产"]);
        assert_eq!(entry.kept_key.as_deref(), Some("id_rsa_2048"));
        assert_eq!(entry.skipped_proxy.as_deref(), Some("Xshell proxy office"));
        assert_eq!(entry.note, "主数据库");
        assert!(entry.identity_files.is_empty());

        let telnet = "[CONNECTION]\r\nHost=switch\r\nProtocol=TELNET\r\n";
        assert!(parse_xsh(telnet, "s".into(), "s".into(), vec![]).is_none());
        let no_host = "[CONNECTION]\r\nHost=\r\nProtocol=SSH\r\n";
        assert!(parse_xsh(no_host, "s".into(), "s".into(), vec![]).is_none());
        let defaults = "[CONNECTION]\r\nHost=10.0.0.5\r\n";
        let entry = parse_xsh(defaults, "s".into(), "s".into(), vec![]).unwrap();
        assert_eq!(
            (entry.port, entry.username, entry.kept_key),
            (22, None, None)
        );
    }

    #[test]
    fn the_sessions_folder_is_read_with_its_subfolders() {
        let root =
            std::env::temp_dir().join(format!("hatoba-import-xshell-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(root.join("Prod").join("DB")).unwrap();
        std::fs::write(root.join("web.xsh"), xsh("web.example.org", "")).unwrap();
        std::fs::write(
            root.join("Prod").join("DB").join("pg.XSH"),
            xsh("pg.example.org", ""),
        )
        .unwrap();
        std::fs::write(root.join("Prod").join("notes.txt"), "not a session").unwrap();

        // A session group called Sessions is a group, not the sessions folder.
        std::fs::create_dir_all(root.join("Sessions")).unwrap();
        std::fs::write(
            root.join("Sessions").join("cache.xsh"),
            xsh("cache.example.org", ""),
        )
        .unwrap();
        let data = root.with_extension("data");
        std::fs::create_dir_all(data.join("Xshell")).unwrap();

        let source = read(Some(&root)).unwrap();
        std::fs::rename(&root, data.join("Xshell").join("Sessions")).unwrap();
        let from_data = read(Some(&data)).unwrap();
        let from_xshell = read(Some(&data.join("Xshell"))).unwrap();
        let _ = std::fs::remove_dir_all(&data);
        assert_eq!(from_data.entries.len(), 3);
        assert_eq!(from_xshell.entries.len(), 3);
        let found: Vec<(&str, &str, Vec<String>)> = source
            .entries
            .iter()
            .map(|e| (e.id.as_str(), e.address.as_str(), e.folder.clone()))
            .collect();
        assert_eq!(
            found,
            [
                (
                    "Prod/DB/pg",
                    "pg.example.org",
                    vec!["Prod".to_owned(), "DB".to_owned()]
                ),
                (
                    "Sessions/cache",
                    "cache.example.org",
                    vec!["Sessions".to_owned()]
                ),
                ("web", "web.example.org", vec![]),
            ]
        );
    }

    #[test]
    fn an_export_is_read_like_the_folder() {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.add_directory("Xshell/Sessions/", options).unwrap();
        zip.start_file("Xshell/Sessions/web.xsh", options).unwrap();
        zip.write_all(&xsh("web.example.org", "")).unwrap();
        zip.start_file("Xshell/Sessions/Prod/db.xsh", options)
            .unwrap();
        zip.write_all(&xsh("db.example.org", "")).unwrap();
        zip.start_file("Xshell/Sessions/Prod/readme.txt", options)
            .unwrap();
        zip.write_all(b"text").unwrap();
        let bytes = zip.finish().unwrap().into_inner();

        let entries = read_zip(Cursor::new(bytes)).unwrap();
        let found: Vec<(&str, &str, Vec<String>)> = entries
            .iter()
            .map(|e| (e.id.as_str(), e.name.as_str(), e.folder.clone()))
            .collect();
        assert_eq!(
            found,
            [
                ("web", "web", vec![]),
                ("Prod/db", "db", vec!["Prod".to_owned()])
            ]
        );
        assert!(read_zip(Cursor::new(b"not a zip".to_vec())).is_err());
    }

    #[test]
    fn zip_names_fall_back_to_an_ansi_code_page() {
        assert_eq!(
            zip_name("Xshell/Sessions/生产.xsh".as_bytes()),
            "Xshell/Sessions/生产.xsh"
        );
        #[cfg(not(windows))]
        {
            let (gbk, _, _) = encoding_rs::GBK.encode("Xshell/Sessions/生产.xsh");
            assert_eq!(zip_name(&gbk), "Xshell/Sessions/生产.xsh");
        }
    }
}
