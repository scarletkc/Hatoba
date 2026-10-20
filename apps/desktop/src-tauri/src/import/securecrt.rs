//! SecureCRT sessions (SSH-15): one `.ini` file per session in the `Sessions` folder of SecureCRT's
//! configuration folder, whose subfolders are the session folders. Each line holds one setting,
//! `S:"Name"=text`, `D:"Name"=` a number in hex, or `Z:"Name"=` the number of the lines that follow
//! with a list's items. **Tools → Export Settings** writes the same settings to an XML file, in a
//! `<key>` element for each folder and session, along with the files they name, key files among
//! them. A session can sign in with the key file set for all sessions, kept in `SSH2.ini`, and
//! reach its server through another session, which SecureCRT calls a firewall. The passwords
//! SecureCRT saves are encrypted, with the configuration passphrase when one is set, so they are
//! not imported.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use quick_xml::XmlVersion;
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use zeroize::Zeroizing;

use super::text::{self, MAX_FILE_BYTES, MAX_FILES};
use super::{Entry, Jump, Source, non_empty};
use crate::error::{AppError, AppResult};

/// The largest XML export an import reads. A session takes about 20 KB in it, so this holds
/// [`MAX_FILES`] of them.
const MAX_EXPORT_BYTES: u64 = 16 * MAX_FILE_BYTES;

/// Where SecureCRT on Windows keeps the configuration folder chosen in its Global Options, under
/// `HKEY_CURRENT_USER`.
#[cfg(windows)]
const CONFIG_PATH_KEY: &str = r"Software\VanDyke\SecureCRT";

/// The settings an import reads; the others, passwords among them, are left out.
const READ: &[&str] = &[
    "Protocol Name",
    "Hostname",
    "[SSH2] Port",
    "Username",
    "SSH2 Authentications V2",
    "Use Global Public Key",
    "Identity Filename V2",
    "Identity Filename",
    "Firewall Name",
    "Description",
];

pub(super) fn read(path: Option<&Path>) -> AppResult<Source> {
    if let Some(file) = path.filter(|p| p.is_file()) {
        return Ok(Source {
            location: Some(file.to_string_lossy().into_owned()),
            ..read_export(file)?
        });
    }
    let Some((config, sessions)) = path.map(folders).or_else(default_folders) else {
        return Ok(Source::default());
    };
    Ok(Source {
        entries: read_sessions(&sessions, config.as_deref())?,
        location: Some(sessions.to_string_lossy().into_owned()),
        encrypted: false,
        ..Source::default()
    })
}

/// The configuration folder, when known, and the sessions folder for a chosen folder: the
/// configuration folder, its `Sessions` folder, or the `VanDyke` folder with a `Config` folder.
/// A session folder may also be called `Sessions`, so a sessions folder, called `Sessions` or
/// holding the default session's `Default.ini`, is not looked into.
fn folders(chosen: &Path) -> (Option<PathBuf>, PathBuf) {
    let named_sessions = chosen
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("Sessions"));
    let is_sessions = named_sessions
        || ["Default.ini", "__FolderData__.ini"]
            .iter()
            .any(|f| chosen.join(f).is_file());
    if !is_sessions {
        for config in [chosen.to_path_buf(), chosen.join("Config")] {
            let sessions = config.join("Sessions");
            if sessions.is_dir() {
                return (Some(config), sessions);
            }
        }
    }
    let config = named_sessions
        .then(|| chosen.parent().map(Path::to_path_buf))
        .flatten();
    (config, chosen.to_path_buf())
}

/// The configuration folder SecureCRT uses on this device, and its sessions folder: the one set
/// in SecureCRT's Global Options, or else its default place on each system.
fn default_folders() -> Option<(Option<PathBuf>, PathBuf)> {
    let chosen = chosen_config();
    let config_dir = dirs::config_dir();
    [
        chosen,
        // `%APPDATA%\VanDyke\Config` on Windows; `VanDyke/SecureCRT/Config` in macOS's Application
        // Support folder, and in `%APPDATA%` in older versions on Windows.
        config_dir
            .as_ref()
            .map(|d| d.join("VanDyke").join("Config")),
        config_dir
            .as_ref()
            .map(|d| d.join("VanDyke").join("SecureCRT").join("Config")),
        // Linux.
        dirs::home_dir().map(|h| h.join(".vandyke").join("SecureCRT").join("Config")),
    ]
    .into_iter()
    .flatten()
    .find(|d| d.join("Sessions").is_dir())
    .map(|config| (Some(config.clone()), config.join("Sessions")))
}

/// The configuration folder chosen in SecureCRT's Global Options, which may name environment
/// variables such as `%APPDATA%`.
#[cfg(windows)]
fn chosen_config() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(CONFIG_PATH_KEY)
        .ok()?;
    let path: String = key.get_value("Config Path").ok()?;
    non_empty(Some(&expand_env(&path))).map(PathBuf::from)
}

/// On macOS and Linux, a configuration folder kept elsewhere is chosen in the dialog.
#[cfg(not(windows))]
fn chosen_config() -> Option<PathBuf> {
    None
}

