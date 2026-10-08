//! Import and export of MCP server lists (AI-33).
//!
//! Import reads the JSON that other clients keep their servers in: the `mcpServers` object of
//! Claude Desktop, Claude Code (`.mcp.json`) and Cursor (`mcp.json`), or the `servers` object of
//! VS Code (`.vscode/mcp.json`, which may have comments and trailing commas). A bare object of
//! servers is accepted too. The text is read once; environment and header values move into
//! [`Zeroizing`] strings for the vault, and the parsed copies of the text are wiped (best effort:
//! the caller's string is the caller's to wipe).
//!
//! Export writes the `mcpServers` format with placeholders such as `"<API_KEY>"` in place of
//! environment and header values, which never leave the vault.

use std::collections::HashSet;
use std::fmt;

use serde::de::{Deserialize, Deserializer, IgnoredAny, MapAccess, Visitor};
use serde_json::Value;
use zeroize::Zeroizing;

use super::{McpError, McpTransportConfig};

/// Longest import text read.
const MAX_IMPORT_BYTES: usize = 1024 * 1024;

/// A server found in an import.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImportedServer {
    /// The key it had in the file, trimmed.
    pub name: String,
    /// How to reach it.
    pub transport: McpTransportConfig,
}

/// What an import found, in file order. `Debug` never prints environment or header values.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// Servers that can be added.
    pub servers: Vec<ImportedServer>,
    /// Entries that cannot, as `(name, reason)`: an unsupported transport such as `sse`, a
    /// missing command or URL, malformed fields, or a name used twice.
    pub skipped: Vec<(String, String)>,
}

/// AI-33: parses pasted JSON or a file's text in the `mcpServers` format (Claude Desktop, Claude
/// Code, Cursor) or with VS Code's `servers` key.
///
/// An entry is stdio when its `type` is `stdio` or it has a `command` (with `args` and `env`),
/// and Streamable HTTP when its `type` is `http` or `streamable-http`, or it has a `url` (or
/// Gemini CLI's `httpUrl`, Windsurf's `serverUrl`) and no command (with `headers`). Other
/// fields (`cwd`, `envFile`, `disabled`, VS Code's `inputs`) are ignored. Entries that cannot be
/// used are listed in [`ImportReport::skipped`] and do not stop the others.
///
/// Errors: the text is not JSON (only the line and column are named, never content), or it holds
/// no server list ([`McpError::InvalidConfig`]).
pub fn parse_import(json: &str) -> Result<ImportReport, McpError> {
    if json.len() > MAX_IMPORT_BYTES {
        return Err(McpError::InvalidConfig(
            "the text is too large for an MCP server list".into(),
        ));
    }
    let text = Zeroizing::new(strip_jsonc(json.trim_start_matches('\u{feff}')));
    let top: TopLevel = serde_json::from_str(&text).map_err(|e| {
        McpError::InvalidConfig(if e.is_data() {
            "\"mcpServers\" and \"servers\" must be objects that map names to servers".to_owned()
        } else {
            format!(
                "the text is not valid JSON (line {}, column {})",
                e.line(),
                e.column()
            )
        })
    })?;

    let entries = if top.found_list {
        top.others.into_iter().for_each(|(key, value)| {
            drop(Zeroizing::new(key));
            wipe(value);
        });
        top.lists
    } else if !top.others.is_empty() && top.others.iter().all(|(_, v)| looks_like_server(v)) {
        top.others
    } else {
        top.others.into_iter().for_each(|(_, value)| wipe(value));
        return Err(McpError::InvalidConfig(
            "no MCP servers found: expected an \"mcpServers\" object (Claude Desktop, Claude \
             Code, Cursor) or a \"servers\" object (VS Code)"
                .into(),
        ));
    };

    let mut report = ImportReport::default();
    let mut names = HashSet::new();
    for (key, value) in entries {
        let name = key.trim().to_owned();
        let parsed = if name.is_empty() {
            wipe(value);
            Err("the server has no name".to_owned())
        } else if !names.insert(name.clone()) {
            wipe(value);
            Err("another server in the import has the same name".to_owned())
        } else {
            parse_entry(value)
        };
        match parsed {
            Ok(transport) => report.servers.push(ImportedServer { name, transport }),
            Err(reason) => report.skipped.push((name, reason)),
        }
    }
    Ok(report)
}

