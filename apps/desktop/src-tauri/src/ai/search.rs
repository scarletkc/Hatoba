//! The search tools on the tab's host (AI-48, AI-49): `glob` and `grep` over an exec channel of
//! the tab's connection. The patterns and the results are in `hatoba_ai::search`, the scripts in
//! `hatoba_ssh::remote_search`.

use std::future::Future;
use std::time::Duration;

use super::conn::Conn;
use hatoba_ai::entry::ToolStatus;
use hatoba_ai::search::{self, Found, MAX_OUTPUT_BYTES, Search};
use hatoba_ai::tools::{self, GlobArgs, GrepArgs, GrepOutput};
use hatoba_ssh::remote_search::{GrepMode, GrepQuery, RemoteSearchError, Scope, SearchOutput};
use tokio_util::sync::CancellationToken;

use super::files::{DISCONNECTED, command_failed};
use super::parse;

/// How long a search may take, here and on the host, which ends it should the channel stay open.
const TIMEOUT: Duration = Duration::from_secs(30);

/// A call of a search tool, with its arguments.
pub(super) enum SearchJob {
    Grep(GrepArgs),
    Glob(GlobArgs),
}

impl SearchJob {
    /// `Some` for a search tool's name: its arguments, or the message for arguments that do not
    /// parse.
    pub(super) fn parse(name: &str, arguments: &str) -> Option<Result<Self, String>> {
        Some(match name {
            tools::GREP => parse(arguments).map(Self::Grep),
            tools::GLOB => parse(arguments).map(Self::Glob),
            _ => return None,
        })
    }
}

/// Runs a search tool. A stop ends it at once: it changes nothing on the host, and the host ends
/// it within a second of its channel closing.
pub(super) async fn run(
    session: Option<Conn>,
    job: SearchJob,
    cancel: &CancellationToken,
) -> (ToolStatus, String) {
    let Some(session) = session else {
        return (ToolStatus::Error, DISCONNECTED.to_owned());
    };
    let result = tokio::select! {
        biased;
        () = cancel.cancelled() => return (ToolStatus::Cancelled, String::new()),
        result = search(&session, job) => result,
    };
    match result {
        Ok(text) => (ToolStatus::Ok, text),
        Err(_) if session.is_closed() => (ToolStatus::Error, DISCONNECTED.to_owned()),
        Err(message) => (ToolStatus::Error, message),
    }
}

async fn search(session: &Conn, mut job: SearchJob) -> Result<String, String> {
    // On Windows itself a glob takes `\` between directories, as its paths do (§7.5); Windows
    // names have no `*` or `?` to escape.
    if session.windows_paths() {
        match &mut job {
            SearchJob::Glob(args) => args.pattern = args.pattern.replace('\\', "/"),
            SearchJob::Grep(args) => {
                if let Some(glob) = &mut args.glob {
                    *glob = glob.replace('\\', "/");
                }
            }
        }
    }
    match job {
        SearchJob::Glob(args) => {
            let target = Search::glob(args.path.as_deref(), &args.pattern)?;
            let out = within(
                &target,
                tools::GLOB,
                None,
                session.glob(&scope(&target), MAX_OUTPUT_BYTES, TIMEOUT),
            )
            .await?;
            Ok(search::glob_result(&args.pattern, &target, &found(&out)))
        }
        SearchJob::Grep(args) => {
            let target = Search::new(args.path.as_deref(), args.glob.as_deref())?;
            let multiline = args.multiline.unwrap_or(false);
            let pattern =
                search::pattern(&args.pattern, args.ignore_case.unwrap_or(false), multiline)?;
            let mode = match args.output_mode.unwrap_or_default() {
                GrepOutput::FilesWithMatches => GrepMode::Files,
                GrepOutput::Count => GrepMode::Count,
                GrepOutput::Content => {
                    let (before, after) = search::context(&args);
                    GrepMode::Lines { before, after }
                }
            };
            let query = GrepQuery {
                scope: scope(&target),
                pattern: &pattern.rg,
                extended: pattern.extended.as_deref().ok(),
                ignore_case: pattern.ignore_case,
                multiline,
                mode,
            };
            let out = within(
                &target,
                tools::GREP,
                pattern.extended.as_ref().err().copied(),
                session.grep(&query, MAX_OUTPUT_BYTES, TIMEOUT),
            )
            .await?;
            Ok(search::grep_result(&args, &target, &found(&out)))
        }
    }
}

fn scope(target: &Search) -> Scope<'_> {
    Scope {
        path: &target.path,
        below: &target.below,
        max_depth: target.max_depth,
        names: &target.names,
    }
}

fn found(out: &SearchOutput) -> Found<'_> {
    Found {
        start: &out.start,
        stdout: &out.stdout,
        nul: out.nul,
        cut: out.cut,
        stderr: &out.stderr,
    }
}