/// `path` with each `%NAME%` that names an environment variable replaced by its value.
#[cfg(windows)]
fn expand_env(path: &str) -> String {
    let mut out = String::new();
    let mut rest = path;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            break;
        };
        match std::env::var(&after[..end]) {
            Ok(value) if end > 0 => {
                out.push_str(&rest[..start]);
                out.push_str(&value);
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str(&rest[..=start]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The SSH sessions under `root`, the sessions folder, with the folders between as their folder.
/// The default session and the files that keep each folder's order are left out.
fn read_sessions(root: &Path, config: Option<&Path>) -> AppResult<Vec<Entry>> {
    let places = Places::new(config);
    // The key file of the sessions that use the global setting.
    let global_key = match config.map(|c| c.join("SSH2.ini")).filter(|p| p.is_file()) {
        Some(path) => identity(&parse(&read_text(&path)?)),
        None => None,
    };
    let mut entries = Vec::new();
    for (path, folder) in text::files_under(root, text::MAX_DEPTH)? {
        let Some(name) = path
            .file_stem()
            .filter(|_| {
                path.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("ini"))
            })
            .map(|n| n.to_string_lossy().into_owned())
        else {
            continue;
        };
        if name.eq_ignore_ascii_case("__FolderData__")
            || (folder.is_empty() && name.eq_ignore_ascii_case("Default"))
        {
            continue;
        }
        let settings = parse(&read_text(&path)?);
        entries.extend(entry(
            &settings,
            name,
            folder,
            global_key.as_deref(),
            &places,
        ));
    }
    link_firewalls(&mut entries);
    Ok(entries)
}

/// The sessions in the XML file that SecureCRT's Export Settings writes, and the key files it
/// holds. `${VDS_CONFIG_PATH}` in its paths stands for this device's configuration folder, as it
/// does once SecureCRT imports the file.
fn read_export(path: &Path) -> AppResult<Source> {
    let text = {
        let bytes = Zeroizing::new(text::read_file_up_to(path, MAX_EXPORT_BYTES)?);
        Zeroizing::new(text::decode(&bytes))
    };
    let export = parse_export(&text)
        .map_err(|e| AppError::invalid("path", format!("not a SecureCRT export: {e}")))?;
    if export.sessions.len() > MAX_FILES {
        return Err(AppError::io("the export holds too many sessions"));
    }
    let config = default_folders().and_then(|(config, _)| config);
    Ok(export_source(export, &Places::new(config.as_deref())))
}

/// The SSH sessions of an export, with the key file its `SSH2` settings set for all sessions, and
/// the files it holds that are their key files.
fn export_source(export: Export, places: &Places) -> Source {
    let global_key = identity(&export.ssh2);
    let mut entries: Vec<Entry> = export
        .sessions
        .into_iter()
        .filter_map(|(folder, name, settings)| {
            entry(&settings, name, folder, global_key.as_deref(), places)
        })
        .collect();
    link_firewalls(&mut entries);
    let named: HashSet<&str> = entries
        .iter()
        .flat_map(|e| e.identity_files.iter().map(String::as_str))
        .collect();
    let key_files = export
        .files
        .into_iter()
        .map(|(path, bytes)| (places.expand(&path), bytes))
        .filter(|(path, _)| named.contains(path.as_str()))
        .collect();
    Source {
        entries,
        key_files,
        ..Source::default()
    }
}

/// Points each jump to the session it names, which a firewall names by its path as SecureCRT
/// shows it, in any case.
fn link_firewalls(entries: &mut [Entry]) {
    let ids: HashMap<String, String> = entries
        .iter()
        .map(|e| (e.id.to_lowercase(), e.id.clone()))
        .collect();
    for entry in entries {
        if let Some(Jump::Entry(target)) = &mut entry.jump
            && let Some(id) = ids.get(&target.to_lowercase())
        {
            target.clone_from(id);
        }
    }
}

/// A file's text, wiped when dropped along with its bytes: it holds the passwords the session
/// saved, which only a configuration passphrase protects.
fn read_text(path: &Path) -> AppResult<Zeroizing<String>> {
    let bytes = Zeroizing::new(text::read_file(path)?);
    Ok(Zeroizing::new(text::decode(&bytes)))
}

/// One setting of a session, or of a file an export holds.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Setting {
    Text(String),
    Number(u32),
    List(Vec<String>),
    /// A binary value, such as a file's bytes; a key file's are wiped when dropped.
    Bytes(Zeroizing<Vec<u8>>),
}

/// The settings of a session file that an import reads, by name.
#[derive(Debug, Default)]
struct Settings(HashMap<String, Setting>);

impl Settings {
    fn text(&self, name: &str) -> Option<&str> {
        match self.0.get(name)? {
            Setting::Text(text) => Some(text),
            _ => None,
        }
    }

    fn number(&self, name: &str) -> Option<u32> {
        match self.0.get(name)? {
            Setting::Number(n) => Some(*n),
            _ => None,
        }
    }

    fn list(&self, name: &str) -> &[String] {
        match self.0.get(name) {
            Some(Setting::List(items)) => items,
            _ => &[],
        }
    }
}

/// The settings in a session file. A list's items, and a binary value's bytes in hex, follow on
/// lines of their own, each after a space.
fn parse(text: &str) -> Settings {
    let mut settings = Settings::default();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some((kind, name, value)) = setting_line(line) else {
            continue;
        };
        let number = || u32::from_str_radix(value.trim(), 16).ok();
        let setting = match kind {
            b'S' => Setting::Text(value.to_owned()),
            b'D' => match number() {
                Some(n) => Setting::Number(n),
                None => continue,
            },
            b'Z' => Setting::List(
                lines
                    .by_ref()
                    .take(number().unwrap_or_default() as usize)
                    .map(|item| item.strip_prefix(' ').unwrap_or(item).to_owned())
                    .collect(),
            ),
            _ => continue,
        };
        if READ.contains(&name) {
            settings.0.insert(name.to_owned(), setting);
        }
    }
    settings
}

/// `T:"Name"=value`, as its type letter, name and value.
fn setting_line(line: &str) -> Option<(u8, &str, &str)> {
    let (kind, rest) = line.split_once(":\"")?;
    let kind = match kind.as_bytes() {
        [kind] if kind.is_ascii_uppercase() => *kind,
        _ => return None,
    };
    let (name, value) = rest.split_once("\"=")?;
    Some((kind, name, value))
}

/// What an import reads from an XML export.
#[derive(Debug, Default)]
struct Export {
    /// The sessions, each with the folders it is in, its name and its settings.
    sessions: Vec<(Vec<String>, String, Settings)>,
    /// The settings in the `SSH2` key, which hold the key file for all sessions.
    ssh2: Settings,
    /// The files it holds, each with the path the settings name it by.
    files: Vec<(String, Zeroizing<Vec<u8>>)>,
}

