//! The file tools on the tab's host (AI-38…40), the preview the approval card shows for
//! `edit_file` and `write_file`, the record of what the model has seen of each file (spec §13.4,
//! "File tools"), and the revert of a change those calls made (AI-41). The text work is in
//! `hatoba_ai::files`, the exec scripts in `hatoba_ssh::remote_file`.

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

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::{guard, parse};
use crate::dto::{AiFilePreview, AiFileRevert, AiRevertFailure};

/// How long a read or a write may take.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A file tool's result while the tab has no live session (AI-08).
const DISCONNECTED: &str = "The terminal tab is disconnected, so the tool did not run. Ask the \
     user to reconnect the tab.";

/// The cancelled result of an `edit_file` or `write_file` call that ran when the turn stopped:
/// its write, if it had begun, was not cut off.
pub(super) const WRITE_STOPPED: &str = "If Hatoba had begun writing the file when the user \
     stopped, the write ran to its end, so the file may have changed. Read it with read_file \
     before you change it.";

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

/// The server a session is connected to, as the record of what the model has seen and the file
/// snapshots name it.
pub(super) fn server(session: &SshSession) -> String {
    format!(
        "{}@{}:{}",
        session.username(),
        session.host(),
        session.port()
    )
}

/// What the model has seen of each file, by conversation (§13.4, "What the model has seen"). In
/// memory only, until the app quits; compaction, edit and resend, and deleting the conversation
/// free a conversation's.
#[derive(Default)]
pub(super) struct Seen(Mutex<HashMap<String, SeenFiles>>);

/// One conversation's record, by server and resolved path.
type SeenFiles = HashMap<(String, String), Sight>;

/// A file as the model last saw it.
struct Sight {
    /// Its POSIX `cksum` checksum and size.
    sum: (u32, u64),
    /// The response whose call read or wrote it: the sight counts while that response is in the
    /// conversation's context.
    response: String,
}

/// Where a call of a file tool is in its conversation: the response that made it and the ids of
/// the entries in the conversation's context, in order.
#[derive(Default)]
pub(super) struct Place {
    pub response: String,
    pub context: Vec<String>,
}

/// What a call's result shows the model of a file. `tool_run` records it once it has stored the
/// result, so a call that a stop or the lock ends records nothing.
#[derive(Debug)]
pub(super) struct Shown {
    server: String,
    path: String,
    sum: (u32, u64),
}

impl Shown {
    fn new(server: &str, path: &str, bytes: &[u8]) -> Self {
        Self {
            server: server.to_owned(),
            path: path.to_owned(),
            sum: remote_file::cksum(bytes),
        }
    }
}

/// What a snapshot says about a change (AI-41): stored sealed as its `meta`, and listed.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ChangeMeta {
    /// The server the call wrote on, as [`server`] names it.
    pub server: String,
    /// The path as the write script resolved it.
    pub path: String,
    /// `edit_file` or `write_file`.
    pub tool: String,
    /// The call created the file.
    pub created: bool,
    /// The file's permission bits before the call, which Restore anyway gives it when it creates
    /// the file again; `None` when the call created it or they could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
}

/// The file's content around a change: stored sealed as the snapshot's `content`. Both are UTF-8,
/// as the file tools write text files only.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct ChangeContent {
    /// What the file held before the call, byte order mark included; `None` when it did not
    /// exist.
    pub before: Option<String>,
    /// What the call wrote.
    pub after: String,
}

impl std::fmt::Debug for ChangeContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChangeContent")
            .field("before_bytes", &self.before.as_ref().map(String::len))
            .field("after_bytes", &self.after.len())
            .finish()
    }
}

/// A change an `edit_file` or `write_file` call makes: what its snapshot keeps.
#[derive(Clone, Debug)]
pub(super) struct Change {
    pub meta: ChangeMeta,
    pub content: ChangeContent,
}

/// Where the snapshot of a change goes (AI-41): kept before the write begins, so a write that
/// fails midway can be reverted too, and marked once the write ended.
pub(super) trait Keep: Sync {
    /// Keeps `change` before its write begins. `Err` when the vault locked: the write does not
    /// begin.
    fn keep(&self, change: &Change) -> Result<(), Stop>;
    /// The write ended: it wrote the file at the path it resolved, or it never began (`None`). A
    /// write that failed after it may have begun leaves the snapshot unfinished.
    fn ended(&self, resolved: Option<&str>);
}

impl Change {
    pub(super) fn new(
        server: &str,
        path: &str,
        tool: &str,
        before: Option<&[u8]>,
        after: &[u8],
    ) -> Option<Self> {
        let text = |bytes: &[u8]| String::from_utf8(bytes.to_vec()).ok();
        Some(Self {
            meta: ChangeMeta {
                server: server.to_owned(),
                path: path.to_owned(),
                tool: tool.to_owned(),
                created: before.is_none(),
                mode: None,
            },
            content: ChangeContent {
                before: match before {
                    Some(bytes) => Some(text(bytes)?),
                    None => None,
                },
                after: text(after)?,
            },
        })
    }

    /// The change, with the permission bits the file had before it.
    pub(super) fn with_mode(mut self, mode: Option<u32>) -> Self {
        self.meta.mode = mode;
        self
    }

    /// The snapshot's `meta` and `content` as stored, sealed by the vault.
    pub fn encode(&self) -> Option<(String, Zeroizing<String>)> {
        Some((
            serde_json::to_string(&self.meta).ok()?,
            Zeroizing::new(serde_json::to_string(&self.content).ok()?),
        ))
    }
}

