//! What the `hatoba` command and the running app say to each other over the endpoint
//! ([`crate::endpoint`]): the command sends one [`Frame::Request`], and the app answers with
//! frames until [`Frame::End`]. Each frame is a kind byte, the length of its payload as a
//! little-endian `u32`, and the payload: raw bytes for output, UTF-8 for a notice and a token, JSON
//! for the rest.
//!
//! An `exec` that passes its stdin is asked for it with [`Frame::SendStdin`]: the command then
//! opens a second connection, sends [`Op::Stdin`] with the token, and after it the stdin as it
//! comes, closing the connection where the stdin ends. The first connection stays free for the
//! output, and its end still stops the command.
//!
//! The memory requests are answered with what the command prints: the memories or what was done
//! as [`Frame::Stdout`], and a line for the person as [`Frame::Notice`]. A host that
//! [`Op::HostAdd`] or [`Op::HostEdit`] saved comes back as [`Frame::Hosts`], and what
//! [`Op::HostDelete`] did as [`Frame::Stdout`].

use std::fmt;
use std::io::{self, Read};

use serde::{Deserialize, Serialize};

/// Changes when a request or a frame changes in a way the other side cannot read.
pub const VERSION: u32 = 4;

/// The largest payload either side sends or takes.
pub const MAX_FRAME: usize = 1 << 20;

/// The kind byte and the length.
pub const HEADER: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub version: u32,
    #[serde(flatten)]
    pub op: Op,
}

/// What a request asks for. A host is a saved host's id or name; local paths are absolute.
/// `sudo` runs the command, or reads or writes the remote file, as root with sudo; a version 1
/// request has none, and still reads, so the app can say the versions differ.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Hosts,
    Exec {
        host: String,
        command: String,
        cwd: Option<String>,
        prompt: bool,
        #[serde(default)]
        sudo: bool,
        /// The command passes its stdin, when [`Frame::SendStdin`] asks for it.
        #[serde(default)]
        stdin: bool,
    },
    Upload {
        host: String,
        local: String,
        remote: String,
        prompt: bool,
        #[serde(default)]
        sudo: bool,
        /// The permission bits of the file written; `None` keeps those of a file already there.
        #[serde(default)]
        mode: Option<u32>,
    },
    Download {
        host: String,
        remote: String,
        local: String,
        prompt: bool,
        #[serde(default)]
        sudo: bool,
    },
    /// The stdin of the `exec` that [`Frame::SendStdin`] gave `token`, which follows on this
    /// connection as raw bytes.
    Stdin {
        token: String,
    },
    /// The AI assistant's memories of `host` (AI-53), as text or as JSON.
    Memories {
        host: String,
        json: bool,
    },
    /// Saves `content` as the memory `name` of `host`, in place of its memory of that name.
    MemorySave {
        host: String,
        name: String,
        content: String,
    },
    /// Deletes the memory `name` of `host`.
    MemoryDelete {
        host: String,
        name: String,
    },
    /// Saves a new host as **New Host** does (HOST-01), with `changes` in its fields;
    /// `changes.address` is needed. `prompt` asks in the window for the password of a host that
    /// signs in with one when `changes` holds none. Boxed, as the largest request.
    HostAdd {
        changes: Box<HostChanges>,
        prompt: bool,
    },
    /// Changes the fields of `host` that `changes` names, and leaves the others as they are.
    HostEdit {
        host: String,
        changes: Box<HostChanges>,
        prompt: bool,
    },
    /// Deletes `host` with its forwarding rules, as **Delete** on the Hosts page does.
    HostDelete {
        host: String,
    },
}

/// The fields `hatoba hosts add` and `edit` set. `None`, `false` and an empty list leave a field
/// as it is, or for a new host as the host editor starts it. A key, a jump host and a proxy are
/// named by their id or their name, a group by its name or its id.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HostChanges {
    pub name: Option<String>,
    pub address: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub auth: Option<AuthKind>,
    /// The key the host signs in with.
    pub key: Option<String>,
    /// The password `--password-stdin` read, which the host signs in with.
    pub password: Option<String>,
    pub jump: Option<String>,
    /// Connects directly, without a jump host.
    pub no_jump: bool,
    /// A saved proxy, `direct` or `default` (SSH-13).
    pub proxy: Option<String>,
    /// A name no group has creates the group.
    pub group: Option<String>,
    pub no_group: bool,
    /// Tags to add.
    pub tags: Vec<String>,
    /// Tags to remove.
    pub untags: Vec<String>,
    /// Environment variables to set (SSH-14), in place of those of the same name.
    pub env: Vec<EnvVar>,
    /// The names of environment variables to remove.
    pub unset_env: Vec<String>,
    /// Replaces the note; empty clears it.
    pub note: Option<String>,
    /// Replaces the AI Notes (AI-37); empty clears them.
    pub ai_notes: Option<String>,
    pub favorite: Option<bool>,
}