/// The sessions and settings in an XML export: `<key>` elements in a `<VanDyke>` element, the
/// sessions and their folders in the `Sessions` key. A key's settings are `<string>` elements,
/// `<dword>` elements with a number in decimal, `<array>` elements with a `<string>` for each
/// item, and `<binary>` elements with bytes in hex. A folder's order is in a `__FolderData__`
/// key. The `Files` key holds a key for each file the settings name, with its path and bytes.
fn parse_export(text: &str) -> Result<Export, String> {
    let mut reader = quick_xml::Reader::from_str(text);
    let mut parser = ExportParser::default();
    loop {
        match reader.read_event().map_err(|e| e.to_string())? {
            Event::Start(e) => parser.start(&e, false)?,
            Event::Empty(e) => parser.start(&e, true)?,
            Event::Text(e) => parser.text(&e.xml10_content()),
            Event::CData(e) => parser.text(&e),
            Event::GeneralRef(e) => match e.resolve_char_ref().map_err(|e| e.to_string())? {
                Some(c) => parser.text(c.encode_utf8(&mut [0; 4])),
                None => parser.text(resolve_predefined_entity(&e).unwrap_or_default()),
            },
            Event::End(e) => parser.end(e.name().as_ref()),
            Event::Eof => break,
            _ => {}
        }
    }
    match parser.root {
        Root::Closed => Ok(parser.export),
        Root::Missing => Err("it has no VanDyke element".into()),
        Root::Open => Err("it ends early".into()),
    }
}

/// Whether the `VanDyke` element has been read.
#[derive(Debug, Default)]
enum Root {
    #[default]
    Missing,
    Open,
    Closed,
}

/// Reads an export one element at a time.
#[derive(Debug, Default)]
struct ExportParser {
    root: Root,
    /// The keys around the element being read, by name, with the settings each holds.
    keys: Vec<(String, Settings)>,
    /// The setting being read.
    value: Option<Value>,
    export: Export,
}

/// A setting being read from its element.
#[derive(Debug)]
struct Value {
    /// `string`, `dword`, `array` or `binary`.
    tag: &'static str,
    name: String,
    /// Wiped when dropped: a key file's bytes are read as text first.
    text: Zeroizing<String>,
    /// A list's items, and the one being read.
    items: Vec<String>,
    item: Option<String>,
}

impl ExportParser {
    fn start(&mut self, e: &BytesStart, empty: bool) -> Result<(), String> {
        let tag = e.name();
        let tag = tag.as_ref();
        match self.root {
            Root::Missing if tag == "VanDyke" => {
                self.root = if empty { Root::Closed } else { Root::Open };
                return Ok(());
            }
            Root::Missing => return Err(format!("it starts with a {tag} element")),
            Root::Open => {}
            Root::Closed => return Ok(()),
        }
        if let Some(value) = &mut self.value {
            if value.tag == "array" && tag == "string" {
                if empty {
                    value.items.push(String::new());
                } else {
                    value.item = Some(String::new());
                }
            }
            return Ok(());
        }
        let tag = match tag {
            "key" if !empty => {
                self.keys.push((name_of(e)?, Settings::default()));
                return Ok(());
            }
            "string" => "string",
            "dword" => "dword",
            "array" => "array",
            "binary" => "binary",
            // An empty key.
            _ => return Ok(()),
        };
        let value = Value {
            tag,
            name: name_of(e)?,
            text: Zeroizing::default(),
            items: Vec::new(),
            item: None,
        };
        if empty {
            self.set(value);
        } else {
            self.value = Some(value);
        }
        Ok(())
    }

    fn text(&mut self, text: &str) {
        match &mut self.value {
            Some(Value {
                item: Some(item), ..
            }) => item.push_str(text),
            Some(value) if value.tag != "array" => value.text.push_str(text),
            _ => {}
        }
    }

    fn end(&mut self, tag: &str) {
        if !matches!(self.root, Root::Open) {
            return;
        }
        match &mut self.value {
            Some(value) if value.item.is_some() => {
                if tag == "string" {
                    value.items.extend(value.item.take());
                }
            }
            Some(value) if value.tag == tag => {
                if let Some(value) = self.value.take() {
                    self.set(value);
                }
            }
            Some(_) => {}
            None if tag == "key" => {
                if let Some((name, settings)) = self.keys.pop() {
                    self.close_key(name, settings);
                }
            }
            None if tag == "VanDyke" => self.root = Root::Closed,
            None => {}
        }
    }

    /// Keeps a setting of the key being read, when an import reads it.
    fn set(&mut self, mut value: Value) {
        let in_file = matches!(
            self.keys.as_slice(),
            [(files, _), _] if files.eq_ignore_ascii_case("Files")
        );
        let Some((_, settings)) = self.keys.last_mut() else {
            return;
        };
        let name = value.name.as_str();
        if !(READ.contains(&name) || in_file && ["Path", "Contents"].contains(&name)) {
            return;
        }
        let setting = match value.tag {
            "string" => Setting::Text(std::mem::take(&mut *value.text)),
            "dword" => match value.text.trim().parse() {
                Ok(n) => Setting::Number(n),
                Err(_) => return,
            },
            "binary" => match hex_bytes(&value.text) {
                Some(bytes) => Setting::Bytes(bytes),
                None => return,
            },
            _ => Setting::List(value.items),
        };
        settings.0.insert(value.name, setting);
    }

