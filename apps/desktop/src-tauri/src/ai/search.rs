//! The search tools on the tab's host (AI-48, AI-49): `glob` and `grep` over an exec channel of
//! the tab's connection. The patterns and the results are in `hatoba_ai::search`, the scripts in
//! `hatoba_ssh::remote_search`.

use std::future::Future;
use std::time::Duration;

use hatoba_ai::entry::ToolStatus;
use hatoba_ai::search::{self, Found, MAX_CONTEXT, MAX_OUTPUT_BYTES, Search};
use hatoba_ai::tools::{self, GlobArgs, GrepArgs, GrepOutput};
use hatoba_ssh::SshSession;
use hatoba_ssh::remote_search::{self, GrepMode, GrepQuery, RemoteSearchError, SearchOutput};
use tokio_util::sync::CancellationToken;

use super::files::{DISCONNECTED, command_failed};
use super::parse;

/// How long a search may take.
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

/// Runs a search tool. A stop ends it at once: it changes nothing on the host.
pub(super) async fn run(
    session: Option<SshSession>,
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

async fn search(session: &SshSession, job: SearchJob) -> Result<String, String> {
    match job {
        SearchJob::Glob(args) => {
            let target = Search::glob(args.path.as_deref(), &args.pattern)?;
            let out = within(
                &target.path,
                tools::GLOB,
                remote_search::glob(
                    session,
                    &target.path,
                    target.max_depth,
                    &target.names,
                    MAX_OUTPUT_BYTES,
                ),
            )
            .await?;
            Ok(search::glob_result(&args.pattern, &target, &found(&out)))
        }
        SearchJob::Grep(args) => {
            let target = Search::new(args.path.as_deref(), args.glob.as_deref())?;
            let pattern = search::extended_regex(&args.pattern)?;
            let mode = match args.output_mode.unwrap_or_default() {
                GrepOutput::FilesWithMatches => GrepMode::Files,
                GrepOutput::Count => GrepMode::Count,
                GrepOutput::Content => GrepMode::Lines {
                    context: u32::try_from(args.context.unwrap_or(0).min(MAX_CONTEXT))
                        .unwrap_or_default(),
                },
            };
            let query = GrepQuery {
                path: &target.path,
                max_depth: target.max_depth,
                names: &target.names,
                pattern: &pattern,
                ignore_case: args.ignore_case.unwrap_or(false),
                mode,
            };
            let out = within(
                &target.path,
                tools::GREP,
                remote_search::grep(session, &query, MAX_OUTPUT_BYTES),
            )
            .await?;
            Ok(search::grep_result(&args, &target, &found(&out)))
        }
    }
}

fn found(out: &SearchOutput) -> Found<'_> {
    Found {
        start: &out.start,
        stdout: &out.stdout,
        cut: out.cut,
        stderr: &out.stderr,
    }
}

/// The search's output within [`TIMEOUT`], or the message for the model.
async fn within(
    path: &str,
    tool: &str,
    search: impl Future<Output = Result<SearchOutput, RemoteSearchError>>,
) -> Result<SearchOutput, String> {
    match tokio::time::timeout(TIMEOUT, search).await {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(describe(path, tool, e)),
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
fn describe(path: &str, tool: &str, error: RemoteSearchError) -> String {
    match error {
        RemoteSearchError::InvalidArgument => {
            "The path or the pattern has a line break, so nothing was searched.".to_owned()
        }
        RemoteSearchError::NotFound => format!("{path} does not exist."),
        RemoteSearchError::DanglingLink => {
            format!("{path} is a symbolic link to a file that does not exist.")
        }
        RemoteSearchError::NotDirectory => format!(
            "{path} is a file, not a directory, so glob found nothing in it. Use read_file to \
             read it, or grep to search it."
        ),
        RemoteSearchError::PermissionDenied => format!(
            "Permission denied: the user this tab is logged in as cannot read {path}. {tool} \
             never uses sudo. To go on, use send_input to run the search with sudo in the \
             terminal's shell, where the user can enter a password, or ask the user."
        ),
        RemoteSearchError::BadPattern(message) => format!(
            "grep -E on the host refused the pattern, so nothing was searched: {message}. The \
             pattern is a POSIX extended regular expression; escape ( ) [ ] {{ }} . * + ? | ^ $ \
             and \\ with a backslash to match them as they are."
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
        assert_eq!(
            describe("/etc/x", tools::GLOB, RemoteSearchError::NotFound),
            "/etc/x does not exist."
        );
        assert!(
            describe("/root", tools::GREP, RemoteSearchError::PermissionDenied)
                .contains("grep never uses sudo. To go on, use send_input")
        );
        let bad = describe(
            "~",
            tools::GREP,
            RemoteSearchError::BadPattern("grep: Unmatched ( or \\(".into()),
        );
        assert!(bad.contains("refused the pattern, so nothing was searched: grep: Unmatched ("));
        assert!(describe("/x", tools::GLOB, RemoteSearchError::NotDirectory).contains("read_file"));
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