impl Seen {
    /// The model has seen `bytes` as the file at `path` on `server`, in a result of a call that
    /// `response` made.
    #[cfg(test)]
    pub(super) fn record(
        &self,
        conversation_id: &str,
        response: &str,
        server: &str,
        path: &str,
        bytes: &[u8],
    ) {
        self.insert(conversation_id, response, Shown::new(server, path, bytes));
    }

    /// The model has seen the file as `shown`, in a stored result of a call that `response` made.
    pub(super) fn insert(&self, conversation_id: &str, response: &str, shown: Shown) {
        guard(&self.0)
            .entry(conversation_id.to_owned())
            .or_default()
            .insert(
                (shown.server, shown.path),
                Sight {
                    sum: shown.sum,
                    response: response.to_owned(),
                },
            );
    }

    /// Forgets what the conversation has seen: its summary or the entries an edit deleted no
    /// longer hold the files it read.
    pub(super) fn forget(&self, conversation_id: &str) {
        guard(&self.0).remove(conversation_id);
    }

    /// Forgets what the conversation has seen of one file: a revert put back other content
    /// (AI-41), so the model reads it again before it changes it.
    pub(super) fn forget_file(&self, conversation_id: &str, server: &str, path: &str) {
        if let Some(files) = guard(&self.0).get_mut(conversation_id) {
            files.remove(&(server.to_owned(), path.to_owned()));
        }
    }

    /// The conversation has a record of some file.
    #[cfg(test)]
    pub(super) fn knows(&self, conversation_id: &str) -> bool {
        guard(&self.0).contains_key(conversation_id)
    }
}

/// Where a file tool checks what the model has seen, and notes what its result shows: the
/// conversation's record, where the call is in the conversation, and the note.
#[derive(Clone, Copy)]
pub(super) struct SeenBy<'a> {
    pub seen: &'a Seen,
    pub conversation_id: &'a str,
    pub place: &'a Place,
    /// What the call's result shows of a file, for `tool_run` to record once it stores it.
    pub shown: &'a Mutex<Option<Shown>>,
    /// Where the snapshot of what an `edit_file` or `write_file` call changes goes (AI-41).
    pub keep: Option<&'a dyn Keep>,
}

impl SeenBy<'_> {
    /// The call's result shows the model `bytes` as the file at `path` on `server`: it read the
    /// file, or wrote it.
    fn shows(&self, server: &str, path: &str, bytes: &[u8]) {
        *guard(self.shown) = Some(Shown::new(server, path, bytes));
    }

    /// AI-39, AI-40: `tool` may change `file` on `server` only when the model saw it as it is, in
    /// a result that is still in the conversation's context. `given` is the path as the model
    /// wrote it.
    fn check(
        &self,
        server: &str,
        file: &RemoteFile,
        tool: &str,
        given: &str,
    ) -> Result<(), String> {
        let seen = guard(&self.seen.0)
            .get(self.conversation_id)
            .and_then(|files| files.get(&(server.to_owned(), file.path.clone())))
            .filter(|sight| {
                self.place
                    .context
                    .binary_search_by(|id| id.as_str().cmp(&sight.response))
                    .is_ok()
            })
            .map(|sight| sight.sum);
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

/// The `edit_file` and `write_file` calls running now, by SSH session id. A write that has begun is
/// not cut off, so a lock that disconnects sessions closes a session once its calls have ended, at
/// most the 30 seconds of a write (§13.4, "File tools").
#[derive(Default)]
pub(super) struct Writes(tokio::sync::watch::Sender<HashMap<String, usize>>);

impl Writes {
    /// A call that may write on the session runs until the returned guard drops.
    pub(super) fn begin(&self, session_id: &str) -> Writing<'_> {
        self.0
            .send_modify(|runs| *runs.entry(session_id.to_owned()).or_default() += 1);
        Writing(self, session_id.to_owned())
    }

    /// Returns once no call that may write runs on the session.
    pub(super) async fn ended(&self, session_id: &str) {
        // The sender lives as long as `self`, so the wait cannot fail.
        let _ = self
            .0
            .subscribe()
            .wait_for(|runs| !runs.contains_key(session_id))
            .await;
    }
}

pub(super) struct Writing<'a>(&'a Writes, String);