    /// Keeps a key that has been read: a session, which holds settings, in the `Sessions` key or
    /// a folder in it, the `SSH2` key, or a file in the `Files` key. The default session and the
    /// folders' order are left out.
    fn close_key(&mut self, name: String, mut settings: Settings) {
        let Some(((top, _), folders)) = self.keys.split_first() else {
            if name.eq_ignore_ascii_case("SSH2") {
                self.export.ssh2 = settings;
            }
            return;
        };
        if top.eq_ignore_ascii_case("Files") {
            if let (Some(Setting::Text(path)), Some(Setting::Bytes(bytes))) =
                (settings.0.remove("Path"), settings.0.remove("Contents"))
            {
                self.export.files.push((path, bytes));
            }
            return;
        }
        if !top.eq_ignore_ascii_case("Sessions")
            || settings.0.is_empty()
            || name.eq_ignore_ascii_case("__FolderData__")
            || (folders.is_empty() && name.eq_ignore_ascii_case("Default"))
        {
            return;
        }
        let folder = folders.iter().map(|(name, _)| name.clone()).collect();
        self.export.sessions.push((folder, name, settings));
    }
}

/// The bytes of a binary value, in hex with a space between them.
fn hex_bytes(hex: &str) -> Option<Zeroizing<Vec<u8>>> {
    // Room for them all, so that no copy is left behind as the buffer grows.
    let mut bytes = Zeroizing::new(Vec::with_capacity(hex.len() / 2 + 1));
    for byte in hex.split_ascii_whitespace() {
        bytes.push(u8::from_str_radix(byte, 16).ok()?);
    }
    Some(bytes)
}

/// The `name` attribute of an element.
fn name_of(e: &BytesStart) -> Result<String, String> {
    match e.try_get_attribute("name").map_err(|e| e.to_string())? {
        Some(name) => Ok(name
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|e| e.to_string())?
            .into_owned()),
        None => Ok(String::new()),
    }
}

/// The private key file the settings name; older versions wrote it without `V2`.
fn identity(settings: &Settings) -> Option<String> {
    non_empty(settings.text("Identity Filename V2"))
        .or_else(|| non_empty(settings.text("Identity Filename")))
}

/// What SecureCRT's placeholders in paths stand for.
struct Places {
    /// `${VDS_CONFIG_PATH}`: the configuration folder.
    config: Option<PathBuf>,
    /// `${VDS_USER_DATA_PATH}`: the user's documents folder.
    documents: Option<PathBuf>,
    /// `${VDS_SSH_DATA_PATH}`: the user's `.ssh` folder.
    ssh: Option<PathBuf>,
}

impl Places {
    fn new(config: Option<&Path>) -> Self {
        Self {
            config: config.map(Path::to_path_buf),
            documents: dirs::document_dir(),
            ssh: dirs::home_dir().map(|home| home.join(".ssh")),
        }
    }

    fn expand(&self, path: &str) -> String {
        for (placeholder, dir) in [
            ("${VDS_CONFIG_PATH}", &self.config),
            ("${VDS_USER_DATA_PATH}", &self.documents),
            ("${VDS_SSH_DATA_PATH}", &self.ssh),
        ] {
            if let (Some(rest), Some(dir)) = (path.strip_prefix(placeholder), dir) {
                // The rest is written with the separators of the system that saved it.
                let rest = rest
                    .trim_start_matches(['/', '\\'])
                    .replace(['/', '\\'], std::path::MAIN_SEPARATOR_STR);
                return dir.join(rest).to_string_lossy().into_owned();
            }
        }
        path.to_owned()
    }
}

