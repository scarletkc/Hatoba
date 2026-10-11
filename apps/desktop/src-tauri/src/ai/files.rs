//! The file tools on the tab's host (AI-38…40), the preview the approval card shows for
//! `edit_file` and `write_file`, and the record of what the model has seen of each file (spec
//! §13.4, "File tools"). The text work is in `hatoba_ai::files`, the exec scripts in
//! `hatoba_ssh::remote_file`.

use std::collections::HashMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use hatoba_ai::entry::ToolStatus;
use hatoba_ai::files::{self, MAX_FILE_BYTES, TextFile};
use hatoba_ai::tools::{self, EditFileArgs, ReadFileArgs, WriteFileArgs};
use hatoba_ssh::SshSession;
use hatoba_ssh::remote_file::{self, Expect, RemoteFile, RemoteFileError};
use tokio_util::sync::CancellationToken;

use super::{guard, parse};
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

/// The server a session is connected to, as the record of what the model has seen names it.
fn server(session: &SshSession) -> String {
    format!(
        "{}@{}:{}",
        session.username(),
        session.host(),
        session.port()
    )
}

/// What the model has seen of each file, by conversation: the POSIX `cksum` checksum and size by
/// server and resolved path (§13.4, "What the model has seen"). In memory only, until the app
/// quits; compaction and edit and resend forget a conversation's.
#[derive(Default)]
pub(super) struct Seen(Mutex<HashMap<String, SeenFiles>>);

/// One conversation's record: POSIX `cksum` checksum and size by server and resolved path.
type SeenFiles = HashMap<(String, String), (u32, u64)>;

impl Seen {
    /// The model has seen `bytes` as the file at `path` on `server`.
    pub(super) fn record(&self, conversation_id: &str, server: &str, path: &str, bytes: &[u8]) {
        guard(&self.0)
            .entry(conversation_id.to_owned())
            .or_default()
            .insert(
                (server.to_owned(), path.to_owned()),
                remote_file::cksum(bytes),
            );
    }

    /// Forgets what the conversation has seen: its summary or the entries an edit deleted no
    /// longer hold the files it read.
    pub(super) fn forget(&self, conversation_id: &str) {
        guard(&self.0).remove(conversation_id);
    }

    /// The conversation has a record of some file.
    #[cfg(test)]
    pub(super) fn knows(&self, conversation_id: &str) -> bool {
        guard(&self.0).contains_key(conversation_id)
    }

    /// AI-39, AI-40: `tool` may change `file` on `server` only when the model saw it as it is.
    /// `given` is the path as the model wrote it.
    fn check(
        &self,
        conversation_id: &str,
        server: &str,
        file: &RemoteFile,
        tool: &str,
        given: &str,
    ) -> Result<(), String> {
        let seen = guard(&self.0)
            .get(conversation_id)
            .and_then(|files| files.get(&(server.to_owned(), file.path.clone())))
            .copied();
        match seen {
            Some(sum) if sum == remote_file::cksum(&file.bytes) => Ok(()),
            Some(_) => Err(format!(
                "{given} changed since you last read it, by the user or another program, so \
                 {tool} did not change it. Call read_file on it again, then call {tool} again \
                 with what it holds now."
            )),
            None => Err(format!(
                "{given} has not been read in this conversation, so {tool} did not change it. \
                 Call read_file on it, then call {tool} again."
            )),
        }
    }
}

/// Where a file tool records and checks what the model has seen: the conversation's record and
/// the server of the tab's session.
#[derive(Clone, Copy)]
pub(super) struct SeenBy<'a> {
    pub seen: &'a Seen,
    pub conversation_id: &'a str,
}

/// What the approval card read for a call: the file as read, or `None` when it did not exist,
/// and the server it was read from. The write checks the file against it, so the user approved
/// the change to this content.
#[derive(Clone, Debug)]
pub(super) struct Base {
    /// The path as the call gives it.
    pub path: String,
    pub server: String,
    pub file: Option<RemoteFile>,
}

