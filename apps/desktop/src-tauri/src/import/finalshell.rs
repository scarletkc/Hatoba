//! FinalShell connections (SSH-15). Its data folder has a `conn` folder with one
//! `<id>_connect_config.json` per connection and a `folder.json` per folder; both name their
//! parent folder by id, and deleted ones keep a `delete_time`. FinalShell keeps private keys in its
//! own config, and the passwords it saves are encoded, so neither is imported.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::text;
use super::{Entry, Source, port_or};
use crate::error::AppResult;

/// `conection_type` (sic) of an SSH connection; others are RDP, VNC and so on.
const SSH: i64 = 100;
/// `authentication_type` of a password; a key is another.
const PASSWORD_AUTH: i64 = 1;

pub(super) fn read(path: Option<&Path>) -> AppResult<Source> {
    let conn = match path {
        // The data folder, or its `conn` folder.
        Some(path) => {
            let conn = path.join("conn");
            if conn.is_dir() {
                conn
            } else {
                path.to_path_buf()
            }
        }
        None => match default_folder() {
            Some(conn) => conn,
            None => return Ok(Source::default()),
        },
    };
    Ok(Source {
        entries: read_conn(&conn)?,
        location: Some(conn.to_string_lossy().into_owned()),
        encrypted: false,
        ..Source::default()
    })
}