/// The search's output within [`TIMEOUT`], or the message for the model. `rg_only` says what
/// only `rg` can run, for grep.
async fn within(
    target: &Search,
    tool: &str,
    rg_only: Option<&str>,
    search: impl Future<Output = Result<SearchOutput, RemoteSearchError>>,
) -> Result<SearchOutput, String> {
    match tokio::time::timeout(TIMEOUT, search).await {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(describe(target, tool, rg_only, e)),
        Err(_) => Err(format!(
            "The search did not finish within 30 seconds, so Hatoba closed its channel. Search a \
             smaller directory, or narrow it with {}.",
            if tool == tools::GREP {
                "glob"
            } else {
                "a pattern that names the directories to search"
            }
        )),
    }
}

/// The message for the model when a search did not run.
fn describe(
    target: &Search,
    tool: &str,
    rg_only: Option<&str>,
    error: RemoteSearchError,
) -> String {
    let path = target.shown();
    match error {
        RemoteSearchError::InvalidArgument => {
            "The path or the pattern has a line break, so nothing was searched.".to_owned()
        }
        RemoteSearchError::NotFound => format!("{path} does not exist."),
        RemoteSearchError::DanglingLink => {
            format!("{path} is a symbolic link to a file that does not exist.")
        }
        RemoteSearchError::NotDirectory(resolved) => format!(
            "{} is a file, not a directory, so glob found nothing in it. Use read_file to read \
             it, or grep to search it.",
            if resolved.is_empty() {
                &path
            } else {
                &resolved
            }
        ),
        RemoteSearchError::PermissionDenied => format!(
            "Permission denied: the user this tab is logged in as cannot read {path}. {tool} \
             never uses sudo. To go on, use send_input to run the search with sudo in the \
             terminal's shell, where the user can enter a password, or ask the user."
        ),
        RemoteSearchError::BadPattern { rg: true, message } => {
            format!("rg on the host refused the pattern, so nothing was searched: {message}.")
        }
        RemoteSearchError::BadPattern { rg: false, message } => format!(
            "grep -E on the host, which runs where there is no rg, refused the pattern as Hatoba \
             translated it, so nothing was searched: {message}. Rewrite the part the message \
             names more simply."
        ),
        RemoteSearchError::NeedsRg => format!(
            "The host has no rg, and grep -E cannot run {}, so nothing was searched. Search \
             again without that.",
            rg_only.unwrap_or("this search")
        ),
        RemoteSearchError::Failed { status, stderr } => {
            command_failed(&format!("search {path}"), status, &stderr)
        }
        RemoteSearchError::Ssh(e) => format!("Could not search {path}: {}.", e.message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jobs_parse_by_name() {
        assert!(SearchJob::parse(tools::READ_FILE, "{}").is_none());
        assert!(matches!(
            SearchJob::parse(tools::GREP, r#"{"pattern":"x","path":"/etc"}"#),
            Some(Ok(SearchJob::Grep(GrepArgs { .. })))
        ));
        assert!(matches!(
            SearchJob::parse(tools::GLOB, ""),
            Some(Err(message)) if message.starts_with("The arguments are not valid")
        ));
    }

    #[test]
    fn failures_tell_the_model_what_to_do() {
        let at = |path: &str, glob: &str| Search::new(Some(path), Some(glob)).unwrap();
        assert_eq!(
            describe(
                &at("/etc", "x/*"),
                tools::GLOB,
                None,
                RemoteSearchError::NotFound
            ),
            "/etc/x does not exist."
        );
        assert!(
            describe(
                &at("/root", ""),
                tools::GREP,
                None,
                RemoteSearchError::PermissionDenied
            )
            .contains("grep never uses sudo. To go on, use send_input")
        );
        let bad = describe(
            &at("~", ""),
            tools::GREP,
            None,
            RemoteSearchError::BadPattern {
                rg: false,
                message: "grep: Unmatched ( or \\(".into(),
            },
        );
        assert!(bad.contains("refused the pattern as Hatoba translated it, so nothing was"));
        let file = describe(
            &at("/etc/hosts", "conf.d/*"),
            tools::GLOB,
            None,
            RemoteSearchError::NotDirectory("/etc/hosts".into()),
        );
        assert!(file.starts_with("/etc/hosts is a file") && file.contains("read_file"));
        let multiline = search::pattern("a.*b", false, true).unwrap().extended.err();
        assert_eq!(
            describe(
                &at("~", ""),
                tools::GREP,
                multiline,
                RemoteSearchError::NeedsRg
            ),
            "The host has no rg, and grep -E cannot run a search across lines (multiline), so \
             nothing was searched. Search again without that."
        );
    }

    #[tokio::test]
    async fn searches_need_a_live_session() {
        let job = SearchJob::Glob(GlobArgs {
            pattern: "*".into(),
            path: None,
        });
        let (status, text) = run(None, job, &CancellationToken::new()).await;
        assert_eq!((status, text.as_str()), (ToolStatus::Error, DISCONNECTED));
    }
}