/// The top level: the server lists in order, and the other keys (a bare server map, maybe).
struct TopLevel {
    found_list: bool,
    lists: Vec<(String, Value)>,
    others: Vec<(String, Value)>,
}

impl<'de> Deserialize<'de> for TopLevel {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TopVisitor;
        impl<'de> Visitor<'de> for TopVisitor {
            type Value = TopLevel;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<TopLevel, A::Error> {
                let mut top = TopLevel {
                    found_list: false,
                    lists: Vec::new(),
                    others: Vec::new(),
                };
                while let Some(key) = map.next_key::<String>()? {
                    if key == "mcpServers" || key == "servers" {
                        top.found_list = true;
                        top.lists.extend(map.next_value::<Entries>()?.0);
                    } else if key == "inputs" {
                        // VS Code's prompts for secrets; nothing to import.
                        map.next_value::<IgnoredAny>()?;
                    } else {
                        let value = map.next_value::<Value>()?;
                        top.others.push((key, value));
                    }
                }
                Ok(top)
            }
        }
        deserializer.deserialize_map(TopVisitor)
    }
}

/// A JSON object's entries in file order (`serde_json::Map` would sort them). `null` is empty.
struct Entries(Vec<(String, Value)>);

impl<'de> Deserialize<'de> for Entries {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EntriesVisitor;
        impl<'de> Visitor<'de> for EntriesVisitor {
            type Value = Entries;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object of servers")
            }
            fn visit_unit<E: serde::de::Error>(self) -> Result<Entries, E> {
                Ok(Entries(Vec::new()))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Entries, A::Error> {
                let mut entries = Vec::new();
                while let Some((key, value)) = map.next_entry::<String, Value>()? {
                    entries.push((key, value));
                }
                Ok(Entries(entries))
            }
        }
        deserializer.deserialize_any(EntriesVisitor)
    }
}

/// Why an `sse` entry is skipped.
const SSE_UNSUPPORTED: &str =
    "SSE servers are not supported; use the server's Streamable HTTP URL if it has one";

/// Keys that name an HTTP server's URL, in order of preference.
const URL_KEYS: [&str; 3] = ["httpUrl", "url", "serverUrl"];

fn looks_like_server(value: &Value) -> bool {
    value.as_object().is_some_and(|entry| {
        entry.contains_key("command") || URL_KEYS.iter().any(|k| entry.contains_key(*k))
    })
}

fn parse_entry(value: Value) -> Result<McpTransportConfig, String> {
    let Value::Object(mut entry) = value else {
        wipe(value);
        return Err("the entry is not an object".into());
    };
    let kind = entry
        .get("type")
        .and_then(Value::as_str)
        .map(|t| t.trim().to_ascii_lowercase());
    let url_key = URL_KEYS.into_iter().find(|k| entry.contains_key(*k));
    let result = match kind.as_deref() {
        Some("stdio") => stdio_entry(&mut entry),
        Some("http" | "streamable-http" | "streamable_http" | "streamablehttp") => {
            http_entry(&mut entry, url_key)
        }
        Some("sse") => Err(SSE_UNSUPPORTED.into()),
        Some(other) => Err(format!(
            "the transport type \"{}\" is not supported",
            other
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
                .take(30)
                .collect::<String>()
        )),
        None if entry.contains_key("command") => stdio_entry(&mut entry),
        None if url_key.is_some() => http_entry(&mut entry, url_key),
        None => Err("the entry has neither a command nor a url".into()),
    };
    wipe(Value::Object(entry));
    result
}

fn stdio_entry(entry: &mut serde_json::Map<String, Value>) -> Result<McpTransportConfig, String> {
    let command = match entry.get("command") {
        Some(Value::String(command)) if !command.trim().is_empty() => command.trim().to_owned(),
        _ => return Err("the command is missing".into()),
    };
    let args = match entry.remove("args") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .into_iter()
            .map(scalar)
            .collect::<Option<Vec<_>>>()
            .ok_or("args must be a list of strings")?,
        Some(other) => {
            wipe(other);
            return Err("args must be a list of strings".into());
        }
    };
    let env = secret_pairs(entry.remove("env"), "env")?;
    Ok(McpTransportConfig::Stdio { command, args, env })
}