/// The `conn` folder in the data folder of FinalShell's installer, on each system.
fn default_folder() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    [
        dirs::data_local_dir().map(|d| d.join("finalshell")),
        dirs::data_dir().map(|d| d.join("finalshell")),
        Some(home.join("Library").join("FinalShell")),
        Some(home.join(".finalshell")),
    ]
    .into_iter()
    .flatten()
    .map(|d| d.join("conn"))
    .find(|d| d.is_dir())
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Folder {
    id: String,
    name: String,
    parent_id: Option<String>,
    delete_time: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Connection {
    id: String,
    name: String,
    host: String,
    /// A number, or in some versions a string.
    port: serde_json::Value,
    user_name: String,
    parent_id: Option<String>,
    conection_type: Option<i64>,
    authentication_type: Option<i64>,
    secret_key_id: String,
    description: String,
    delete_time: i64,
}

fn read_conn(conn: &Path) -> AppResult<Vec<Entry>> {
    let mut folders: HashMap<String, Folder> = HashMap::new();
    let mut connections: Vec<Connection> = Vec::new();
    for (path, _) in text::files_under(conn, text::MAX_DEPTH)? {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        // A file that does not parse is left out rather than failing the others.
        if name == "folder.json" {
            if let Some(folder) = parse::<Folder>(&path)?.filter(|f| f.delete_time == 0) {
                folders.insert(folder.id.clone(), folder);
            }
        } else if name.ends_with("_connect_config.json") {
            connections.extend(parse::<Connection>(&path)?.map(|mut c| {
                // Each connection has its own file, which stands in for a missing id.
                if c.id.is_empty() {
                    c.id = path.to_string_lossy().into_owned();
                }
                c
            }));
        }
    }
    Ok(connections
        .into_iter()
        .filter_map(|c| entry(c, &folders))
        .collect())
}

fn parse<T: for<'de> Deserialize<'de>>(path: &Path) -> AppResult<Option<T>> {
    Ok(serde_json::from_slice(&text::read_file(path)?).ok())
}

fn entry(c: Connection, folders: &HashMap<String, Folder>) -> Option<Entry> {
    if c.delete_time != 0 || c.conection_type.is_some_and(|t| t != SSH) {
        return None;
    }
    let address = c.host.trim().to_owned();
    if address.is_empty() {
        return None;
    }
    let key = !c.secret_key_id.is_empty() && c.authentication_type != Some(PASSWORD_AUTH);
    Some(Entry {
        id: c.id,
        name: match c.name.trim() {
            "" => address.clone(),
            name => name.to_owned(),
        },
        port: match &c.port {
            serde_json::Value::Number(n) => port_or(Some(&n.to_string()), 22),
            serde_json::Value::String(s) => port_or(Some(s), 22),
            _ => 22,
        },
        username: Some(c.user_name.trim().to_owned()).filter(|u| !u.is_empty()),
        folder: folder_path(c.parent_id.as_deref(), folders),
        kept_key: key.then(String::new),
        note: c.description,
        address,
        ..Entry::default()
    })
}

/// The names of the folder `id` and those it is in, outermost first.
fn folder_path<'a>(mut id: Option<&'a str>, folders: &'a HashMap<String, Folder>) -> Vec<String> {
    let mut path = Vec::new();
    // At most once per folder, in case of a loop.
    for _ in 0..=folders.len() {
        let Some(folder) = id.and_then(|id| folders.get(id)) else {
            break;
        };
        if !folder.name.trim().is_empty() {
            path.push(folder.name.trim().to_owned());
        }
        id = folder.parent_id.as_deref();
    }
    path.reverse();
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connections_import_with_their_folders_and_skip_others() {
        let root =
            std::env::temp_dir().join(format!("hatoba-import-finalshell-{}", uuid::Uuid::now_v7()));
        let conn = root.join("conn");
        std::fs::create_dir_all(conn.join("f1").join("f2")).unwrap();
        let write = |path: PathBuf, json: &str| std::fs::write(path, json).unwrap();
        write(
            conn.join("f1").join("folder.json"),
            r#"{"id":"f1","name":"生产","parent_id":"root","delete_time":0}"#,
        );
        write(
            conn.join("f1").join("f2").join("folder.json"),
            r#"{"id":"f2","name":"数据库","parent_id":"f1","delete_time":0}"#,
        );
        write(
            conn.join("f1")
                .join("f2")
                .join("1700000000001_connect_config.json"),
            r#"{"id":"1700000000001","name":"pg-主库","host":" 10.0.0.2 ","port":2222,"user_name":"postgres",
                "parent_id":"f2","conection_type":100,"authentication_type":2,"secret_key_id":"k1",
                "password":"","description":"主库","delete_time":0,"terminal_encoding":"UTF-8"}"#,
        );
        write(
            conn.join("1700000000002_connect_config.json"),
            r#"{"id":"1700000000002","name":"","host":"web.example.org","port":"22","user_name":"root",
                "parent_id":"root","conection_type":100,"authentication_type":1,"secret_key_id":"",
                "password":"AwcLDRETFx1OXQgZ","delete_time":0}"#,
        );
        write(
            conn.join("1700000000003_connect_config.json"),
            r#"{"id":"1700000000003","name":"desktop","host":"10.0.0.3","port":3389,"conection_type":101}"#,
        );
        write(
            conn.join("1700000000004_connect_config.json"),
            r#"{"id":"1700000000004","name":"old","host":"10.0.0.4","conection_type":100,"delete_time":1700000000000}"#,
        );
        write(conn.join("config.json"), r#"{"secret_key_list":[]}"#);
        write(conn.join("1700000000005_connect_config.json"), "{ not json");

        // The data folder and its `conn` folder both work.
        let from_data = read(Some(&root)).unwrap();
        let from_conn = read(Some(&conn)).unwrap();
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(from_data.entries.len(), 2);
        assert_eq!(from_conn.entries.len(), 2);
        assert_eq!(
            from_data.location,
            Some(conn.to_string_lossy().into_owned())
        );

        let web = &from_data.entries[0];
        assert_eq!(
            (
                web.name.as_str(),
                web.address.as_str(),
                web.port,
                web.username.as_deref()
            ),
            ("web.example.org", "web.example.org", 22, Some("root"))
        );
        assert!(web.folder.is_empty());
        assert_eq!(web.kept_key, None, "a password connection");

        let pg = &from_data.entries[1];
        assert_eq!(
            (
                pg.name.as_str(),
                pg.address.as_str(),
                pg.port,
                pg.username.as_deref()
            ),
            ("pg-主库", "10.0.0.2", 2222, Some("postgres"))
        );
        assert_eq!(pg.folder, ["生产", "数据库"]);
        assert_eq!(pg.kept_key.as_deref(), Some(""));
        assert_eq!(pg.note, "主库");
    }

    #[test]
    fn a_folder_loop_ends() {
        let folders: HashMap<String, Folder> = [("a", "b"), ("b", "a")]
            .into_iter()
            .map(|(id, parent)| {
                (
                    id.to_owned(),
                    Folder {
                        id: id.into(),
                        name: id.to_uppercase(),
                        parent_id: Some(parent.into()),
                        delete_time: 0,
                    },
                )
            })
            .collect();
        assert_eq!(folder_path(Some("a"), &folders).len(), 3);
    }
}
