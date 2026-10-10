//! The file tools on the tab's host (AI-38…40) and the preview the approval card shows for
//! `edit_file` and `write_file` (spec §13.4, "File tools"). The text work is in
//! `hatoba_ai::files`, the exec scripts in `hatoba_ssh::remote_file`.

use std::time::Duration;

use hatoba_ai::entry::ToolStatus;
use hatoba_ai::files::{self, MAX_FILE_BYTES, TextFile};
use hatoba_ai::tools::{self, EditFileArgs, ReadFileArgs, WriteFileArgs};
use hatoba_ssh::SshSession;
use hatoba_ssh::remote_file::{self, Expect, RemoteFileError};
use tokio_util::sync::CancellationToken;

use super::parse;
use crate::dto::AiFilePreview;

/// How long a read or a write may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A file tool's result while the tab has no live session (AI-08).
const DISCONNECTED: &str = "The terminal tab is disconnected, so the tool did not run. Ask the \
     user to reconnect the tab.";

/// A call of a file tool, with its arguments.
pub(super) enum FileJob {
    Read(ReadFileArgs),
    Edit(EditFileArgs),
    Write(WriteFileArgs),
}

impl FileJob {
    /// `Some` for a file tool's name: its arguments, or the message for arguments that do not
    /// parse.
    pub(super) fn parse(name: &str, arguments: &str) -> Option<Result<Self, String>> {
        Some(match name {
            tools::READ_FILE => parse(arguments).map(Self::Read),
            tools::EDIT_FILE => parse(arguments).map(Self::Edit),
            tools::WRITE_FILE => parse(arguments).map(Self::Write),
            _ => return None,
        })
    }

    fn path(&self) -> &str {
        match self {
            Self::Read(a) => &a.path,
            Self::Edit(a) => &a.path,
            Self::Write(a) => &a.path,
        }
    }
}

/// What the approval card read for a call: the file's bytes, or `None` when it did not exist.
/// The write checks the file against it, so the user approved the change to this content.
#[derive(Clone, Debug)]
pub(super) struct Base {
    pub path: String,
    pub bytes: Option<Vec<u8>>,
}

/// Runs a file tool. `base` is what the approval card read for the call, if it showed one.
pub(super) async fn run(
    session: Option<SshSession>,
    job: FileJob,
    base: Option<Base>,
    cancel: &CancellationToken,
) -> (ToolStatus, String) {
    let Some(session) = session else {
        return (ToolStatus::Error, DISCONNECTED.to_owned());
    };
    let work = async {
        match job {
            FileJob::Read(args) => read(&session, &args).await,
            FileJob::Edit(args) => edit(&session, &args, base).await,
            FileJob::Write(args) => write(&session, &args, base).await,
        }
    };
    tokio::select! {
        biased;
        () = cancel.cancelled() => (ToolStatus::Cancelled, String::new()),
        result = work => match result {
            Ok(text) => (ToolStatus::Ok, text),
            Err(_) if session.is_closed() => (ToolStatus::Error, DISCONNECTED.to_owned()),
            Err(message) => (ToolStatus::Error, message),
        },
    }
}

