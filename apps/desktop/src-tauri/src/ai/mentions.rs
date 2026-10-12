//! What the panel's `@` menu reads on the tab's host (AI-51): a directory's entries, to complete
//! a path and to attach a folder, and a text file, to attach it. Like the file tools, they run
//! fixed scripts over an exec channel of the tab's connection (`hatoba_ssh::remote_file`), never
//! use `sudo`, and stop after 30 seconds. Nothing here is logged: paths and file names are the
//! host's.

use std::future::Future;
use std::time::Duration;

use hatoba_ssh::SshSession;
use hatoba_ssh::remote_file::{self, RemoteFileError};

use crate::dto::{AiHostEntry, AiHostFile, AiHostListing, AiHostRefusal};

/// How long a listing or a read may take.
const TIMEOUT: Duration = Duration::from_secs(30);
/// The most a listing reads of the host's output: some thousands of entries.
const MAX_LISTING_BYTES: u64 = 256 * 1024;
/// The largest file an `@` mention attaches, as a text file from this computer (AI-35).
pub const MAX_ATTACH_BYTES: u64 = 256 * 1024;

/// The entries of the directory at `path` on the session's host, sorted by name.
pub async fn list(session: Option<SshSession>, path: &str) -> AiHostListing {
    let Some(session) = session.filter(|s| !s.is_closed()) else {
        return AiHostListing::Refused {
            reason: AiHostRefusal::Disconnected,
        };
    };
    match within(remote_file::list_dir(&session, path, MAX_LISTING_BYTES)).await {
        Ok(dir) => {
            let mut entries: Vec<AiHostEntry> = dir
                .entries
                .into_iter()
                .map(|e| AiHostEntry {
                    name: e.name,
                    dir: e.dir,
                })
                .collect();
            entries.sort_by(|a, b| a.name.cmp(&b.name));
            AiHostListing::Ok {
                path: dir.path,
                entries,
                cut: dir.cut,
            }
        }
        Err(reason) => AiHostListing::Refused {
            reason: refusal(&session, reason),
        },
    }
}

/// The text file at `path` on the session's host: UTF-8 without NUL bytes, at most
/// [`MAX_ATTACH_BYTES`], without a byte order mark.
pub async fn read(session: Option<SshSession>, path: &str) -> AiHostFile {
    let Some(session) = session.filter(|s| !s.is_closed()) else {
        return AiHostFile::Refused {
            reason: AiHostRefusal::Disconnected,
        };
    };
    match within(remote_file::read(&session, path, MAX_ATTACH_BYTES)).await {
        Ok(file) => match text(file.bytes) {
            Some(text) => AiHostFile::Ok {
                path: file.path,
                text,
            },
            None => AiHostFile::Refused {
                reason: AiHostRefusal::Binary,
            },
        },
        Err(reason) => AiHostFile::Refused {
            reason: refusal(&session, reason),
        },
    }
}

/// A file's bytes as text: UTF-8 without NUL bytes, its byte order mark left out.
fn text(bytes: Vec<u8>) -> Option<String> {
    if bytes.contains(&0) {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    Some(match text.strip_prefix('\u{feff}') {
        Some(rest) => rest.to_owned(),
        None => text,
    })
}

/// The operation within [`TIMEOUT`]; a timeout counts as a failure.
async fn within<T>(
    operation: impl Future<Output = Result<T, RemoteFileError>>,
) -> Result<T, Option<RemoteFileError>> {
    match tokio::time::timeout(TIMEOUT, operation).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(Some(e)),
        Err(_) => Err(None),
    }
}

/// Why a listing or a read failed, for the panel.
fn refusal(session: &SshSession, error: Option<RemoteFileError>) -> AiHostRefusal {
    match error {
        Some(RemoteFileError::NotFound) => AiHostRefusal::NotFound,
        Some(RemoteFileError::IsDirectory | RemoteFileError::NotDirectory) => {
            AiHostRefusal::WrongKind
        }
        Some(RemoteFileError::NotRegular | RemoteFileError::DanglingLink) => {
            AiHostRefusal::NotRegular
        }
        Some(RemoteFileError::PermissionDenied) => AiHostRefusal::PermissionDenied,
        Some(RemoteFileError::TooLarge(_)) => AiHostRefusal::TooLarge,
        _ if session.is_closed() => AiHostRefusal::Disconnected,
        _ => AiHostRefusal::Failed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_files_are_utf8_without_nul_or_bom() {
        assert_eq!(text(b"a\r\nb".to_vec()).as_deref(), Some("a\r\nb"));
        assert_eq!(text("\u{feff}x".as_bytes().to_vec()).as_deref(), Some("x"));
        assert_eq!(text(b"a\0b".to_vec()), None);
        assert_eq!(text(vec![0xff, 0xfe]), None);
        assert_eq!(text(Vec::new()).as_deref(), Some(""));
    }

    #[tokio::test]
    async fn a_disconnected_tab_lists_and_reads_nothing() {
        assert_eq!(
            list(None, "~").await,
            AiHostListing::Refused {
                reason: AiHostRefusal::Disconnected
            }
        );
        assert_eq!(
            read(None, "x").await,
            AiHostFile::Refused {
                reason: AiHostRefusal::Disconnected
            }
        );
    }
}
