//! What the `hatoba` command and the running app say to each other over the endpoint
//! ([`crate::endpoint`]): the command sends one [`Frame::Request`], and the app answers with
//! frames until [`Frame::End`]. Each frame is a kind byte, the length of its payload as a
//! little-endian `u32`, and the payload: raw bytes for output, UTF-8 for a notice, JSON for the
//! rest.

use std::io::{self, Read};

use serde::{Deserialize, Serialize};

/// Changes when a request or a frame changes in a way the other side cannot read.
pub const VERSION: u32 = 1;

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Hosts,
    Exec {
        host: String,
        command: String,
        cwd: Option<String>,
        prompt: bool,
    },
    Upload {
        host: String,
        local: String,
        remote: String,
        prompt: bool,
    },
    Download {
        host: String,
        remote: String,
        local: String,
        prompt: bool,
    },
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
    /// The group's name.
    pub group: Option<String>,
    pub tags: Vec<String>,
    /// The jump host's name.
    pub jump: Option<String>,
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
    Progress(Progress),
    Hosts(Vec<HostInfo>),
    End(End),
}

const REQUEST: u8 = b'R';
const STDOUT: u8 = b'1';
const STDERR: u8 = b'2';
const NOTICE: u8 = b'N';
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
                },
            }),
            Frame::Stdout(b"out\n".to_vec()),
            Frame::Stderr(Vec::new()),
            Frame::Notice("waiting".into()),
            Frame::Progress(Progress { bytes: 5, total: 9 }),
            Frame::Hosts(vec![HostInfo {
                id: "h1".into(),
                name: "db".into(),
                address: "10.0.0.5".into(),
                port: 22,
                username: "deploy".into(),
                auth: AuthKind::Ask,
                group: None,
                tags: vec!["prod".into()],
                jump: Some("bastion".into()),
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
    fn a_frame_too_large_is_refused() {
        let mut header = [b'1', 0, 0, 0, 0];
        header[1..].copy_from_slice(&((MAX_FRAME as u32) + 1).to_le_bytes());
        assert!(Frame::header(header).is_err());
        assert!(Frame::decode(b'?', Vec::new()).is_err());
    }
}