/// One session, by its name and the folders it is in; `None` for one that is not SSH2, such as
/// Telnet, serial, RDP or SSH1.
fn entry(
    s: &Settings,
    name: String,
    folder: Vec<String>,
    global_key: Option<&str>,
    places: &Places,
) -> Option<Entry> {
    if !s
        .text("Protocol Name")
        .is_none_or(|p| p.trim().eq_ignore_ascii_case("SSH2"))
    {
        return None;
    }
    let address = non_empty(s.text("Hostname"))?;
    // The key file counts only while public key authentication is among the session's methods.
    let public_key = s.text("SSH2 Authentications V2").is_none_or(|m| {
        m.split(',')
            .any(|m| m.trim().eq_ignore_ascii_case("publickey"))
    });
    let key = if !public_key {
        None
    } else if s.number("Use Global Public Key") == Some(0) {
        identity(s)
    } else {
        global_key.map(str::to_owned)
    };
    // A key in the Windows certificate store, such as `capi::rawkey::my`, is not a file.
    let (identity_files, kept_key) = match key {
        Some(key) if key.to_ascii_lowercase().starts_with("capi::") => (Vec::new(), Some(key)),
        Some(key) => (vec![places.expand(&key)], None),
        None => (Vec::new(), None),
    };
    // `None`, a firewall set in the Global Options, or `Session:` and another session's path.
    let firewall = s.text("Firewall Name").map(str::trim).unwrap_or_default();
    let session = firewall
        .get(.."Session:".len())
        .is_some_and(|p| p.eq_ignore_ascii_case("Session:"));
    let (jump, skipped_proxy) = if firewall.is_empty() || firewall.eq_ignore_ascii_case("None") {
        (None, None)
    } else if session {
        let target = firewall["Session:".len()..].replace('\\', "/");
        (Some(Jump::Entry(target.trim_matches('/').to_owned())), None)
    } else {
        (None, Some(format!("SecureCRT firewall {firewall}")))
    };
    let port = s
        .number("[SSH2] Port")
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0)
        .unwrap_or(22);
    Some(Entry {
        // The session's path, as a firewall names it.
        id: folder
            .iter()
            .map(String::as_str)
            .chain([name.as_str()])
            .collect::<Vec<_>>()
            .join("/"),
        name,
        address,
        port,
        username: non_empty(s.text("Username")),
        folder,
        identity_files,
        kept_key,
        jump,
        skipped_proxy,
        note: s.list("Description").join("\n").trim().to_owned(),
        ..Entry::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A session file as SecureCRT 9 writes it: UTF-8 with a byte order mark, CRLF lines, and a
    /// few hundred settings, some of them binary values or lists over several lines.
    fn session(settings: &str) -> String {
        format!(
            "\u{feff}S:\"Username\"=deploy\r\n\
             S:\"Password V2\"=02:4ffff611d02e5d7f8c94f65b5ff9b89ff97b086237df6ab06607e42905f1e2a8\r\n\
             D:\"Session Password Saved\"=00000001\r\n\
             B:\"Normal Font v2\"=00000060\r\n \
             f3 ff ff ff 00 00 00 00 00 00 00 00 00 00 00 00 90 01 00 00 00 00 00 01 00 00 00 01\r\n \
             63 00 69 00 64 00 61 00 20 00 43 00 6f 00 6e 00 73 00 6f 00 6c 00 65 00 00 00 00 00\r\n\
             D:\"Is Session\"=00000001\r\n\
             S:\"Protocol Name\"=SSH2\r\n\
             S:\"Firewall Name\"=None\r\n\
             D:\"[SSH2] Port\"=00000016\r\n\
             S:\"SSH2 Authentications V2\"=publickey,keyboard-interactive,password\r\n\
             D:\"Use Global Public Key\"=00000001\r\n\
             S:\"Identity Filename V2\"=\r\n\
             {settings}\
             Z:\"Port Forward Table V2\"=00000000\r\n\
             Z:\"SCP Shell Prompts\"=00000001\r\n \
             \"? \",0,\"\\n\"\r\n"
        )
    }

    fn places() -> Places {
        Places {
            config: Some(PathBuf::from(
                "C:\\Users\\ops\\AppData\\Roaming\\VanDyke\\Config",
            )),
            documents: Some(PathBuf::from("C:\\Users\\ops\\Documents")),
            ssh: Some(PathBuf::from("C:\\Users\\ops\\.ssh")),
        }
    }

    /// A session file with only the given settings.
    fn bare(text: &str) -> Option<Entry> {
        entry(&parse(text), "s".into(), vec![], None, &places())
    }

    fn entry_of(settings: &str, global_key: Option<&str>) -> Option<Entry> {
        entry(
            &parse(&text::decode(session(settings).as_bytes())),
            "s".into(),
            vec![],
            global_key,
            &places(),
        )
    }

    #[test]
    fn settings_parse_with_their_types_and_lists() {
        let settings = parse(&text::decode(
            session(
                "S:\"Hostname\"=db.example.org\r\nZ:\"Description\"=00000002\r\n 主数据库\r\n rack 4\r\n",
            )
            .as_bytes(),
        ));
        assert_eq!(settings.text("Hostname"), Some("db.example.org"));
        assert_eq!(settings.number("[SSH2] Port"), Some(22));
        assert_eq!(settings.list("Description"), ["主数据库", "rack 4"]);
        assert_eq!(settings.text("Password V2"), None, "passwords are not kept");
        assert_eq!(
            settings.text("Username"),
            Some("deploy"),
            "after the byte order mark"
        );
        assert_eq!(setting_line(" 63 00 69 00"), None);
        assert_eq!(setting_line("SS:\"a\"=b"), None);
    }

    #[test]
    fn ssh2_sessions_parse_and_others_are_left_out() {
        let entry = entry_of(
            "S:\"Hostname\"= db.example.org \r\nD:\"[SSH2] Port\"=000008ae\r\n\
             Z:\"Description\"=00000002\r\n 主数据库\r\n rack 4\r\n",
            None,
        )
        .unwrap();
        assert_eq!(
            (
                entry.address.as_str(),
                entry.port,
                entry.username.as_deref()
            ),
            ("db.example.org", 2222, Some("deploy")),
            "the later port setting wins"
        );
        assert_eq!(entry.note, "主数据库\nrack 4");
        assert!(entry.identity_files.is_empty() && entry.kept_key.is_none());
        assert!(entry.jump.is_none() && entry.skipped_proxy.is_none());

        assert!(bare("S:\"Protocol Name\"=Telnet\r\nS:\"Hostname\"=switch\r\n").is_none());
        assert!(bare("S:\"Protocol Name\"=SSH1\r\nS:\"Hostname\"=old\r\n").is_none());
        assert!(
            entry_of("S:\"Hostname\"=\r\n", None).is_none(),
            "no host name"
        );
        let defaults = bare("S:\"Hostname\"=10.0.0.5\r\nD:\"[SSH2] Port\"=00000000\r\n").unwrap();
        assert_eq!((defaults.port, defaults.username), (22, None));
    }

    #[test]
    fn keys_come_from_the_session_or_the_global_setting() {
        let global = Some("${VDS_USER_DATA_PATH}\\keys\\global.ppk");
        let sep = std::path::MAIN_SEPARATOR_STR;

        // The global setting, by default.
        let entry = entry_of("S:\"Hostname\"=web\r\n", global).unwrap();
        assert_eq!(
            entry.identity_files,
            [format!("C:\\Users\\ops\\Documents{sep}keys{sep}global.ppk")]
        );
        assert!(
            entry_of("S:\"Hostname\"=web\r\n", None)
                .unwrap()
                .identity_files
                .is_empty()
        );

        // The session's own key.
        let own = "S:\"Hostname\"=web\r\nD:\"Use Global Public Key\"=00000000\r\n\
                   S:\"Identity Filename V2\"=${VDS_CONFIG_PATH}/Keys/id_ed25519\r\n";
        assert_eq!(
            entry_of(own, global).unwrap().identity_files,
            [format!(
                "C:\\Users\\ops\\AppData\\Roaming\\VanDyke\\Config{sep}Keys{sep}id_ed25519"
            )]
        );
        let plain = "S:\"Hostname\"=web\r\nD:\"Use Global Public Key\"=00000000\r\n\
                     S:\"Identity Filename V2\"=D:\\keys\\web.ppk\r\n";
        assert_eq!(
            entry_of(plain, global).unwrap().identity_files,
            ["D:\\keys\\web.ppk"]
        );
        let ssh = "S:\"Hostname\"=web\r\nD:\"Use Global Public Key\"=00000000\r\n\
                   S:\"Identity Filename V2\"=${VDS_SSH_DATA_PATH}\\id_ed25519\r\n";
        assert_eq!(
            entry_of(ssh, global).unwrap().identity_files,
            [format!("C:\\Users\\ops\\.ssh{sep}id_ed25519")]
        );

        // A session that does not sign in with a public key.
        let password =
            "S:\"Hostname\"=web\r\nS:\"SSH2 Authentications V2\"=password,keyboard-interactive\r\n";
        assert!(
            entry_of(password, global)
                .unwrap()
                .identity_files
                .is_empty()
        );

        // A key in the Windows certificate store.
        let capi = "S:\"Hostname\"=web\r\nD:\"Use Global Public Key\"=00000000\r\n\
                    S:\"Identity Filename V2\"=capi::rawkey::my\r\n";
        let entry = entry_of(capi, global).unwrap();
        assert!(entry.identity_files.is_empty());
        assert_eq!(entry.kept_key.as_deref(), Some("capi::rawkey::my"));
    }

    #[test]
    fn firewalls_become_jump_hosts_or_are_left_out() {
        let via = |firewall: &str| {
            entry_of(
                &format!("S:\"Hostname\"=web\r\nS:\"Firewall Name\"={firewall}\r\n"),
                None,
            )
            .unwrap()
        };
        assert_eq!(
            via("Session:Jump/bastion").jump,
            Some(Jump::Entry("Jump/bastion".into()))
        );
        assert_eq!(
            via("session:\\Jump\\bastion").jump,
            Some(Jump::Entry("Jump/bastion".into()))
        );
        let office = via("Office SOCKS");
        assert_eq!(office.jump, None);
        assert_eq!(
            office.skipped_proxy.as_deref(),
            Some("SecureCRT firewall Office SOCKS")
        );
        assert_eq!(via("none").skipped_proxy, None);
        assert_eq!(via("").skipped_proxy, None);
    }

    /// A configuration folder with sessions in folders, as SecureCRT keeps it.
    struct Config(PathBuf);

    impl Config {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("hatoba-import-securecrt-{}", uuid::Uuid::now_v7()))
                .join("Config");
            std::fs::create_dir_all(root.join("Sessions")).unwrap();
            Self(root)
        }

        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }

    impl Drop for Config {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
        }
    }

    #[test]
    fn the_sessions_folder_is_read_with_its_folders_firewalls_and_global_key() {
        let config = Config::new();
        config.write(
            "SSH2.ini",
            "\u{feff}S:\"Identity Filename V2\"=${VDS_CONFIG_PATH}\\Keys\\id_rsa\r\nD:\"Public Key Type\"=00000000\r\n",
        );
        config.write("Sessions/Default.ini", &session("S:\"Hostname\"=\r\n"));
        config.write(
            "Sessions/__FolderData__.ini",
            "\u{feff}S:\"Folder List\"=Jump:生产:\r\nS:\"Session List\"=Default:\r\n",
        );
        config.write(
            "Sessions/Jump/Bastion.ini",
            &session("S:\"Hostname\"=bastion.example.org\r\n"),
        );
        config.write(
            "Sessions/生产/DB/pg.INI",
            &session("S:\"Hostname\"=10.0.0.2\r\nS:\"Firewall Name\"=Session:jump/bastion\r\n"),
        );
        // A session called Default is a session outside the top folder.
        config.write(
            "Sessions/生产/Default.ini",
            &session("S:\"Hostname\"=10.0.0.3\r\n"),
        );
        config.write(
            "Sessions/生产/serial.ini",
            "S:\"Protocol Name\"=Serial\r\nS:\"Com Port\"=COM3\r\n",
        );
        config.write("Sessions/readme.txt", "S:\"Hostname\"=not a session\r\n");
        // Names whose last four bytes do not start at a character.
        config.write("Sessions/文文", "");
        config.write("Sessions/生产/备份.ini.bak", "");

        let from_config = read(Some(&config.0)).unwrap();
        let from_sessions = read(Some(&config.0.join("Sessions"))).unwrap();
        let from_vandyke = read(Some(config.0.parent().unwrap())).unwrap();
        for source in [&from_sessions, &from_vandyke] {
            assert_eq!(source.location, from_config.location);
            assert_eq!(source.entries.len(), from_config.entries.len());
        }
        assert_eq!(
            from_config.location,
            Some(config.0.join("Sessions").to_string_lossy().into_owned())
        );
        let found: Vec<(&str, &str, Vec<String>)> = from_config
            .entries
            .iter()
            .map(|e| (e.id.as_str(), e.address.as_str(), e.folder.clone()))
            .collect();
        assert_eq!(
            found,
            [
                (
                    "Jump/Bastion",
                    "bastion.example.org",
                    vec!["Jump".to_owned()]
                ),
                (
                    "生产/DB/pg",
                    "10.0.0.2",
                    vec!["生产".to_owned(), "DB".to_owned()]
                ),
                ("生产/Default", "10.0.0.3", vec!["生产".to_owned()]),
            ]
        );
        let pg = &from_config.entries[1];
        assert_eq!(pg.name, "pg");
        assert_eq!(
            pg.jump,
            Some(Jump::Entry("Jump/Bastion".into())),
            "in any case"
        );
        let key = config
            .0
            .join("Keys")
            .join("id_rsa")
            .to_string_lossy()
            .into_owned();
        assert_eq!(pg.identity_files, std::slice::from_ref(&key));
        assert_eq!(from_sessions.entries[1].identity_files, [key]);
    }

    #[test]
    fn a_session_folder_called_sessions_is_a_folder() {
        let config = Config::new();
        config.write("Sessions/Default.ini", &session("S:\"Hostname\"=\r\n"));
        config.write(
            "Sessions/web.ini",
            &session("S:\"Hostname\"=web.example.org\r\n"),
        );
        config.write(
            "Sessions/Sessions/cache.ini",
            &session("S:\"Hostname\"=cache.example.org\r\n"),
        );
        let ids = || -> Vec<String> {
            let source = read(Some(&config.0.join("Sessions"))).unwrap();
            source.entries.into_iter().map(|e| e.id).collect()
        };
        assert_eq!(ids(), ["Sessions/cache", "web"]);
        // A copy without the default session, told by its name.
        std::fs::remove_file(config.0.join("Sessions").join("Default.ini")).unwrap();
        assert_eq!(ids(), ["Sessions/cache", "web"]);
    }

    #[test]
    fn a_thousand_sessions_read() {
        let config = Config::new();
        for i in 0..1_200 {
            config.write(
                &format!("Sessions/Site {}/sw{i:04}.ini", i / 100),
                &session(&format!("S:\"Hostname\"=10.0.{}.{}\r\n", i / 250, i % 250)),
            );
        }
        let source = read(Some(&config.0)).unwrap();
        assert_eq!(source.entries.len(), 1_200);
        let in_site = |site: &str| source.entries.iter().filter(|e| e.folder == [site]).count();
        assert_eq!((in_site("Site 0"), in_site("Site 11")), (100, 100));
    }

    /// An XML export as SecureCRT 9 writes it, with most of each key's settings left out: tabs,
    /// the settings in order of name, and the files the settings name, such as key files.
    const EXPORT: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<VanDyke version="3.0">
	<key name="Security">
		<string name="Passphrase">03:637dca491ee5dc1979744cecc6d4b0d662285574c56fc6b2aa97afaa2b9a5429</string>
	</key>
	<key name="Global">
		<array name="Status Bar Item List">
			<string>Protocol Information</string>
			<string>Cursor Position</string>
		</array>
		<string name="Username">not a session</string>
	</key>
	<key name="Sessions">
		<key name="Jump">
			<key name="Bastion">
				<dword name="[SSH2] Port">22</dword>
				<array name="Description"/>
				<string name="Firewall Name">None</string>
				<string name="Hostname">bastion.example.org</string>
				<string name="Identity Filename V2"/>
				<dword name="Is Session">1</dword>
				<string name="Password V2">02:4ffff611d02e5d7f8c94f65b5ff9b89ff97b086237df6ab06607e429</string>
				<string name="Protocol Name">SSH2</string>
				<string name="SSH2 Authentications V2">publickey,keyboard-interactive,password,gssapi</string>
				<dword name="Use Global Public Key">1</dword>
				<string name="Username">jump</string>
			</key>
			<key name="__FolderData__">
				<string name="Hostname">not a session</string>
				<string name="Session List">Bastion:</string>
			</key>
		</key>
		<key name="生产">
			<key name="DB">
				<key name="pg">
					<dword name="[SSH2] Port">2222</dword>
					<array name="Description">
						<string>主数据库</string>
						<string>rack 4 &amp; 5 &lt;east&gt;</string>
					</array>
					<string name="Firewall Name">Session:jump/bastion</string>
					<string name="Hostname">10.0.0.2</string>
					<dword name="Is Session">1</dword>
					<binary name="Normal Font v2">f3 ff ff ff 00 00 00 00</binary>
					<string name="Protocol Name">SSH2</string>
					<dword name="Use Global Public Key">1</dword>
					<string name="Username">postgres</string>
				</key>
			</key>
			<key name="Default">
				<string name="Hostname">10.0.0.3</string>
				<dword name="Is Session">1</dword>
				<string name="Protocol Name">SSH2</string>
				<dword name="Use Global Public Key">0</dword>
				<string name="Identity Filename V2">${VDS_SSH_DATA_PATH}\id_ed25519</string>
			</key>
			<key name="console">
				<dword name="Is Session">1</dword>
				<string name="Protocol Name">Serial</string>
				<string name="Com Port">COM3</string>
			</key>
			<key name="Office">
				<string name="Firewall Name">Office SOCKS</string>
				<string name="Hostname">office.example.org</string>
				<dword name="Is Session">1</dword>
				<string name="Protocol Name">SSH2</string>
			</key>
			<key name="switch">
				<dword name="Port">23</dword>
				<string name="Hostname">10.0.9.1</string>
				<dword name="Is Session">1</dword>
				<string name="Protocol Name">Telnet</string>
			</key>
		</key>
		<key name="Default">
			<string name="Hostname">template.example.org</string>
			<dword name="Is Session">1</dword>
			<string name="Protocol Name">SSH2</string>
		</key>
		<key name="Default_RDP">
			<string name="Hostname">desktop.example.org</string>
			<dword name="Is Session">1</dword>
			<string name="Protocol Name">RDP</string>
		</key>
		<key name="web">
			<dword name="[SSH2] Port">0</dword>
			<string name="Hostname">web.example.org</string>
			<dword name="Is Session">1</dword>
			<string name="Protocol Name">SSH2</string>
			<string name="Username"/>
		</key>
	</key>
	<key name="Commands"/>
	<key name="SSH2">
		<string name="Identity Filename V2">${VDS_CONFIG_PATH}\Keys\id_rsa</string>
		<dword name="Public Key Type">0</dword>
		<array name="Agent Keys To Load"/>
	</key>
	<key name="Files">
		<key name="File 0">
			<string name="Path">${VDS_CONFIG_PATH}\KnownHosts\HostKeyDB.txt</string>
			<binary name="Contents">ef bb bf 32 2c 31 30 2e 30 2e 30 2e 32</binary>
		</key>
		<key name="File 1">
			<string name="Path">${VDS_CONFIG_PATH}\Keys\id_rsa</string>
			<binary name="Contents">2d 2d 2d 2d 2d 42 45 47 49 4e</binary>
		</key>
		<key name="File 2">
			<string name="Path">${VDS_SSH_DATA_PATH}\id_ed25519</string>
			<binary name="Contents">6b 65 79</binary>
		</key>
		<key name="File 3">
			<string name="Path">${VDS_SSH_DATA_PATH}\id_ed25519.pub</string>
			<binary name="Contents">73 73 68</binary>
		</key>
	</key>