impl Drop for Writing<'_> {
    fn drop(&mut self) {
        self.0.0.send_modify(|runs| {
            if let Some(n) = runs.get_mut(&self.1) {
                *n -= 1;
                if *n == 0 {
                    runs.remove(&self.1);
                }
            }
        });
    }
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
#[derive(Debug)]
pub(super) enum Stop {
    /// The turn stopped or the vault locked before a write began.
    Cancelled,
    /// The message for the model, or for the approval card.
    Failed(String),
    /// A write failed after it may have begun: the message says the file may be cut short, and
    /// stands when the connection is gone too.
    Unfinished(String),
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
        Err(Stop::Failed(message) | Stop::Unfinished(message)) => (ToolStatus::Error, message),
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
        && let Err(message) = seen.check(&base.server, file, tool, path)
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

/// `read_file`: a page of the file. The model has now seen the whole file as it was read. A file
/// over 1 MB is read a part at a time instead, and recorded as seen by no one: the other file
/// tools do not change it.
async fn read(
    session: &SshSession,
    args: &ReadFileArgs,
    seen: SeenBy<'_>,
) -> Result<String, String> {
    let file = match tokio::time::timeout(
        TIMEOUT,
        remote_file::read(session, &args.path, MAX_FILE_BYTES),
    )
    .await
    {
        Err(_) => return Err(timed_out(&args.path, Op::Read)),
        Ok(Ok(file)) => file,
        Ok(Err(RemoteFileError::TooLarge(_))) => return read_large(session, args).await,
        Ok(Err(RemoteFileError::NotFound)) => return Err(format!("{} does not exist.", args.path)),
        Ok(Err(e)) => return Err(describe(&args.path, Op::Read, e, false)),
    };
    let page = files::read_page(
        &args.path,
        &decode(&args.path, &file.bytes)?,
        args.offset,
        args.limit,
    )?;
    seen.shows(&server(session), &file.path, &file.bytes);
    Ok(page)
}

/// `read_file` on a file over 1 MB (AI-38): at most [`files::MAX_PAGE_BYTES`] of it from the line
/// asked for, whatever the file's size.
async fn read_large(session: &SshSession, args: &ReadFileArgs) -> Result<String, String> {
    let from = args.offset.unwrap_or(1).max(1);
    match tokio::time::timeout(
        TIMEOUT,
        remote_file::read_lines(session, &args.path, from, files::MAX_PAGE_BYTES),
    )
    .await
    {
        Err(_) => Err(timed_out(&args.path, Op::Read)),
        Ok(Ok(part)) => files::read_large_page(
            &args.path,
            from,
            part.lines,
            &part.bytes,
            part.cut,
            args.limit,
        ),
        Ok(Err(RemoteFileError::NotFound)) => Err(format!("{} does not exist.", args.path)),
        Ok(Err(e)) => Err(describe(&args.path, Op::Read, e, false)),
    }
}

async fn edit(
    session: &SshSession,
    args: &EditFileArgs,
    base: Option<Base>,
    seen: SeenBy<'_>,
    cancel: &CancellationToken,
) -> Result<String, Stop> {
    let server = server(session);
    let (file, from_card) = unless_cancelled(
        cancel,
        current(session, &server, &args.path, base, tools::EDIT_FILE),
    )
    .await?;
    let file = file.ok_or_else(|| missing_for_edit(&args.path))?;
    seen.check(&server, &file, tools::EDIT_FILE, &args.path)?;
    let edit = files::apply_edit(&args.path, &decode(&args.path, &file.bytes)?, args)?;
    let new = edit.file.encode();
    let expect = Expect::Content(&file.bytes);
    let change = Change::new(
        &server,
        &file.path,
        tools::EDIT_FILE,
        Some(&file.bytes),
        &new,
    )
    .map(|c| c.with_mode(file.mode));
    let keep = seen.keep.zip(change.as_ref());
    let path = write_bytes(session, &args.path, &new, expect, from_card, cancel, keep).await?;
    seen.shows(&server, &path, &new);
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
    let (file, from_card) = unless_cancelled(
        cancel,
        current(session, &server, &args.path, base, tools::WRITE_FILE),
    )
    .await?;
    // Replacing a file needs a record of it; creating one does not.
    if let Some(file) = &file {
        seen.check(&server, file, tools::WRITE_FILE, &args.path)?;
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
    // A file the call creates has its path resolved by the write.
    let change = Change::new(
        &server,
        file.as_ref()
            .map_or(args.path.as_str(), |f| f.path.as_str()),
        tools::WRITE_FILE,
        file.as_ref().map(|f| f.bytes.as_slice()),
        &bytes,
    )
    .map(|c| c.with_mode(file.as_ref().and_then(|f| f.mode)));
    let keep = seen.keep.zip(change.as_ref());
    let path = write_bytes(session, &args.path, &bytes, expect, from_card, cancel, keep).await?;
    seen.shows(&server, &path, &bytes);
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
    files::check_result(path, &file)?;
    Ok(file)
}

/// The file to change: what the approval card read, or without a card, the file as it is now.
/// The flag says it came from the card.
async fn current(
    session: &SshSession,
    server: &str,
    path: &str,
    base: Option<Base>,
    tool: &str,
) -> Result<(Option<RemoteFile>, bool), String> {
    match card_read(base, server, path, tool)? {
        Some(file) => Ok((file, true)),
        None => Ok((read_bytes(session, path).await?, false)),
    }
}

/// What the approval card read for the call (the file, or `None` when it did not exist), or
/// `None` when no card read one. A card that read another path, or the file on another server
/// than the tab is connected to now, showed the user a diff of something else, so the call
/// writes nothing.
fn card_read(
    base: Option<Base>,
    server: &str,
    path: &str,
    tool: &str,
) -> Result<Option<Option<RemoteFile>>, String> {
    match base {
        None => Ok(None),
        Some(base) if base.path == path && base.server == server => Ok(Some(base.file)),
        Some(base) => Err(format!(
            "{path} was not changed: the approval card showed the change to {} on {}, and the \
             tab is now connected to {server}. Call {tool} again so that the user reviews the \
             change on this server.",
            base.path, base.server
        )),
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
/// cut short. `keep` keeps the snapshot of the change before the write begins (AI-41).
async fn write_bytes(
    session: &SshSession,
    path: &str,
    bytes: &[u8],
    expect: Expect<'_>,
    from_card: bool,
    cancel: &CancellationToken,
    keep: Option<(&dyn Keep, &Change)>,
) -> Result<String, Stop> {
    if cancel.is_cancelled() {
        return Err(Stop::Cancelled);
    }
    if let Some((keep, change)) = keep {
        keep.keep(change)?;
        // A stop or the lock may have come while the snapshot was stored: the write never begins.
        if cancel.is_cancelled() {
            keep.ended(None);
            return Err(Stop::Cancelled);
        }
    }
    let result = match tokio::time::timeout(
        TIMEOUT,
        remote_file::write(session, path, bytes, expect, None),
    )
    .await
    {
        Err(_) => Err(Stop::Unfinished(timed_out(path, Op::Write))),
        Ok(Ok(resolved)) => Ok(resolved),
        // The command may have begun writing when it failed or the connection went.
        Ok(Err(e @ (RemoteFileError::Failed { .. } | RemoteFileError::Ssh(_)))) => {
            Err(Stop::Unfinished(unfinished(path, &e, session.is_closed())))
        }
        Ok(Err(e)) => Err(Stop::Failed(describe(path, Op::Write, e, from_card))),
    };
    if let Some((keep, _)) = keep {
        match &result {
            Ok(resolved) => keep.ended(Some(resolved)),
            Err(Stop::Unfinished(_)) => {}
            Err(_) => keep.ended(None),
        }
    }
    result
}

/// The message for a write that failed after it may have begun.
fn unfinished(path: &str, error: &RemoteFileError, closed: bool) -> String {
    let reconnect = if closed {
        " once the user has reconnected the tab"
    } else {
        ""
    };
    format!(
        "Writing {path} failed: {error}. The file may be unchanged, cut short or empty, so read \
         it with read_file{reconnect} before you change it."
    )
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
        // Only a deletion (AI-41's revert) refuses a link.
        RemoteFileError::SymbolicLink => format!("{path} is a symbolic link."),
        RemoteFileError::PermissionDenied => format!(
            "Permission denied: the user this tab is logged in as cannot {verb} {path}. The file \
             tools never use sudo. To go on, use send_input to run a command with sudo in the \
             terminal's shell, where the user can enter a password, or ask the user."
        ),
        // Only edit_file and write_file read a file whole; read_file reads a large one in parts.
        RemoteFileError::TooLarge(size) => format!(
            "{path} is {size} bytes, and edit_file and write_file change files up to 1 MB. Use \
             run_command or send_input to change it, for example with sed."
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

// ---- revert (AI-41) ----

fn revert_failed(reason: AiRevertFailure, detail: Option<String>) -> Revert {
    Revert::from(AiFileRevert::Failed { reason, detail })
}

/// What a revert found the file holding when it showed the diff: what Restore anyway writes over.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Found {
    Missing,
    /// Its POSIX `cksum` checksum and size.
    File((u32, u64)),
}

impl Found {
    fn of(current: Option<&[u8]>) -> Self {
        current.map_or(Self::Missing, |bytes| Self::File(remote_file::cksum(bytes)))
    }
}

/// What a revert did: the outcome, and for a diff, what the file held as it showed it.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Revert {
    pub outcome: AiFileRevert,
    pub found: Option<Found>,
}

impl From<AiFileRevert> for Revert {
    fn from(outcome: AiFileRevert) -> Self {
        Self {
            outcome,
            found: None,
        }
    }
}

/// What a revert does with the file it read.
#[derive(Debug, PartialEq, Eq)]
enum Plan {
    /// Writes back what the file held before the call, or deletes the file the call created, over
    /// what it holds now: what the call wrote, or with Restore anyway, what the diff showed.
    Restore,
    /// The file holds what it held before the call, or the file the call created is gone: there
    /// is nothing to write, as after a revert whose result was lost.
    Done,
    /// The file holds something else: an unfinished write or revert, or a change by the user,
    /// another program or a later call. The diff shows it.
    Show,
}

/// The plan for the file as read (`None`: it does not exist). `shown` is what the diff showed when
/// the user chose Restore anyway.
fn plan(change: &Change, current: Option<&[u8]>, shown: Option<Found>) -> Plan {
    let sum = remote_file::cksum;
    match (current, change.content.before.as_deref()) {
        (Some(bytes), _) if sum(bytes) == sum(change.content.after.as_bytes()) => Plan::Restore,
        (None, None) => Plan::Done,
        (Some(bytes), Some(before)) if sum(bytes) == sum(before.as_bytes()) => Plan::Done,
        _ if shown == Some(Found::of(current)) => Plan::Restore,
        _ => Plan::Show,
    }
}

/// The file at `path` as a revert finds it: its bytes, `None` when it does not exist, or why the
/// revert stops here.
async fn current_file(session: &SshSession, path: &str) -> Result<Option<Vec<u8>>, Revert> {
    match tokio::time::timeout(TIMEOUT, remote_file::read(session, path, MAX_FILE_BYTES)).await {
        Err(_) => Err(revert_failed(AiRevertFailure::TimedOut, None)),
        Ok(Ok(file)) => Ok(Some(file.bytes)),
        Ok(Err(RemoteFileError::NotFound)) => Ok(None),
        Ok(Err(RemoteFileError::PermissionDenied)) => {
            Err(revert_failed(AiRevertFailure::PermissionDenied, None))
        }
        // It changed into something the tools do not read, so there is no diff to show.
        Ok(Err(
            RemoteFileError::IsDirectory
            | RemoteFileError::NotRegular
            | RemoteFileError::DanglingLink
            | RemoteFileError::TooLarge(_),
        )) => Err(revert_failed(AiRevertFailure::NotText, None)),
        Ok(Err(_)) if session.is_closed() => {
            Err(revert_failed(AiRevertFailure::Disconnected, None))
        }
        Ok(Err(e)) => Err(revert_failed(AiRevertFailure::Failed, Some(e.to_string()))),
    }
}

/// The text the panel diffs: CRLF line breaks as `\n`, like the approval card's.
fn shown_text(bytes: &[u8]) -> Option<String> {
    TextFile::decode(bytes).ok().map(|f| f.display_text())
}

/// The change as the panel diffs it, from what the file held before the call (`None`: the call
/// created it) to what the call wrote.
pub(super) fn change_preview(change: &Change) -> AiFilePreview {
    let before = change
        .content
        .before
        .as_deref()
        .map(|before| TextFile::decode(before.as_bytes()));
    AiFilePreview {
        path: change.meta.path.clone(),
        before: before.as_ref().map(|file| {
            file.as_ref()
                .map(TextFile::display_text)
                .unwrap_or_default()
        }),
        after: Some(shown_text(change.content.after.as_bytes()).unwrap_or_default()),
        error: None,
        crlf: before.is_some_and(|file| file.is_ok_and(|f| f.crlf)),
    }
}

/// The diff from what the file holds now to what it held before the call, which Restore anyway
/// writes, and what the file held as the diff shows it.
fn show(change: &Change, current: Option<&[u8]>) -> Revert {
    let text = match current.map(shown_text) {
        Some(None) => return revert_failed(AiRevertFailure::NotText, None),
        text => text.flatten(),
    };
    Revert {
        outcome: AiFileRevert::Changed {
            path: change.meta.path.clone(),
            current: text,
            before: change
                .content
                .before
                .as_deref()
                .map(|before| shown_text(before.as_bytes()).unwrap_or_default()),
            later: false,
        },
        found: Some(Found::of(current)),
    }
}

/// Reads the file again after a write or deletion found it changed since the read: done when it
/// holds what it held before the call, otherwise the diff.
async fn found_again(session: &SshSession, change: &Change) -> Revert {
    match current_file(session, &change.meta.path).await {
        Ok(current) if plan(change, current.as_deref(), None) == Plan::Done => {
            AiFileRevert::Reverted { wrote: false }.into()
        }
        Ok(current) => show(change, current.as_deref()),
        Err(revert) => revert,
    }
}

/// The permission bits a file Restore anyway creates again gets when its snapshot has none (it
/// was taken before snapshots kept them, or `ls` showed none): readable by its owner only.
const RECREATED_MODE: u32 = 0o600;

/// Writes back what the file held before the call, or deletes the file the call created, only
/// while it holds exactly `current`, by `cksum` checksum and size. A file deleted since is created
/// again with the permission bits it had, owned by the user the tab is logged in as.
async fn restore(session: &SshSession, change: &Change, current: Option<&[u8]>) -> Revert {
    let path = &change.meta.path;
    let ended = match &change.content.before {
        None => {
            let Some(current) = current else {
                return AiFileRevert::Reverted { wrote: false }.into();
            };
            let delete = remote_file::delete(session, path, current);
            deleted(tokio::time::timeout(TIMEOUT, delete).await.ok())
        }
        Some(before) => {
            let expect = current.map_or(Expect::Missing, Expect::Content);
            let mode = current
                .is_none()
                .then(|| change.meta.mode.unwrap_or(RECREATED_MODE));
            let write = remote_file::write(session, path, before.as_bytes(), expect, mode);
            written(tokio::time::timeout(TIMEOUT, write).await.ok())
        }
    };
    match ended {
        Some(revert) => revert,
        None => found_again(session, change).await,
    }
}

/// What the end of a revert's deletion means (`None`: it timed out), or `None` when the file
/// changed since it was read, so it is read again.
fn deleted(result: Option<Result<String, RemoteFileError>>) -> Option<Revert> {
    Some(match result {
        Some(Ok(_)) => AiFileRevert::Reverted { wrote: true }.into(),
        Some(Err(RemoteFileError::Changed | RemoteFileError::NotFound)) => return None,
        // Neither the link nor its target is deleted. Reading through the link again would show
        // its target, which may hold what the call wrote, so there is nothing to show.
        Some(Err(RemoteFileError::SymbolicLink)) => {
            revert_failed(AiRevertFailure::SymbolicLink, None)
        }
        Some(Err(RemoteFileError::PermissionDenied)) => {
            revert_failed(AiRevertFailure::PermissionDenied, None)
        }
        Some(Err(RemoteFileError::IsDirectory | RemoteFileError::NotRegular)) => {
            revert_failed(AiRevertFailure::NotText, None)
        }
        // `rm` may have run before the time ran out, the command failed or the connection went.
        None => revert_failed(AiRevertFailure::Unfinished, None),
        Some(Err(e @ (RemoteFileError::Failed { .. } | RemoteFileError::Ssh(_)))) => {
            revert_failed(AiRevertFailure::Unfinished, Some(e.to_string()))
        }
        Some(Err(e)) => revert_failed(AiRevertFailure::Failed, Some(e.to_string())),
    })
}

/// What the end of a revert's write means (`None`: it timed out), or `None` when the file changed
/// since it was read, so it is read again.
fn written(result: Option<Result<String, RemoteFileError>>) -> Option<Revert> {
    Some(match result {
        Some(Ok(_)) => AiFileRevert::Reverted { wrote: true }.into(),
        Some(Err(RemoteFileError::Changed)) => return None,
        Some(Err(RemoteFileError::PermissionDenied)) => {
            revert_failed(AiRevertFailure::PermissionDenied, None)
        }
        Some(Err(
            RemoteFileError::IsDirectory
            | RemoteFileError::NotRegular
            | RemoteFileError::DanglingLink,
        )) => revert_failed(AiRevertFailure::NotText, None),
        // The write may have begun before the time ran out, the command failed or the connection
        // went.
        None => revert_failed(AiRevertFailure::Unfinished, None),
        Some(Err(e @ (RemoteFileError::Failed { .. } | RemoteFileError::Ssh(_)))) => {
            revert_failed(AiRevertFailure::Unfinished, Some(e.to_string()))
        }
        Some(Err(e)) => revert_failed(AiRevertFailure::Failed, Some(e.to_string())),
    })
}

/// AI-41: puts back what the file held before `change`, or deletes the file the change created.
/// It writes only over what the call wrote, or with Restore anyway (`shown`), over what the diff
/// showed, by `cksum` checksum and size. A file that holds what it held before the call already is
/// left as it is and counts as reverted. `None` when `cancel` (the lock) fired before anything was
/// written; once a write or deletion has begun, it runs to its end.
pub(super) async fn revert(
    session: Option<SshSession>,
    change: &Change,
    shown: Option<Found>,
    cancel: &CancellationToken,
) -> Option<Revert> {
    let Some(session) = session else {
        return Some(revert_failed(AiRevertFailure::Disconnected, None));
    };
    if server(&session) != change.meta.server {
        return Some(revert_failed(
            AiRevertFailure::OtherServer,
            Some(change.meta.server.clone()),
        ));
    }
    let current = tokio::select! {
        biased;
        () = cancel.cancelled() => return None,
        current = current_file(&session, &change.meta.path) => current,
    };
    let current = match current {
        Ok(current) => current,
        Err(revert) => return Some(revert),
    };
    match plan(change, current.as_deref(), shown) {
        Plan::Done => Some(AiFileRevert::Reverted { wrote: false }.into()),
        Plan::Show => Some(show(change, current.as_deref())),
        Plan::Restore if cancel.is_cancelled() => None,
        Plan::Restore => Some(restore(&session, change, current.as_deref()).await),
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
        let full = RemoteFileError::Failed {
            status: Some(1),
            stderr: "cat: write error: No space left on device".into(),
        };
        assert_eq!(
            unfinished("x", &full, false),
            "Writing x failed: the command exited with status 1: cat: write error: No space left \
             on device. The file may be unchanged, cut short or empty, so read it with read_file \
             before you change it."
        );
        let gone = RemoteFileError::Ssh(hatoba_ssh::SshError::new(
            hatoba_ssh::SshErrorKind::Disconnected,
            "the connection closed",
        ));
        assert!(unfinished("x", &gone, true).contains("once the user has reconnected the tab"));
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
                mode: Some(0o644),
            }),
        }
    }

    fn job(name: &str, arguments: &str) -> FileJob {
        preview_job(name, arguments).ok().unwrap().0
    }

    /// A call of response `e3`, with `e1` to `e3` in the conversation's context: a summary
    /// replaced what came before `e1`.
    fn place() -> Place {
        Place {
            response: "e3".into(),
            context: vec!["e1".into(), "e2".into(), "e3".into()],
        }
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
        let place = place();
        let by = SeenBy {
            seen: &seen,
            conversation_id: "c1",
            place: &place,
            shown: &Mutex::default(),
            keep: None,
        };
        let hosts = base("~/hosts", Some(b"127.0.0.1 a\r\n::1 a\r\n"));
        seen.record(
            "c1",
            "e2",
            SERVER,
            "/home/ops/hosts",
            b"127.0.0.1 a\r\n::1 a\r\n",
        );
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
        seen.record("c1", "e2", SERVER, "/bin/ls", b"\x7fELF\0");
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
        let place = place();
        let by = SeenBy {
            seen: &seen,
            conversation_id: "c1",
            place: &place,
            shown: &Mutex::default(),
            keep: None,
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
        seen.record("c2", "e2", SERVER, "/home/ops/app.conf", b"port = 80\n");
        seen.record(
            "c1",
            "e2",
            "ops@other:22",
            "/home/ops/app.conf",
            b"port = 80\n",
        );
        assert!(preview_of(&edit, &conf, by).error.is_some());
        // Read in a response the context no longer holds (a compaction or an edit, maybe on
        // another device): refused too.
        seen.record("c1", "e0", SERVER, "/home/ops/app.conf", b"port = 80\n");
        assert!(
            preview_of(&edit, &conf, by)
                .error
                .unwrap()
                .contains("has not been read")
        );

        // Read here: allowed.
        seen.record("c1", "e2", SERVER, "/home/ops/app.conf", b"port = 80\n");
        assert_eq!(preview_of(&edit, &conf, by).error, None);
        assert_eq!(preview_of(&overwrite, &conf, by).error, None);

        // Changed outside since (the user edited it in vim): refused until read again.
        let changed = base("~/app.conf", Some(b"port = 81\n"));
        let error = preview_of(&edit, &changed, by).error.unwrap();
        assert!(error.contains("changed since you last read it"), "{error}");
        assert!(preview_of(&overwrite, &changed, by).error.is_some());

        // Its own write is what it saw last, so it can edit again; the old content no longer passes.
        // A call only notes what its result shows: it counts once `tool_run` stores the result.
        let shown = Mutex::default();
        let wrote = SeenBy {
            shown: &shown,
            ..by
        };
        wrote.shows(SERVER, "/home/ops/app.conf", b"port = 8080\n");
        let after_edit = base("~/app.conf", Some(b"port = 8080\n"));
        let again = job(
            tools::EDIT_FILE,
            r#"{"path":"~/app.conf","old_string":"8080","new_string":"8081"}"#,
        );
        assert!(preview_of(&again, &after_edit, by).error.is_some());
        seen.insert("c1", "e3", guard(&shown).take().unwrap());
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
            place: &place,
            shown: &Mutex::default(),
            keep: None,
        };
        assert_eq!(preview_of(&edit, &conf, other).error, None);
    }

    /// AI-41: what a write keeps for its revert, before the write begins.
    #[tokio::test]
    async fn changes_are_kept_with_what_the_file_held_before() {
        let change = Change::new(
            SERVER,
            "/etc/app.conf",
            tools::EDIT_FILE,
            Some(b"a\r\n"),
            b"b\r\n",
        )
        .unwrap();
        assert_eq!(
            change.meta,
            ChangeMeta {
                server: SERVER.into(),
                path: "/etc/app.conf".into(),
                tool: tools::EDIT_FILE.into(),
                created: false,
                mode: None,
            }
        );
        assert_eq!(change.content.before.as_deref(), Some("a\r\n"));
        let (meta, content) = change.encode().unwrap();
        assert!(meta.contains("\"created\":false"), "{meta}");
        assert!(!meta.contains("mode"), "{meta}");
        // The permission bits the file had go with it, for Restore anyway to create it again.
        let (with_mode, _) = change.clone().with_mode(Some(0o640)).encode().unwrap();
        assert!(with_mode.contains("\"mode\":416"), "{with_mode}");
        assert_eq!(
            serde_json::from_str::<ChangeMeta>(&with_mode).unwrap().mode,
            Some(0o640)
        );
        assert_eq!(
            serde_json::from_str::<ChangeMeta>(&meta).unwrap().mode,
            None
        );
        assert!(
            content.contains("\"after\":\"b\\r\\n\""),
            "{}",
            content.as_str()
        );
        let created = Change::new(SERVER, "new", tools::WRITE_FILE, None, b"x").unwrap();
        assert!(created.meta.created && created.content.before.is_none());

        // Without a live session nothing is read or written.
        let cancel = CancellationToken::new();
        assert_eq!(
            revert(None, &change, None, &cancel).await,
            Some(Revert::from(AiFileRevert::Failed {
                reason: AiRevertFailure::Disconnected,
                detail: None
            }))
        );
    }

    /// AI-41: how the end of a revert's write or deletion reads. One that may have begun is
    /// unfinished, a timeout included; a symbolic link in a created file's place stops the revert
    /// rather than showing its target again.
    #[test]
    fn reverts_tell_what_their_write_or_deletion_did() {
        let failed = |reason| {
            Some(Revert::from(AiFileRevert::Failed {
                reason,
                detail: None,
            }))
        };
        let done = Some(Revert::from(AiFileRevert::Reverted { wrote: true }));
        assert_eq!(deleted(Some(Ok("/srv/new".into()))), done);
        assert_eq!(written(Some(Ok("/etc/app.conf".into()))), done);
        assert_eq!(deleted(None), failed(AiRevertFailure::Unfinished));
        assert_eq!(written(None), failed(AiRevertFailure::Unfinished));
        let gone = || {
            RemoteFileError::Ssh(hatoba_ssh::SshError::new(
                hatoba_ssh::SshErrorKind::Disconnected,
                "the connection closed",
            ))
        };
        for result in [deleted(Some(Err(gone()))), written(Some(Err(gone())))] {
            assert!(matches!(
                result,
                Some(Revert {
                    outcome: AiFileRevert::Failed {
                        reason: AiRevertFailure::Unfinished,
                        detail: Some(_),
                    },
                    found: None,
                })
            ));
        }
        // Changed since the read: read again.
        assert_eq!(deleted(Some(Err(RemoteFileError::Changed))), None);
        assert_eq!(deleted(Some(Err(RemoteFileError::NotFound))), None);
        assert_eq!(written(Some(Err(RemoteFileError::Changed))), None);
        assert_eq!(
            deleted(Some(Err(RemoteFileError::SymbolicLink))),
            failed(AiRevertFailure::SymbolicLink)
        );
        assert_eq!(
            deleted(Some(Err(RemoteFileError::PermissionDenied))),
            failed(AiRevertFailure::PermissionDenied)
        );
    }

    /// AI-41: what a revert does with the file it reads. It writes only over what the call wrote,
    /// or with Restore anyway over what its diff showed; a file that holds what it held before
    /// counts as reverted, so a revert whose result was lost can be done again.
    #[test]
    fn reverts_write_only_over_what_they_can_tell() {
        let edit = Change::new(
            SERVER,
            "/etc/app.conf",
            tools::EDIT_FILE,
            Some(b"a\n"),
            b"b\n",
        )
        .unwrap();
        let created = Change::new(SERVER, "/srv/new", tools::WRITE_FILE, None, b"x\n").unwrap();

        // (a) The file holds what the call wrote: written back, or deleted.
        assert_eq!(plan(&edit, Some(b"b\n"), None), Plan::Restore);
        assert_eq!(plan(&created, Some(b"x\n"), None), Plan::Restore);
        // (b) It holds what it held before, or the file the call created is gone: nothing to
        // write. So a revert whose result was lost is done when tried again, and so is an
        // unfinished write that never changed the file.
        assert_eq!(plan(&edit, Some(b"a\n"), None), Plan::Done);
        assert_eq!(plan(&created, None, None), Plan::Done);
        // (c) Anything else is shown: changed since, a write cut short, a missing file the call
        // did not create.
        for current in [Some(&b"c\n"[..]), Some(&b"b"[..]), None] {
            assert_eq!(plan(&edit, current, None), Plan::Show, "{current:?}");
        }
        assert_eq!(plan(&created, Some(b"y\n"), None), Plan::Show);

        // The diff goes from the file now to what it held before, which Restore anyway writes.
        let shown = show(&edit, Some(b"b"));
        assert_eq!(
            shown,
            Revert {
                outcome: AiFileRevert::Changed {
                    path: "/etc/app.conf".into(),
                    current: Some("b".into()),
                    before: Some("a\n".into()),
                    later: false,
                },
                found: Some(Found::File(remote_file::cksum(b"b"))),
            }
        );
        // Restore anyway writes over exactly that, and over nothing else: a file that changed
        // again since the diff is shown again.
        assert_eq!(plan(&edit, Some(b"b"), shown.found), Plan::Restore);
        assert_eq!(plan(&edit, Some(b"bb"), shown.found), Plan::Show);
        assert_eq!(plan(&edit, None, shown.found), Plan::Show);
        // A missing file is put back, a changed file the call created deleted, each only as shown.
        let gone = show(&edit, None);
        assert!(matches!(
            gone.outcome,
            AiFileRevert::Changed {
                current: None,
                before: Some(_),
                ..
            }
        ));
        assert_eq!(plan(&edit, None, gone.found), Plan::Restore);
        assert_eq!(plan(&edit, Some(b"c\n"), gone.found), Plan::Show);
        let changed = show(&created, Some(b"y\n"));
        assert!(matches!(
            changed.outcome,
            AiFileRevert::Changed { before: None, .. }
        ));
        assert_eq!(plan(&created, Some(b"y\n"), changed.found), Plan::Restore);

        // A revert whose own write was cut short leaves the file neither way: shown, and then
        // restored anyway over what it holds; tried again after that, it is done.
        let cut = show(&edit, Some(b""));
        assert_eq!(plan(&edit, Some(b""), cut.found), Plan::Restore);
        assert_eq!(plan(&edit, Some(b"a\n"), cut.found), Plan::Done);

        // Line breaks as the card shows them; nothing to diff for a file that is not text.
        let crlf = Change::new(
            SERVER,
            "/etc/c.conf",
            tools::EDIT_FILE,
            Some(b"a\r\n"),
            b"b\r\n",
        )
        .unwrap();
        assert!(matches!(
            show(&crlf, Some(b"c\r\n")).outcome,
            AiFileRevert::Changed { current: Some(ref c), before: Some(ref b), .. } if c == "c\n" && b == "a\n"
        ));
        assert_eq!(
            show(&edit, Some(b"\0bin")),
            Revert::from(AiFileRevert::Failed {
                reason: AiRevertFailure::NotText,
                detail: None
            })
        );
    }

    /// AI-41: a revert ends the model's record of that file only.
    #[test]
    fn a_revert_forgets_what_the_model_saw_of_the_file() {
        let seen = Seen::default();
        let place = place();
        let by = SeenBy {
            seen: &seen,
            conversation_id: "c1",
            place: &place,
            shown: &Mutex::default(),
            keep: None,
        };
        seen.record("c1", "e2", SERVER, "/etc/app.conf", b"b\r\n");
        seen.record("c1", "e2", SERVER, "/etc/other.conf", b"o");
        seen.forget_file("c1", SERVER, "/etc/app.conf");
        let conf = base("/etc/app.conf", Some(b"b\r\n"));
        let edit = job(
            tools::EDIT_FILE,
            r#"{"path":"/etc/app.conf","old_string":"b","new_string":"c"}"#,
        );
        assert!(
            preview_of(&edit, &conf, by)
                .error
                .unwrap()
                .contains("has not been read")
        );
        let other = base("/etc/other.conf", Some(b"o"));
        let edit_other = job(
            tools::EDIT_FILE,
            r#"{"path":"/etc/other.conf","old_string":"o","new_string":"p"}"#,
        );
        assert_eq!(preview_of(&edit_other, &other, by).error, None);
    }

    /// A lock that disconnects sessions waits for the edits and writes that run on each.
    #[tokio::test]
    async fn writes_are_waited_for_by_session() {
        let writes = Writes::default();
        writes.ended("s1").await;
        let first = writes.begin("s1");
        let second = writes.begin("s1");
        let other = writes.begin("s2");
        let wait = |id| tokio::time::timeout(Duration::from_millis(50), writes.ended(id));
        assert!(wait("s1").await.is_err());
        assert!(wait("s3").await.is_ok());
        drop(first);
        assert!(wait("s1").await.is_err());
        drop(second);
        assert!(wait("s1").await.is_ok());
        assert!(wait("s2").await.is_err());
        drop(other);
        assert!(wait("s2").await.is_ok());
    }

    /// A run writes over what its approval card read, and only on the server the card read it on.
    #[test]
    fn runs_write_over_what_the_card_showed() {
        let conf = || Some(base("/etc/app.conf", Some(b"x\n")));
        let read = card_read(conf(), SERVER, "/etc/app.conf", tools::EDIT_FILE).unwrap();
        assert_eq!(read.unwrap().unwrap().bytes, b"x\n");
        let missing = Some(base("/srv/new.txt", None));
        assert_eq!(
            card_read(missing, SERVER, "/srv/new.txt", tools::WRITE_FILE).unwrap(),
            Some(None)
        );
        // Without a card, the run reads the file itself.
        assert_eq!(
            card_read(None, SERVER, "/etc/app.conf", tools::EDIT_FILE).unwrap(),
            None
        );
        // The tab reconnected to another server since the card read the file.
        let error = card_read(conf(), "ops@db2:22", "/etc/app.conf", tools::EDIT_FILE).unwrap_err();
        assert!(error.contains("now connected to ops@db2:22"), "{error}");
        assert!(error.contains("Call edit_file again"), "{error}");
        assert!(card_read(conf(), SERVER, "/etc/other.conf", tools::EDIT_FILE).is_err());
    }
}