/// The diff the approval card shows for an `edit_file` or `write_file` call, and what it read,
/// to keep for the run. `cached` is what an earlier preview of the call read, used again for its
/// path so that editing the call on the card does not read the file on every change.
pub(super) async fn preview(
    session: Option<SshSession>,
    name: &str,
    arguments: &str,
    cached: Option<Base>,
) -> (AiFilePreview, Option<Base>) {
    let job = match FileJob::parse(name, arguments) {
        Some(Ok(job @ (FileJob::Edit(_) | FileJob::Write(_)))) => job,
        Some(Err(message)) => return (failed_preview("", message), None),
        _ => {
            return (failed_preview("", format!("{name} shows no diff.")), None);
        }
    };
    let path = job.path().to_owned();
    let base = match cached.filter(|b| b.path == path) {
        Some(base) => base,
        None => {
            let Some(session) = session else {
                return (failed_preview(&path, DISCONNECTED.to_owned()), None);
            };
            match read_bytes(&session, &path).await {
                Ok(bytes) => Base {
                    path: path.clone(),
                    bytes,
                },
                Err(_) if session.is_closed() => {
                    return (failed_preview(&path, DISCONNECTED.to_owned()), None);
                }
                Err(message) => return (failed_preview(&path, message), None),
            }
        }
    };
    let before = match base.bytes.as_deref().map(|b| decode(&path, b)).transpose() {
        Ok(before) => before,
        Err(message) => return (failed_preview(&path, message), Some(base)),
    };
    let after = match &job {
        FileJob::Edit(args) => before
            .as_ref()
            .ok_or_else(|| missing_for_edit(&path))
            .and_then(|file| files::apply_edit(&path, file, args).map(|e| e.file)),
        FileJob::Write(args) => new_content(&path, before.as_ref(), &args.content),
        FileJob::Read(_) => Err(format!("{name} shows no diff.")),
    };
    let crlf = before.as_ref().is_some_and(|f| f.crlf);
    let preview = match after {
        Ok(after) => AiFilePreview {
            path,
            before: before.as_ref().map(TextFile::display_text),
            after: Some(after.display_text()),
            error: None,
            crlf,
        },
        Err(message) => AiFilePreview {
            error: Some(message),
            ..failed_preview(&path, String::new())
        },
    };
    (preview, Some(base))
}

fn failed_preview(path: &str, message: String) -> AiFilePreview {
    AiFilePreview {
        path: path.to_owned(),
        before: None,
        after: None,
        error: Some(message),
        crlf: false,
    }
}

async fn read(session: &SshSession, args: &ReadFileArgs) -> Result<String, String> {
    let bytes = read_bytes(session, &args.path)
        .await?
        .ok_or_else(|| format!("{} does not exist.", args.path))?;
    let file = decode(&args.path, &bytes)?;
    files::read_page(&args.path, &file, args.offset, args.limit)
}

async fn edit(
    session: &SshSession,
    args: &EditFileArgs,
    base: Option<Base>,
) -> Result<String, String> {
    let (bytes, from_card) = current(session, &args.path, base).await?;
    let bytes = bytes.ok_or_else(|| missing_for_edit(&args.path))?;
    let edit = files::apply_edit(&args.path, &decode(&args.path, &bytes)?, args)?;
    let new = edit.file.encode();
    write_bytes(
        session,
        &args.path,
        &new,
        Expect::Content(&bytes),
        from_card,
    )
    .await?;
    Ok(files::edit_result(&args.path, &edit))
}

async fn write(
    session: &SshSession,
    args: &WriteFileArgs,
    base: Option<Base>,
) -> Result<String, String> {
    let (bytes, from_card) = current(session, &args.path, base).await?;
    let before = bytes
        .as_deref()
        .map(|b| decode(&args.path, b))
        .transpose()?;
    let file = new_content(&args.path, before.as_ref(), &args.content)?;
    let expect = bytes.as_deref().map_or(Expect::Missing, Expect::Content);
    write_bytes(session, &args.path, &file.encode(), expect, from_card).await?;
    Ok(files::write_result(&args.path, &file, bytes.is_none()))
}

/// The file `write_file` writes: `content` in the style of the file it replaces.
fn new_content(path: &str, before: Option<&TextFile>, content: &str) -> Result<TextFile, String> {
    let file = match before {
        Some(before) => before.with_text(content),
        None => TextFile {
            text: content.to_owned(),
            bom: false,
            crlf: false,
        },
    };
    files::check_size(path, &file)?;
    Ok(file)
}

/// The file's content to change: what the approval card read for this path, or the file as it
/// is now. The flag says it came from the card.
async fn current(
    session: &SshSession,
    path: &str,
    base: Option<Base>,
) -> Result<(Option<Vec<u8>>, bool), String> {
    match base.filter(|b| b.path == path) {
        Some(base) => Ok((base.bytes, true)),
        None => Ok((read_bytes(session, path).await?, false)),
    }
}

