//! What the panel's `@` menu reads on the tab's host (AI-51): a directory's entries, to complete
//! a path and to attach a folder, and a text file, to attach it. Like the file tools, they run
//! fixed scripts over an exec channel of the tab's connection (`hatoba_ssh::remote_file`), never
//! use `sudo`, and stop after 30 seconds. Nothing here is logged: paths and file names are the
//! host's.

use std::future::Future;
use std::time::Duration;

use super::conn::Conn;
use hatoba_ssh::remote_file::RemoteFileError;

use crate::dropped::{FILE_MAX_BYTES, text_of};
use crate::dto::{AiHostEntry, AiHostFile, AiHostListing, AiHostRefusal};

/// How long a listing or a read may take.
const TIMEOUT: Duration = Duration::from_secs(30);
/// The most a listing reads of the host's output: some thousands of entries.
const MAX_LISTING_BYTES: u64 = 256 * 1024;

/// The entries of the directory at `path` on the session's host, sorted by name.
pub async fn list(session: Option<Conn>, path: &str) -> AiHostListing {
    let Some(session) = session.filter(|s| !s.is_closed()) else {
        return AiHostListing::Refused {
            reason: AiHostRefusal::Disconnected,
        };
    };
    match within(session.list_dir(path, MAX_LISTING_BYTES)).await {
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
/// [`FILE_MAX_BYTES`] as a text file from this computer (AI-35), without a byte order mark.
pub async fn read(session: Option<Conn>, path: &str) -> AiHostFile {
    let Some(session) = session.filter(|s| !s.is_closed()) else {
        return AiHostFile::Refused {
            reason: AiHostRefusal::Disconnected,
        };
    };
    match within(session.read(path, FILE_MAX_BYTES)).await {
        Ok(file) => match text_of(file.bytes) {
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
fn refusal(session: &Conn, error: Option<RemoteFileError>) -> AiHostRefusal {
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
        assert_eq!(text_of(b"a\r\nb".to_vec()).as_deref(), Some("a\r\nb"));
        assert_eq!(
            text_of("\u{feff}x".as_bytes().to_vec()).as_deref(),
            Some("x")
        );
        assert_eq!(text_of(b"a\0b".to_vec()), None);
        assert_eq!(text_of(vec![0xff, 0xfe]), None);
        assert_eq!(text_of(Vec::new()).as_deref(), Some(""));
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