/// Leaves the password out, so a request logged by mistake holds none.
impl fmt::Debug for HostChanges {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostChanges")
            .field("name", &self.name)
            .field("address", &self.address)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("auth", &self.auth)
            .field("key", &self.key)
            .field("password", &self.password.as_ref().map(|_| "…"))
            .field("jump", &self.jump)
            .field("no_jump", &self.no_jump)
            .field("proxy", &self.proxy)
            .field("group", &self.group)
            .field("no_group", &self.no_group)
            .field("tags", &self.tags)
            .field("untags", &self.untags)
            .field("env", &self.env)
            .field("unset_env", &self.unset_env)
            .field("note", &self.note)
            .field("ai_notes", &self.ai_notes)
            .field("favorite", &self.favorite)
            .finish()
    }
}

/// One of a host's environment variables (SSH-14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVar {
    pub name: String,
    pub value: String,
}

/// A saved host, without its credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    pub id: String,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub username: String,
    pub auth: AuthKind,
    /// The name of the key it signs in with.
    #[serde(default)]
    pub key: Option<String>,
    /// The group's name.
    pub group: Option<String>,
    pub tags: Vec<String>,
    /// The jump host's name.
    pub jump: Option<String>,
    /// `default` for this device's default proxy, `direct`, or the saved proxy's name (SSH-13).
    pub proxy: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env: Vec<EnvVar>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// What the host's settings tell AI assistants about it (AI-37).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ai_notes: String,
}

/// How a host signs in. `Ask` is a password that is not saved, which Hatoba asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    Password,
    Key,
    Agent,
    Ask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub bytes: u64,
    /// 0 when unknown.
    pub total: u64,
}