/// The file's bytes, `None` when it does not exist, or the message for the model.
async fn read_bytes(session: &SshSession, path: &str) -> Result<Option<Vec<u8>>, String> {
    match tokio::time::timeout(TIMEOUT, remote_file::read(session, path, MAX_FILE_BYTES)).await {
        Err(_) => Err(timed_out(path, Op::Read)),
        Ok(Ok(bytes)) => Ok(Some(bytes)),
        Ok(Err(RemoteFileError::NotFound)) => Ok(None),
        Ok(Err(e)) => Err(describe(path, Op::Read, e, false)),
    }
}

/// Writes `bytes` if the file is as `expect` says. `from_card`: `expect` is what the approval
/// card read.
async fn write_bytes(
    session: &SshSession,
    path: &str,
    bytes: &[u8],
    expect: Expect<'_>,
    from_card: bool,
) -> Result<(), String> {
    match tokio::time::timeout(TIMEOUT, remote_file::write(session, path, bytes, expect)).await {
        Err(_) => Err(timed_out(path, Op::Write)),
        Ok(result) => result.map_err(|e| describe(path, Op::Write, e, from_card)),
    }
}

fn decode(path: &str, bytes: &[u8]) -> Result<TextFile, String> {
    TextFile::decode(bytes).map_err(|why| {
        format!("{path} cannot be read as text: {why}. The file tools read UTF-8 text files only.")
    })
}

fn missing_for_edit(path: &str) -> String {
    format!("{path} does not exist. Use write_file to create it.")
}