/// Why a file tool or a preview ended without a result of its own.
pub(super) enum Stop {
    /// The turn stopped or the vault locked before a write began.
    Cancelled,
    /// The message for the model, or for the approval card.
    Failed(String),
}

impl From<String> for Stop {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

/// `work`, unless `cancel` fires first.
async fn unless_cancelled<T>(
    cancel: &CancellationToken,
    work: impl Future<Output = Result<T, String>>,
) -> Result<T, Stop> {
    tokio::select! {
        biased;
        () = cancel.cancelled() => Err(Stop::Cancelled),
        result = work => result.map_err(Stop::Failed),
    }
}

/// Runs a file tool. `base` is what the approval card read for the call, if it showed one. A stop
/// ends the call while it reads, but never cuts a write off once it has begun.
pub(super) async fn run(
    session: Option<SshSession>,
    job: FileJob,
    base: Option<Base>,
    seen: SeenBy<'_>,
    cancel: &CancellationToken,
) -> (ToolStatus, String) {
    let Some(session) = session else {
        return (ToolStatus::Error, DISCONNECTED.to_owned());
    };
    let result = match job {
        FileJob::Read(args) => unless_cancelled(cancel, read(&session, &args, seen)).await,
        FileJob::Edit(args) => edit(&session, &args, base, seen, cancel).await,
        FileJob::Write(args) => write(&session, &args, base, seen, cancel).await,
    };
    match result {
        Ok(text) => (ToolStatus::Ok, text),
        Err(Stop::Cancelled) => (ToolStatus::Cancelled, String::new()),
        Err(Stop::Failed(_)) if session.is_closed() => (ToolStatus::Error, DISCONNECTED.to_owned()),
        Err(Stop::Failed(message)) => (ToolStatus::Error, message),
    }
}

/// An `edit_file` or `write_file` call for its approval card, with its path, or the preview that
/// says why there is no diff to show.
pub(super) fn preview_job(name: &str, arguments: &str) -> Result<(FileJob, String), AiFilePreview> {
    match FileJob::parse(name, arguments) {
        Some(Ok(job @ (FileJob::Edit(_) | FileJob::Write(_)))) => {
            let path = job.path().to_owned();
            Ok((job, path))
        }
        Some(Err(message)) => Err(failed_preview("", message)),
        _ => Err(failed_preview("", format!("{name} shows no diff."))),
    }
}

/// Reads the file the approval card shows a change to.
pub(super) async fn read_base(
    session: Option<SshSession>,
    path: &str,
    cancel: &CancellationToken,
) -> Result<Base, Stop> {
    let Some(session) = session else {
        return Err(Stop::Failed(DISCONNECTED.to_owned()));
    };
    match unless_cancelled(cancel, read_bytes(&session, path)).await {
        Ok(file) => Ok(Base {
            path: path.to_owned(),
            server: server(&session),
            file,
        }),
        Err(Stop::Failed(_)) if session.is_closed() => Err(Stop::Failed(DISCONNECTED.to_owned())),
        Err(stop) => Err(stop),
    }
}

/// The change `job` makes to `base`, as the approval card shows it, or the refusal running it
/// would give, such as for a file the model has not read.
pub(super) fn preview_of(job: &FileJob, base: &Base, seen: SeenBy<'_>) -> AiFilePreview {
    let path = &base.path;
    let tool = match job {
        FileJob::Edit(_) => tools::EDIT_FILE,
        _ => tools::WRITE_FILE,
    };
    if let Some(file) = &base.file
        && let Err(message) = seen
            .seen
            .check(seen.conversation_id, &base.server, file, tool, path)
    {
        return failed_preview(path, message);
    }
    let before = match base
        .file
        .as_ref()
        .map(|f| decode(path, &f.bytes))
        .transpose()
    {
        Ok(before) => before,
        Err(message) => return failed_preview(path, message),
    };
    let after = match job {
        FileJob::Edit(args) => before
            .as_ref()
            .ok_or_else(|| missing_for_edit(path))
            .and_then(|file| files::apply_edit(path, file, args).map(|e| e.file)),
        FileJob::Write(args) => new_content(path, before.as_ref(), &args.content),
        FileJob::Read(_) => Err(format!("{} shows no diff.", tools::READ_FILE)),
    };
    match after {
        Ok(after) => AiFilePreview {
            path: path.clone(),
            before: before.as_ref().map(TextFile::display_text),
            after: Some(after.display_text()),
            error: None,
            crlf: before.as_ref().is_some_and(|f| f.crlf),
        },
        Err(message) => failed_preview(path, message),
    }
}

pub(super) fn failed_preview(path: &str, message: String) -> AiFilePreview {
    AiFilePreview {
        path: path.to_owned(),
        before: None,
        after: None,
        error: Some(message),
        crlf: false,
    }
}

/// `read_file`: a page of the file. The model has now seen the whole file as it was read.
async fn read(
    session: &SshSession,
    args: &ReadFileArgs,
    seen: SeenBy<'_>,
) -> Result<String, String> {
    let file = read_bytes(session, &args.path)
        .await?
        .ok_or_else(|| format!("{} does not exist.", args.path))?;
    let page = files::read_page(
        &args.path,
        &decode(&args.path, &file.bytes)?,
        args.offset,
        args.limit,
    )?;
    seen.seen.record(
        seen.conversation_id,
        &server(session),
        &file.path,
        &file.bytes,
    );
    Ok(page)
}

async fn edit(
    session: &SshSession,
    args: &EditFileArgs,
    base: Option<Base>,
    seen: SeenBy<'_>,
    cancel: &CancellationToken,
) -> Result<String, Stop> {
    let server = server(session);
    let (file, from_card) =
        unless_cancelled(cancel, current(session, &server, &args.path, base)).await?;
    let file = file.ok_or_else(|| missing_for_edit(&args.path))?;
    seen.seen.check(
        seen.conversation_id,
        &server,
        &file,
        tools::EDIT_FILE,
        &args.path,
    )?;
    let edit = files::apply_edit(&args.path, &decode(&args.path, &file.bytes)?, args)?;
    let new = edit.file.encode();
    let expect = Expect::Content(&file.bytes);
    let path = write_bytes(session, &args.path, &new, expect, from_card, cancel).await?;
    seen.seen.record(seen.conversation_id, &server, &path, &new);
    Ok(files::edit_result(&args.path, &edit))
}

async fn write(
    session: &SshSession,
    args: &WriteFileArgs,
    base: Option<Base>,
    seen: SeenBy<'_>,
    cancel: &CancellationToken,
) -> Result<String, Stop> {
    let server = server(session);
    let (file, from_card) =
        unless_cancelled(cancel, current(session, &server, &args.path, base)).await?;
    // Replacing a file needs a record of it; creating one does not.
    if let Some(file) = &file {
        seen.seen.check(
            seen.conversation_id,
            &server,
            file,
            tools::WRITE_FILE,
            &args.path,
        )?;
    }
    let before = file
        .as_ref()
        .map(|f| decode(&args.path, &f.bytes))
        .transpose()?;
    let new = new_content(&args.path, before.as_ref(), &args.content)?;
    let bytes = new.encode();
    let expect = file
        .as_ref()
        .map_or(Expect::Missing, |f| Expect::Content(&f.bytes));
    let path = write_bytes(session, &args.path, &bytes, expect, from_card, cancel).await?;
    seen.seen
        .record(seen.conversation_id, &server, &path, &bytes);
    Ok(files::write_result(&args.path, &new, file.is_none()))
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

/// The file to change: what the approval card read for this path on this server, or the file as
/// it is now. The flag says it came from the card.
async fn current(
    session: &SshSession,
    server: &str,
    path: &str,
    base: Option<Base>,
) -> Result<(Option<RemoteFile>, bool), String> {
    match base.filter(|b| b.path == path && b.server == server) {
        Some(base) => Ok((base.file, true)),
        None => Ok((read_bytes(session, path).await?, false)),
    }
}

/// The file, `None` when it does not exist, or the message for the model.
async fn read_bytes(session: &SshSession, path: &str) -> Result<Option<RemoteFile>, String> {
    match tokio::time::timeout(TIMEOUT, remote_file::read(session, path, MAX_FILE_BYTES)).await {
        Err(_) => Err(timed_out(path, Op::Read)),
        Ok(Ok(file)) => Ok(Some(file)),
        Ok(Err(RemoteFileError::NotFound)) => Ok(None),
        Ok(Err(e)) => Err(describe(path, Op::Read, e, false)),
    }
}

/// Writes `bytes` if the file is as `expect` says, and returns the path as resolved. `from_card`:
/// `expect` is what the approval card read. A stop before the write prevents it; once it has
/// begun it runs to the end (or the timeout), because closing the channel midway leaves the file
/// cut short.
async fn write_bytes(
    session: &SshSession,
    path: &str,
    bytes: &[u8],
    expect: Expect<'_>,
    from_card: bool,
    cancel: &CancellationToken,
) -> Result<String, Stop> {
    if cancel.is_cancelled() {
        return Err(Stop::Cancelled);
    }
    match tokio::time::timeout(TIMEOUT, remote_file::write(session, path, bytes, expect)).await {
        Err(_) => Err(Stop::Failed(timed_out(path, Op::Write))),
        Ok(result) => result.map_err(|e| Stop::Failed(describe(path, Op::Write, e, from_card))),
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
        RemoteFileError::DanglingLink => format!(
            "{path} is a symbolic link to a file that does not exist, so the file tools do not \
             {verb} it: writing would create the link's target, which may be anywhere. Use the \
             path the link should point to, or ask the user."
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

    const SERVER: &str = "ops@db.example.net:22";

    /// What the card read for `path`, resolved as the scripts resolve it with `$HOME` at `/home/ops`.
    fn base(path: &str, bytes: Option<&[u8]>) -> Base {
        let resolved = match path.strip_prefix("~/") {
            Some(rest) => format!("/home/ops/{rest}"),
            None if path.starts_with('/') => path.to_owned(),
            None => format!("/home/ops/{path}"),
        };
        Base {
            path: path.into(),
            server: SERVER.into(),
            file: bytes.map(|b| RemoteFile {
                path: resolved,
                bytes: b.to_vec(),
            }),
        }
    }

    fn job(name: &str, arguments: &str) -> FileJob {
        preview_job(name, arguments).ok().unwrap().0
    }

    #[tokio::test]
    async fn previews_show_the_change_against_what_was_read() {
        // Without a session nothing is read, and the card says why.
        let cancel = CancellationToken::new();
        match read_base(None, "/etc/hosts", &cancel).await {
            Err(Stop::Failed(message)) => assert_eq!(message, DISCONNECTED),
            _ => panic!("expected the disconnected message"),
        }

        let seen = Seen::default();
        let by = SeenBy {
            seen: &seen,
            conversation_id: "c1",
        };
        let hosts = base("~/hosts", Some(b"127.0.0.1 a\r\n::1 a\r\n"));
        seen.record("c1", SERVER, "/home/ops/hosts", b"127.0.0.1 a\r\n::1 a\r\n");
        let edit = job(
            tools::EDIT_FILE,
            r#"{"path":"~/hosts","old_string":"::1 a\n","new_string":""}"#,
        );
        let card = preview_of(&edit, &hosts, by);
        assert_eq!(card.error, None);
        assert_eq!(card.before.as_deref(), Some("127.0.0.1 a\n::1 a\n"));
        assert_eq!(card.after.as_deref(), Some("127.0.0.1 a\n"));
        assert!(card.crlf);

        // A new file needs no record.
        let missing = base("/srv/new.txt", None);
        let write = job(
            tools::WRITE_FILE,
            r#"{"path":"/srv/new.txt","content":"hello\n"}"#,
        );
        let card = preview_of(&write, &missing, by);
        assert_eq!(
            (card.before, card.after.as_deref()),
            (None, Some("hello\n"))
        );
        let edit_missing = job(
            tools::EDIT_FILE,
            r#"{"path":"/srv/new.txt","old_string":"a","new_string":"b"}"#,
        );
        let error = preview_of(&edit_missing, &missing, by).error.unwrap();
        assert!(error.contains("Use write_file"), "{error}");
        let binary = base("/bin/ls", Some(b"\x7fELF\0"));
        seen.record("c1", SERVER, "/bin/ls", b"\x7fELF\0");
        let replace_ls = job(tools::WRITE_FILE, r#"{"path":"/bin/ls","content":"x"}"#);
        let error = preview_of(&replace_ls, &binary, by).error.unwrap();
        assert!(error.contains("cannot be read as text"), "{error}");

        // Other tools and broken arguments have no diff.
        let no_diff = |name, arguments| preview_job(name, arguments).err().unwrap().error.unwrap();
        assert!(no_diff(tools::READ_FILE, r#"{"path":"a"}"#).contains("shows no diff"));
        assert!(no_diff(tools::EDIT_FILE, "{").starts_with("The arguments are not valid"));
    }

    /// §13.4, "What the model has seen": a file is changed only as the model last saw it.
    #[test]
    fn files_change_only_as_the_model_last_saw_them() {
        let seen = Seen::default();
        let by = SeenBy {
            seen: &seen,
            conversation_id: "c1",
        };
        let conf = base("~/app.conf", Some(b"port = 80\n"));
        let edit = job(
            tools::EDIT_FILE,
            r#"{"path":"~/app.conf","old_string":"80","new_string":"8080"}"#,
        );
        let overwrite = job(
            tools::WRITE_FILE,
            r#"{"path":"~/app.conf","content":"port = 9090\n"}"#,
        );

        // Never read: both refused, on the card as in a run.
        for call in [&edit, &overwrite] {
            let error = preview_of(call, &conf, by).error.unwrap();
            assert!(
                error.contains("has not been read in this conversation"),
                "{error}"
            );
            assert!(error.contains("Call read_file on it"), "{error}");
        }

        // Read (in any conversation but this one, or on another server): still refused.
        seen.record("c2", SERVER, "/home/ops/app.conf", b"port = 80\n");
        seen.record("c1", "ops@other:22", "/home/ops/app.conf", b"port = 80\n");
        assert!(preview_of(&edit, &conf, by).error.is_some());

        // Read here: allowed.
        seen.record("c1", SERVER, "/home/ops/app.conf", b"port = 80\n");
        assert_eq!(preview_of(&edit, &conf, by).error, None);
        assert_eq!(preview_of(&overwrite, &conf, by).error, None);

        // Changed outside since (the user edited it in vim): refused until read again.
        let changed = base("~/app.conf", Some(b"port = 81\n"));
        let error = preview_of(&edit, &changed, by).error.unwrap();
        assert!(error.contains("changed since you last read it"), "{error}");
        assert!(preview_of(&overwrite, &changed, by).error.is_some());

        // Its own write is what it saw last, so it can edit again; the old content no longer passes.
        seen.record("c1", SERVER, "/home/ops/app.conf", b"port = 8080\n");
        let after_edit = base("~/app.conf", Some(b"port = 8080\n"));
        let again = job(
            tools::EDIT_FILE,
            r#"{"path":"~/app.conf","old_string":"8080","new_string":"8081"}"#,
        );
        assert_eq!(preview_of(&again, &after_edit, by).error, None);
        assert!(preview_of(&edit, &conf, by).error.is_some());

        // Compaction and edit and resend forget the conversation's records, and only its.
        seen.forget("c1");
        assert!(
            preview_of(&again, &after_edit, by)
                .error
                .unwrap()
                .contains("has not been read")
        );
        let other = SeenBy {
            seen: &seen,
            conversation_id: "c2",
        };
        assert_eq!(preview_of(&edit, &conf, other).error, None);
    }
}