/// How a request ended: `status` is the exit status of a command that ran, `error` says why
/// nothing ran or what went wrong.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct End {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl End {
    pub fn ok() -> Self {
        Self::default()
    }

    pub fn status(status: i32) -> Self {
        Self {
            status: Some(status),
            error: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            status: None,
            error: Some(message.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    Request(Request),
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    /// A line for the person at the terminal, such as what Hatoba waits for.
    Notice(String),
    /// Asks the command for its stdin on a second connection, with this token ([`Op::Stdin`]).
    SendStdin(String),
    Progress(Progress),
    Hosts(Vec<HostInfo>),
    End(End),
}

const REQUEST: u8 = b'R';
const STDOUT: u8 = b'1';
const STDERR: u8 = b'2';
const NOTICE: u8 = b'N';
const SEND_STDIN: u8 = b'I';
const PROGRESS: u8 = b'P';
const HOSTS: u8 = b'H';
const END: u8 = b'X';

impl Frame {
    /// The frame as it goes over the endpoint. A payload over [`MAX_FRAME`] is the sender's
    /// mistake; output is sent in smaller pieces.
    pub fn encode(&self) -> Vec<u8> {
        let (kind, payload) = match self {
            Self::Request(r) => (REQUEST, json(r)),
            Self::Stdout(b) => (STDOUT, b.clone()),
            Self::Stderr(b) => (STDERR, b.clone()),
            Self::Notice(text) => (NOTICE, text.as_bytes().to_vec()),
            Self::SendStdin(token) => (SEND_STDIN, token.as_bytes().to_vec()),
            Self::Progress(p) => (PROGRESS, json(p)),
            Self::Hosts(h) => (HOSTS, json(h)),
            Self::End(e) => (END, json(e)),
        };
        let mut out = Vec::with_capacity(HEADER + payload.len());
        out.push(kind);
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(&payload);
        out
    }

    /// The kind byte and payload length of a header, or an error for a payload too large.
    pub fn header(header: [u8; HEADER]) -> io::Result<(u8, usize)> {
        let len = u32::from_le_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if len > MAX_FRAME {
            return Err(invalid(format!("a frame of {len} bytes is too large")));
        }
        Ok((header[0], len))
    }

    pub fn decode(kind: u8, payload: Vec<u8>) -> io::Result<Self> {
        Ok(match kind {
            REQUEST => Self::Request(parse(&payload)?),
            STDOUT => Self::Stdout(payload),
            STDERR => Self::Stderr(payload),
            NOTICE => Self::Notice(String::from_utf8_lossy(&payload).into_owned()),
            SEND_STDIN => Self::SendStdin(String::from_utf8_lossy(&payload).into_owned()),
            PROGRESS => Self::Progress(parse(&payload)?),
            HOSTS => Self::Hosts(parse(&payload)?),
            END => Self::End(parse(&payload)?),
            other => return Err(invalid(format!("unknown frame kind {other}"))),
        })
    }

    /// Reads the next frame; `None` when the other side closed the endpoint between frames.
    pub fn read(from: &mut impl Read) -> io::Result<Option<Self>> {
        let mut header = [0u8; HEADER];
        match from.read_exact(&mut header) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
        let (kind, len) = Self::header(header)?;
        let mut payload = vec![0u8; len];
        from.read_exact(&mut payload)?;
        Self::decode(kind, payload).map(Some)
    }
}

fn json(value: &impl Serialize) -> Vec<u8> {
    serde_json::to_vec(value).expect("protocol types serialize")
}

fn parse<'a, T: Deserialize<'a>>(payload: &'a [u8]) -> io::Result<T> {
    serde_json::from_slice(payload).map_err(|e| invalid(e.to_string()))
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_read_back_as_they_were_sent() {
        let frames = [
            Frame::Request(Request {
                version: VERSION,
                op: Op::Exec {
                    host: "db".into(),
                    command: "ls -la".into(),
                    cwd: Some("/srv".into()),
                    prompt: true,
                    sudo: true,
                    stdin: true,
                },
            }),
            Frame::Request(Request {
                version: VERSION,
                op: Op::Stdin { token: "t1".into() },
            }),
            Frame::Request(Request {
                version: VERSION,
                op: Op::Memories {
                    host: "db".into(),
                    json: true,
                },
            }),
            Frame::Request(Request {
                version: VERSION,
                op: Op::MemorySave {
                    host: "db".into(),
                    name: "sudo".into(),
                    content: "Uses doas.\nNot sudo.".into(),
                },
            }),
            Frame::Request(Request {
                version: VERSION,
                op: Op::MemoryDelete {
                    host: "db".into(),
                    name: "sudo".into(),
                },
            }),
            Frame::Request(Request {
                version: VERSION,
                op: Op::HostAdd {
                    changes: Box::new(HostChanges {
                        name: Some("web-3".into()),
                        address: Some("10.0.0.7".into()),
                        username: Some("deploy".into()),
                        key: Some("ci".into()),
                        jump: Some("bastion".into()),
                        tags: vec!["tokyo".into()],
                        env: vec![EnvVar {
                            name: "TZ".into(),
                            value: "Asia/Tokyo".into(),
                        }],
                        ..HostChanges::default()
                    }),
                    prompt: true,
                },
            }),
            Frame::Request(Request {
                version: VERSION,
                op: Op::HostEdit {
                    host: "web-3".into(),
                    changes: Box::new(HostChanges {
                        port: Some(2222),
                        password: Some("pw".into()),
                        no_group: true,
                        untags: vec!["tokyo".into()],
                        unset_env: vec!["TZ".into()],
                        ai_notes: Some(String::new()),
                        favorite: Some(false),
                        ..HostChanges::default()
                    }),
                    prompt: false,
                },
            }),
            Frame::Request(Request {
                version: VERSION,
                op: Op::HostDelete {
                    host: "web-3".into(),
                },
            }),
            Frame::Stdout(b"out\n".to_vec()),
            Frame::Stderr(Vec::new()),
            Frame::Notice("waiting".into()),
            Frame::SendStdin("t1".into()),
            Frame::Progress(Progress { bytes: 5, total: 9 }),
            Frame::Hosts(vec![HostInfo {
                id: "h1".into(),
                name: "db".into(),
                address: "10.0.0.5".into(),
                port: 22,
                username: "deploy".into(),
                auth: AuthKind::Ask,
                key: None,
                group: None,
                tags: vec!["prod".into()],
                jump: Some("bastion".into()),
                proxy: "default".into(),
                env: Vec::new(),
                favorite: true,
                note: "Primary.".into(),
                ai_notes: String::new(),
            }]),
            Frame::End(End::status(3)),
            Frame::End(End::error("locked")),
        ];
        let mut bytes = Vec::new();
        for frame in &frames {
            bytes.extend(frame.encode());
        }
        let mut reader = bytes.as_slice();
        for frame in &frames {
            assert_eq!(Frame::read(&mut reader).unwrap().as_ref(), Some(frame));
        }
        assert_eq!(Frame::read(&mut reader).unwrap(), None);
    }

    #[test]
    fn requests_name_their_operation() {
        let json = serde_json::to_value(Request {
            version: VERSION,
            op: Op::Hosts,
        })
        .unwrap();
        assert_eq!(json, serde_json::json!({"version": VERSION, "op": "hosts"}));
    }

    #[test]
    fn a_version_1_request_still_reads() {
        let request: Request = parse(
            br#"{"version":1,"op":"exec","host":"db","command":"ls","cwd":null,"prompt":true}"#,
        )
        .unwrap();
        assert_eq!(request.version, 1);
        assert!(matches!(request.op, Op::Exec { sudo: false, .. }));
    }

    #[test]
    fn a_host_change_shows_no_password_when_debugged() {
        let changes = HostChanges {
            password: Some("PASSWORD-SECRET".into()),
            ..HostChanges::default()
        };
        let text = format!("{changes:?}");
        assert!(!text.contains("PASSWORD-SECRET"), "{text}");
        assert!(text.contains("password: Some"), "{text}");
    }

    #[test]
    fn a_frame_too_large_is_refused() {
        let mut header = [b'1', 0, 0, 0, 0];
        header[1..].copy_from_slice(&((MAX_FRAME as u32) + 1).to_le_bytes());
        assert!(Frame::header(header).is_err());
        assert!(Frame::decode(b'?', Vec::new()).is_err());
    }
}