fn timed_out(path: &str, op: Op) -> String {
    match op {
        Op::Read => format!("Reading {path} did not finish within 30 seconds."),
        Op::Write => format!(
            "Writing {path} did not finish within 30 seconds. Hatoba closed its channel; the file \
             may be partly written, so read it again before you change it."
        ),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Read,
    Write,
}

/// The message for the model when a read or write did not happen.
fn describe(path: &str, op: Op, error: RemoteFileError, from_card: bool) -> String {
    let verb = if op == Op::Read { "read" } else { "write" };
    match error {
        RemoteFileError::InvalidPath => {
            "The path is empty or has a line break, so nothing was read or written.".to_owned()
        }
        RemoteFileError::NotFound => format!("{path} does not exist."),
        RemoteFileError::NoDirectory => format!(
            "The directory of {path} does not exist, so the file was not created. Create the \
             directory first, for example with mkdir -p in run_command."
        ),
        RemoteFileError::IsDirectory => format!("{path} is a directory, not a file."),
        RemoteFileError::NotRegular => format!(
            "{path} is not a regular file (it is a device, a pipe or a socket), so the file tools \
             do not {verb} it."
        ),
        RemoteFileError::PermissionDenied => format!(
            "Permission denied: the user this tab is logged in as cannot {verb} {path}. The file \
             tools never use sudo. To go on, use send_input to run a command with sudo in the \
             terminal's shell, where the user can enter a password, or ask the user."
        ),
        RemoteFileError::TooLarge(size) => format!(
            "{path} is {size} bytes, and the file tools read files up to 1 MB. Use run_command \
             with grep, head, tail or sed -n to read parts of it."
        ),
        RemoteFileError::Changed if from_card => format!(
            "{path} changed on the host after Hatoba read it for the approval card, so nothing \
             was written. Read it again with read_file before you change it."
        ),
        RemoteFileError::Changed => format!(
            "{path} changed on the host while Hatoba was changing it, so nothing was written. \
             Read it again with read_file before you change it."
        ),
        RemoteFileError::Failed { status, stderr } => {
            let mut message = format!("Could not {verb} {path}: ");
            match status {
                Some(code) => message.push_str(&format!("the command exited with status {code}")),
                None => message.push_str("the command ended without an exit status"),
            }
            if !stderr.is_empty() {
                message.push_str(&format!(" ({stderr})"));
            }
            message.push('.');
            message
        }
        RemoteFileError::Ssh(e) => format!("Could not {verb} {path}: {}.", e.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_tell_the_model_what_to_do() {
        let denied = describe(
            "/etc/shadow",
            Op::Read,
            RemoteFileError::PermissionDenied,
            false,
        );
        assert!(denied.contains("cannot read /etc/shadow"));
        assert!(denied.contains("send_input") && denied.contains("sudo"));
        let changed = describe("a", Op::Write, RemoteFileError::Changed, true);
        assert!(changed.contains("approval card") && changed.contains("nothing was written"));
        assert!(!describe("a", Op::Write, RemoteFileError::Changed, false).contains("card"));
        assert!(
            describe(
                "big.log",
                Op::Read,
                RemoteFileError::TooLarge(5_000_000),
                false
            )
            .contains("5000000 bytes")
        );
        assert_eq!(
            describe(
                "x",
                Op::Write,
                RemoteFileError::Failed {
                    status: Some(1),
                    stderr: "cat: write error: No space left on device".into()
                },
                false
            ),
            "Could not write x: the command exited with status 1 (cat: write error: No space \
             left on device)."
        );
    }

    #[test]
    fn writes_keep_the_style_of_the_file_they_replace() {
        let crlf = TextFile::decode(b"a\r\nb\r\n").unwrap();
        assert_eq!(
            new_content("p", Some(&crlf), "x\ny\n").unwrap().encode(),
            b"x\r\ny\r\n"
        );
        assert_eq!(
            new_content("p", None, "x\ny\n").unwrap().encode(),
            b"x\ny\n"
        );
        let big = "z".repeat(MAX_FILE_BYTES as usize + 1);
        assert!(new_content("p", None, &big).unwrap_err().contains("1 MB"));
    }

    #[tokio::test]
    async fn previews_without_a_session_or_of_other_tools_explain_why() {
        let (card, base) = preview(
            None,
            tools::EDIT_FILE,
            r#"{"path":"/etc/hosts","old_string":"a","new_string":"b"}"#,
            None,
        )
        .await;
        assert_eq!(card.path, "/etc/hosts");
        assert_eq!(card.error.as_deref(), Some(DISCONNECTED));
        assert!(base.is_none());

        // A preview the card read before needs no session.
        let cached = Base {
            path: "/etc/hosts".into(),
            bytes: Some(b"127.0.0.1 a\r\n::1 a\r\n".to_vec()),
        };
        let (card, base) = preview(
            None,
            tools::EDIT_FILE,
            r#"{"path":"/etc/hosts","old_string":"::1 a\n","new_string":""}"#,
            Some(cached),
        )
        .await;
        assert_eq!(card.error, None);
        assert_eq!(card.before.as_deref(), Some("127.0.0.1 a\n::1 a\n"));
        assert_eq!(card.after.as_deref(), Some("127.0.0.1 a\n"));
        assert!(card.crlf);
        assert!(base.is_some());

        let new = Base {
            path: "/srv/new.txt".into(),
            bytes: None,
        };
        let (card, _) = preview(
            None,
            tools::WRITE_FILE,
            r#"{"path":"/srv/new.txt","content":"hello\n"}"#,
            Some(new.clone()),
        )
        .await;
        assert_eq!(
            (card.before, card.after.as_deref()),
            (None, Some("hello\n"))
        );
        let (card, _) = preview(
            None,
            tools::EDIT_FILE,
            r#"{"path":"/srv/new.txt","old_string":"a","new_string":"b"}"#,
            Some(new),
        )
        .await;
        assert!(card.error.unwrap().contains("Use write_file"));

        let (card, _) = preview(None, tools::READ_FILE, r#"{"path":"a"}"#, None).await;
        assert!(card.error.is_some());
        let (card, _) = preview(None, tools::EDIT_FILE, "{", None).await;
        assert!(
            card.error
                .unwrap()
                .starts_with("The arguments are not valid")
        );
    }
}