</VanDyke>
"#;

    #[test]
    fn an_export_is_read_with_its_folders_firewalls_and_global_key() {
        let source = export_source(parse_export(EXPORT).unwrap(), &places());
        let entries = &source.entries;
        let found: Vec<(&str, &str, u16, Vec<String>)> = entries
            .iter()
            .map(|e| (e.id.as_str(), e.address.as_str(), e.port, e.folder.clone()))
            .collect();
        assert_eq!(
            found,
            [
                (
                    "Jump/Bastion",
                    "bastion.example.org",
                    22,
                    vec!["Jump".to_owned()]
                ),
                (
                    "生产/DB/pg",
                    "10.0.0.2",
                    2222,
                    vec!["生产".to_owned(), "DB".to_owned()]
                ),
                ("生产/Default", "10.0.0.3", 22, vec!["生产".to_owned()]),
                (
                    "生产/Office",
                    "office.example.org",
                    22,
                    vec!["生产".to_owned()]
                ),
                ("web", "web.example.org", 22, vec![]),
            ]
        );
        let sep = std::path::MAIN_SEPARATOR_STR;
        let global =
            format!("C:\\Users\\ops\\AppData\\Roaming\\VanDyke\\Config{sep}Keys{sep}id_rsa");
        let (bastion, pg, default, office, web) = (
            &entries[0],
            &entries[1],
            &entries[2],
            &entries[3],
            &entries[4],
        );
        assert_eq!(
            (bastion.name.as_str(), bastion.username.as_deref()),
            ("Bastion", Some("jump"))
        );
        assert_eq!(bastion.identity_files, std::slice::from_ref(&global));
        assert_eq!(pg.username.as_deref(), Some("postgres"));
        assert_eq!(pg.note, "主数据库\nrack 4 & 5 <east>");
        assert_eq!(
            pg.jump,
            Some(Jump::Entry("Jump/Bastion".into())),
            "in any case"
        );
        assert_eq!(pg.identity_files, [global]);
        assert_eq!(
            default.identity_files,
            [format!("C:\\Users\\ops\\.ssh{sep}id_ed25519")]
        );
        assert_eq!(
            office.skipped_proxy.as_deref(),
            Some("SecureCRT firewall Office SOCKS")
        );
        assert_eq!((web.username.as_deref(), web.note.as_str()), (None, ""));
        assert_eq!(
            web.identity_files, pg.identity_files,
            "the global setting by default"
        );

        // The key files the sessions name; not the known hosts or a public key.
        let mut held: Vec<(&str, &[u8])> = source
            .key_files
            .iter()
            .map(|(path, bytes)| (path.as_str(), bytes.as_slice()))
            .collect();
        held.sort_unstable();
        assert_eq!(
            held,
            [
                (default.identity_files[0].as_str(), b"key".as_slice()),
                (pg.identity_files[0].as_str(), b"-----BEGIN".as_slice()),
            ]
        );
    }

    #[test]
    fn an_export_is_read_from_the_file_chosen() {
        let config = Config::new();
        config.write(
            "SecureCRT.xml",
            &format!("\u{feff}{}", EXPORT.replace('\n', "\r\n")),
        );
        let file = config.0.join("SecureCRT.xml");
        let source = read(Some(&file)).unwrap();
        assert_eq!(source.location, Some(file.to_string_lossy().into_owned()));
        let ids: Vec<&str> = source.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "Jump/Bastion",
                "生产/DB/pg",
                "生产/Default",
                "生产/Office",
                "web"
            ]
        );

        config.write("Sessions/web.ini", &session("S:\"Hostname\"=web\r\n"));
        let error = read(Some(&config.0.join("Sessions").join("web.ini"))).unwrap_err();
        assert_eq!(error.code, crate::error::ErrorCode::InvalidInput);
    }

    #[test]
    fn a_thousand_sessions_read_from_an_export() {
        let mut xml = String::from("<VanDyke version=\"3.0\">\n<key name=\"Sessions\">\n");
        for site in 0..12 {
            xml.push_str(&format!("<key name=\"Site {site}\">\n"));
            for i in 0..100 {
                xml.push_str(&format!(
                    "<key name=\"sw{i:02}\">\n\
                     <dword name=\"[SSH2] Port\">22</dword>\n\
                     <array name=\"Description\"/>\n\
                     <string name=\"Hostname\">10.0.{site}.{i}</string>\n\
                     <dword name=\"Is Session\">1</dword>\n\
                     <string name=\"Protocol Name\">SSH2</string>\n\
                     </key>\n"
                ));
            }
            xml.push_str("</key>\n");
        }
        xml.push_str("</key>\n</VanDyke>\n");
        let entries = export_source(parse_export(&xml).unwrap(), &places()).entries;
        assert_eq!(entries.len(), 1_200);
        let in_site = |site: &str| entries.iter().filter(|e| e.folder == [site]).count();
        assert_eq!((in_site("Site 0"), in_site("Site 11")), (100, 100));
    }

    #[test]
    fn a_file_that_is_not_an_export_is_refused() {
        assert!(parse_export("<VanDyke version=\"3.0\"></VanDyke>").is_ok());
        assert!(parse_export("<?xml version=\"1.0\"?>\n<config/>").is_err());
        assert!(parse_export("<VanDyke><key name=\"Sessions\">").is_err());
        assert!(parse_export("<VanDyke><key name=\"Sessions\"></string></VanDyke>").is_err());
        assert!(parse_export("S:\"Hostname\"=web").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn environment_variables_expand() {
        let windir = std::env::var("windir").unwrap();
        assert_eq!(
            expand_env("%windir%\\VanDyke"),
            format!("{windir}\\VanDyke")
        );
        assert_eq!(
            expand_env("100% %NOT_SET_HATOBA% x"),
            "100% %NOT_SET_HATOBA% x"
        );
        assert_eq!(expand_env("a%%b"), "a%%b");
    }
}