fn http_entry(
    entry: &mut serde_json::Map<String, Value>,
    url_key: Option<&str>,
) -> Result<McpTransportConfig, String> {
    let url = match url_key.and_then(|key| entry.get(key)) {
        Some(Value::String(url)) if !url.trim().is_empty() => url.trim().to_owned(),
        _ => return Err("the url is missing".into()),
    };
    if !url::Url::parse(&url).is_ok_and(|u| matches!(u.scheme(), "http" | "https")) {
        return Err("the url is not an http or https URL".into());
    }
    let headers = secret_pairs(entry.remove("headers"), "headers")?;
    Ok(McpTransportConfig::Http { url, headers })
}

/// A string, or a number or boolean written without quotes.
fn scalar(value: Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        other => {
            wipe(other);
            None
        }
    }
}

/// `env` or `headers`: names to secret values, with `null` values left out. The strings are
/// moved, not copied, into `Zeroizing`.
fn secret_pairs(
    value: Option<Value>,
    what: &str,
) -> Result<Vec<(String, Zeroizing<String>)>, String> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Object(map)) => {
            let mut pairs = Vec::with_capacity(map.len());
            let mut bad = false;
            for (name, value) in map {
                match value {
                    Value::Null => {}
                    other if bad => wipe(other),
                    other => match scalar(other) {
                        Some(text) => pairs.push((name, Zeroizing::new(text))),
                        None => bad = true,
                    },
                }
            }
            if bad {
                Err(format!("{what} values must be strings"))
            } else {
                Ok(pairs)
            }
        }
        Some(other) => {
            wipe(other);
            Err(format!("{what} must be an object"))
        }
    }
}

/// Drops a parsed value with its strings wiped (they may be secrets).
fn wipe(value: Value) {
    match value {
        Value::String(text) => drop(Zeroizing::new(text)),
        Value::Array(items) => items.into_iter().for_each(wipe),
        Value::Object(map) => map.into_iter().for_each(|(key, value)| {
            drop(Zeroizing::new(key));
            wipe(value);
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// JSON with comments (`//`, `/* */`) and trailing commas made plain JSON, as VS Code writes
/// `mcp.json`. Strings are left alone.
fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if in_string {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        let rest = &text[at + c.len_utf8()..];
        match c {
            '"' => {
                in_string = true;
                out.push(c);
            }
            '/' if rest.starts_with('/') => {
                // A line comment: skip to the line break, which is kept.
                while chars.next_if(|&(_, n)| n != '\n').is_some() {}
            }
            '/' if rest.starts_with('*') => {
                chars.next();
                let mut previous = '\0';
                for (_, n) in chars.by_ref() {
                    if previous == '*' && n == '/' {
                        break;
                    }
                    previous = n;
                }
                out.push(' ');
            }
            ',' if matches!(next_significant(rest), Some('}' | ']')) => {}
            _ => out.push(c),
        }
    }
    out
}

/// The next character that is not whitespace or inside a comment.
fn next_significant(mut rest: &str) -> Option<char> {
    loop {
        let trimmed = rest.trim_start();
        if let Some(after) = trimmed.strip_prefix("//") {
            rest = after.find('\n').map_or("", |at| &after[at..]);
        } else if let Some(after) = trimmed.strip_prefix("/*") {
            rest = after.find("*/").map_or("", |at| &after[at + 2..]);
        } else {
            return trimmed.chars().next();
        }
    }
}

/// AI-33: the servers in the `mcpServers` format (stdio as `command`/`args`/`env`, HTTP as
/// `"type": "http"` with `url`/`headers`), pretty-printed in the given order. Every environment
/// and header value is replaced by a placeholder naming it, such as `"<API_KEY>"`. A name used
/// twice gets `-2`, `-3`, … so every key stays unique.
#[must_use]
pub fn export_json(servers: &[(String, McpTransportConfig)]) -> String {
    if servers.is_empty() {
        return "{\n  \"mcpServers\": {}\n}\n".to_owned();
    }
    let mut used = HashSet::new();
    let mut entries = Vec::with_capacity(servers.len());
    for (name, transport) in servers {
        let mut key = name.clone();
        let mut n = 2;
        while !used.insert(key.clone()) {
            key = format!("{name}-{n}");
            n += 1;
        }
        let mut fields = Vec::new();
        match transport {
            McpTransportConfig::Stdio { command, args, env } => {
                fields.push(format!("\"command\": {}", quote(command)));
                let args: Vec<String> = args.iter().map(|a| quote(a)).collect();
                fields.push(format!("\"args\": [{}]", args.join(", ")));
                if !env.is_empty() {
                    fields.push(format!("\"env\": {}", placeholders(env)));
                }
            }
            McpTransportConfig::Http { url, headers } => {
                fields.push("\"type\": \"http\"".to_owned());
                fields.push(format!("\"url\": {}", quote(url)));
                if !headers.is_empty() {
                    fields.push(format!("\"headers\": {}", placeholders(headers)));
                }
            }
        }
        entries.push(format!(
            "    {}: {{\n      {}\n    }}",
            quote(&key),
            fields.join(",\n      ")
        ));
    }
    format!(
        "{{\n  \"mcpServers\": {{\n{}\n  }}\n}}\n",
        entries.join(",\n")
    )
}

fn quote(text: &str) -> String {
    serde_json::to_string(text).expect("a string always serializes")
}

/// `{ "NAME": "<NAME>", … }` for `env` or `headers`, each name once.
fn placeholders(pairs: &[(String, Zeroizing<String>)]) -> String {
    let mut seen = HashSet::new();
    let lines: Vec<String> = pairs
        .iter()
        .filter(|(name, _)| seen.insert(name.as_str()))
        .map(|(name, _)| format!("        {}: {}", quote(name), quote(&format!("<{name}>"))))
        .collect();
    format!("{{\n{}\n      }}", lines.join(",\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret(value: &str) -> Zeroizing<String> {
        Zeroizing::new(value.to_owned())
    }

    fn stdio(command: &str, args: &[&str], env: &[(&str, &str)]) -> McpTransportConfig {
        McpTransportConfig::Stdio {
            command: command.into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), secret(v)))
                .collect(),
        }
    }

    fn http(url: &str, headers: &[(&str, &str)]) -> McpTransportConfig {
        McpTransportConfig::Http {
            url: url.into(),
            headers: headers
                .iter()
                .map(|(k, v)| ((*k).to_owned(), secret(v)))
                .collect(),
        }
    }

    fn server(name: &str, transport: McpTransportConfig) -> ImportedServer {
        ImportedServer {
            name: name.into(),
            transport,
        }
    }

    #[test]
    fn claude_desktop() {
        let report = parse_import(
            r#"{
              "mcpServers": {
                "filesystem": {
                  "command": "npx",
                  "args": ["-y", "@modelcontextprotocol/server-filesystem", "C:\\Users\\me\\Desktop"]
                },
                "brave": {
                  "command": "npx",
                  "args": ["-y", "@modelcontextprotocol/server-brave-search"],
                  "env": {"BRAVE_API_KEY": "brv-secret", "PORT": 8080, "UNSET": null}
                }
              },
              "globalShortcut": "Ctrl+Space"
            }"#,
        )
        .unwrap();
        assert_eq!(
            report.servers,
            vec![
                server(
                    "filesystem",
                    stdio(
                        "npx",
                        &[
                            "-y",
                            "@modelcontextprotocol/server-filesystem",
                            r"C:\Users\me\Desktop"
                        ],
                        &[]
                    )
                ),
                server(
                    "brave",
                    stdio(
                        "npx",
                        &["-y", "@modelcontextprotocol/server-brave-search"],
                        &[("BRAVE_API_KEY", "brv-secret"), ("PORT", "8080")]
                    )
                ),
            ]
        );
        assert!(report.skipped.is_empty());
        let debug = format!("{report:?}");
        assert!(
            !debug.contains("brv-secret") && debug.contains("BRAVE_API_KEY"),
            "{debug}"
        );
    }

    #[test]
    fn claude_code_and_cursor() {
        let report = parse_import(
            r#"{"mcpServers": {
                "local": {"type": "stdio", "command": "uvx", "args": ["mcp-server-git"], "env": {}},
                "github": {
                    "type": "http",
                    "url": "https://api.githubcopilot.com/mcp/",
                    "headers": {"Authorization": "Bearer ghp_secret"}
                },
                "events": {"type": "sse", "url": "https://example.com/sse"},
                "cursor-remote": {"url": "https://mcp.example.com/mcp", "headers": {"X-Key": "k1"}},
                "gemini-remote": {"httpUrl": "https://g.example.com/mcp", "url": "https://g.example.com/sse"},
                "windsurf-remote": {"serverUrl": "https://w.example.com/mcp"}
            }}"#,
        )
        .unwrap();
        assert_eq!(
            report.servers,
            vec![
                server("local", stdio("uvx", &["mcp-server-git"], &[])),
                server(
                    "github",
                    http(
                        "https://api.githubcopilot.com/mcp/",
                        &[("Authorization", "Bearer ghp_secret")]
                    )
                ),
                server(
                    "cursor-remote",
                    http("https://mcp.example.com/mcp", &[("X-Key", "k1")])
                ),
                server("gemini-remote", http("https://g.example.com/mcp", &[])),
                server("windsurf-remote", http("https://w.example.com/mcp", &[])),
            ]
        );
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].0, "events");
        assert!(report.skipped[0].1.contains("SSE"));
        assert!(!format!("{report:?}").contains("ghp_secret"));
    }

    #[test]
    fn vs_code_with_comments_and_trailing_commas() {
        let report = parse_import(
            "\u{feff}{
              // Servers for this workspace
              \"inputs\": [{\"type\": \"promptString\", \"id\": \"key\", \"password\": true}],
              \"servers\": {
                \"memory\": {
                  \"type\": \"stdio\",
                  \"command\": \"npx\", /* inline */
                  \"args\": [\"-y\", \"@modelcontextprotocol/server-memory\",],
                  \"env\": {\"KEY\": \"${input:key}\"},
                  \"envFile\": \"${workspaceFolder}/.env\",
                },
                \"docs\": {\"type\": \"http\", \"url\": \"http://localhost:3000/mcp\"},
              },
            }",
        )
        .unwrap();
        assert_eq!(
            report.servers,
            vec![
                server(
                    "memory",
                    stdio(
                        "npx",
                        &["-y", "@modelcontextprotocol/server-memory"],
                        &[("KEY", "${input:key}")]
                    )
                ),
                server("docs", http("http://localhost:3000/mcp", &[])),
            ]
        );
    }

    #[test]
    fn strings_that_look_like_comments_are_kept() {
        let report = parse_import(
            r#"{"servers": {"s": {"command": "run", "args": ["//not a comment", "/* nor this */", "a,}"]}}}"#,
        )
        .unwrap();
        assert_eq!(
            report.servers,
            vec![server(
                "s",
                stdio("run", &["//not a comment", "/* nor this */", "a,}"], &[])
            )]
        );
    }

    #[test]
    fn a_bare_map_of_servers_is_accepted() {
        let report =
            parse_import(r#"{"fs": {"command": "npx"}, "web": {"url": "https://x.example/mcp"}}"#)
                .unwrap();
        assert_eq!(
            report.servers,
            vec![
                server("fs", stdio("npx", &[], &[])),
                server("web", http("https://x.example/mcp", &[]))
            ]
        );
    }

    #[test]
    fn bad_entries_are_skipped_with_reasons() {
        let report = parse_import(
            r#"{"mcpServers": {
                "ok": {"command": "a"},
                "  ": {"command": "b"},
                "ok ": {"command": "c"},
                "no-command": {"type": "stdio", "args": ["x"]},
                "bad-args": {"command": "x", "args": "-y pkg"},
                "bad-arg": {"command": "x", "args": [{"nested": true}]},
                "bad-env": {"command": "x", "env": {"K": ["v"]}},
                "ws": {"type": "websocket", "url": "wss://x"},
                "ftp": {"url": "ftp://x.example/mcp"},
                "empty": {},
                "not-object": "npx"
            }}"#,
        )
        .unwrap();
        assert_eq!(report.servers, vec![server("ok", stdio("a", &[], &[]))]);
        let skipped: Vec<(&str, &str)> = report
            .skipped
            .iter()
            .map(|(n, r)| (n.as_str(), r.as_str()))
            .collect();
        assert_eq!(
            skipped,
            vec![
                ("", "the server has no name"),
                ("ok", "another server in the import has the same name"),
                ("no-command", "the command is missing"),
                ("bad-args", "args must be a list of strings"),
                ("bad-arg", "args must be a list of strings"),
                ("bad-env", "env values must be strings"),
                ("ws", "the transport type \"websocket\" is not supported"),
                ("ftp", "the url is not an http or https URL"),
                ("empty", "the entry has neither a command nor a url"),
                ("not-object", "the entry is not an object"),
            ]
        );
    }

    #[test]
    fn errors_never_echo_the_text() {
        for (text, wanted) in [
            (
                r#"{"mcpServers": {"x": {"env": {"K": "sk-secret"}}"#,
                "not valid JSON",
            ),
            ("sk-secret", "not valid JSON"),
            (r#""sk-secret""#, "must be objects"),
            (r#"{"mcpServers": ["sk-secret"]}"#, "must be objects"),
            (
                r#"{"theme": "dark", "token": "sk-secret"}"#,
                "no MCP servers found",
            ),
            ("[]", "must be objects"),
        ] {
            let err = parse_import(text).unwrap_err();
            let message = err.to_string();
            assert!(matches!(err, McpError::InvalidConfig(_)), "{text}");
            assert!(message.contains(wanted), "{text}: {message}");
            assert!(!message.contains("sk-secret"), "{message}");
        }
        assert_eq!(
            parse_import(r#"{"mcpServers": {}}"#).unwrap(),
            ImportReport::default()
        );
        assert_eq!(
            parse_import(r#"{"mcpServers": null}"#).unwrap(),
            ImportReport::default()
        );
        let huge = format!("{{\"x\": \"{}\"}}", "a".repeat(MAX_IMPORT_BYTES));
        assert!(parse_import(&huge).is_err());
    }

    #[test]
    fn export_uses_placeholders_and_round_trips() {
        let servers = vec![
            (
                "fs".to_owned(),
                stdio(
                    "npx",
                    &["-y", "@modelcontextprotocol/server-filesystem", r"C:\a b"],
                    &[("API_KEY", "sk-secret"), ("API_KEY", "sk-other")],
                ),
            ),
            (
                "remote".to_owned(),
                http(
                    "https://mcp.example.com/mcp",
                    &[("Authorization", "Bearer tok-secret")],
                ),
            ),
            ("fs".to_owned(), stdio("uvx", &[], &[])),
        ];
        let json = export_json(&servers);
        assert!(
            !json.contains("secret") && !json.contains("sk-other"),
            "{json}"
        );
        assert_eq!(
            json,
            r#"{
  "mcpServers": {
    "fs": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "C:\\a b"],
      "env": {
        "API_KEY": "<API_KEY>"
      }
    },
    "remote": {
      "type": "http",
      "url": "https://mcp.example.com/mcp",
      "headers": {
        "Authorization": "<Authorization>"
      }
    },
    "fs-2": {
      "command": "uvx",
      "args": []
    }
  }
}
"#
        );
        let back = parse_import(&json).unwrap();
        assert_eq!(
            back.servers,
            vec![
                server(
                    "fs",
                    stdio(
                        "npx",
                        &["-y", "@modelcontextprotocol/server-filesystem", r"C:\a b"],
                        &[("API_KEY", "<API_KEY>")]
                    )
                ),
                server(
                    "remote",
                    http(
                        "https://mcp.example.com/mcp",
                        &[("Authorization", "<Authorization>")]
                    )
                ),
                server("fs-2", stdio("uvx", &[], &[])),
            ]
        );
        assert_eq!(export_json(&[]), "{\n  \"mcpServers\": {}\n}\n");
        serde_json::from_str::<Value>(&export_json(&[])).unwrap();
    }
}
