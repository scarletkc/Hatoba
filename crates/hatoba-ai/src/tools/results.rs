//! Tool result formatting and truncation helpers (spec §13.4).

/// Results longer than this many characters are shortened (§13.4).
pub const MAX_RESULT_CHARS: usize = 30_000;
/// MCP tool results longer than this many characters are shortened (§13.9): servers such as
/// documentation or database tools return long results by design.
pub const MAX_MCP_RESULT_CHARS: usize = 50_000;

/// Shortens a tool result longer than [`MAX_RESULT_CHARS`] (30,000) characters to its first
/// 7,500 and last 22,500 characters, with a line between them saying how many were left out
/// (§13.4). Counts characters, never splitting one.
#[must_use]
pub fn truncate_result(s: &str) -> String {
    truncate_result_to(s, MAX_RESULT_CHARS)
}

/// [`truncate_result`] for a result of at most `max` characters: a quarter of them from its start
/// and the rest from its end.
#[must_use]
pub fn truncate_result_to(s: &str, max: usize) -> String {
    let total = s.chars().count();
    if total <= max {
        return s.to_owned();
    }
    let head = max / 4;
    let tail = max - head;
    let left_out = total - head - tail;
    let head_end = s.char_indices().nth(head).map_or(s.len(), |(i, _)| i);
    let tail_start = s
        .char_indices()
        .nth(total - tail)
        .map_or(s.len(), |(i, _)| i);
    format!(
        "{}\n\n[… {left_out} characters left out …]\n\n{}",
        &s[..head_end],
        &s[tail_start..]
    )
}

/// The `run_command` result text: a first line `exit status: N` (or `exit status: none (…)` when
/// the command ended without one), then stdout and stderr, each only when non-empty. The
/// frontend reads the status from that first line, so keep its wording. Not shortened; pass it
/// through [`truncate_result`].
#[must_use]
pub fn format_exec_result(exit_status: Option<u32>, stdout: &str, stderr: &str) -> String {
    let mut out = match exit_status {
        Some(code) => format!("exit status: {code}\n"),
        None => "exit status: none (killed by a signal, or the channel closed without a status)\n"
            .to_owned(),
    };
    push_streams(&mut out, stdout, stderr);
    out
}

/// Why Hatoba stopped a `run_command` command before it ended on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandStop {
    /// The user stopped the turn (AI-18).
    User,
    /// The call's timeout passed: this many seconds (AI-12).
    Timeout(u64),
}

/// What Hatoba knows of a `run_command` command it stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoppedCommand {
    /// The stop came before the command reached the host, so it did not run.
    NotStarted,
    /// The command and the processes it started ended.
    Ended,
    /// Hatoba could not confirm that the command ended.
    MayRun,
}

/// The cancelled result of a `run_command` call whose command still ran when the turn stopped,
/// and whose end Hatoba could not confirm: the stop did not wait for it, as locking the vault does,
/// or the command did not report in time (AI-18).
pub const COMMAND_STOP_UNCONFIRMED: &str = "The command was still running when the turn stopped. \
     Hatoba told it to end but could not confirm that it did, so it may still be running on the \
     host.";

/// The `run_command` result text of a command Hatoba stopped (AI-12, AI-18): a line that says
/// what became of it, then what it printed until then, as [`format_exec_result`] shows it, without
/// the exit status. Not shortened; pass it through [`truncate_result`].
#[must_use]
pub fn stopped_command_result(
    stop: CommandStop,
    state: StoppedCommand,
    stdout: &str,
    stderr: &str,
) -> String {
    let mut out = match (stop, state) {
        (CommandStop::User, StoppedCommand::NotStarted) => {
            return "The user stopped the call before the command reached the host, so it did \
                    not run."
                .to_owned();
        }
        (CommandStop::Timeout(secs), StoppedCommand::NotStarted) => {
            return format!(
                "The command did not reach the host within {secs} seconds, so Hatoba gave up on \
                 it; it did not run."
            );
        }
        (CommandStop::User, StoppedCommand::Ended) => "The user stopped the command while it \
             ran. Hatoba ended it and the processes it started, so none of them is running on \
             the host now, apart from any it started as a daemon or a service, such as a \
             container.\n"
            .to_owned(),
        (CommandStop::User, StoppedCommand::MayRun) => "The user stopped the command while it \
             ran. Hatoba told it to end but could not confirm that it did, so it may still be \
             running on the host.\n"
            .to_owned(),
        (CommandStop::Timeout(secs), StoppedCommand::Ended) => format!(
            "The command did not finish within {secs} seconds, so Hatoba ended it and the \
             processes it started; none of them is running on the host now, apart from any it \
             started as a daemon or a service, such as a container.\n"
        ),
        (CommandStop::Timeout(secs), StoppedCommand::MayRun) => format!(
            "The command did not finish within {secs} seconds. Hatoba told it to end but could \
             not confirm that it did, so it may still be running on the host.\n"
        ),
    };
    push_streams(&mut out, stdout, stderr);
    out
}

/// Adds stdout and stderr to a `run_command` result, each only when non-empty, or `(no output)`.
fn push_streams(out: &mut String, stdout: &str, stderr: &str) {
    if stdout.is_empty() && stderr.is_empty() {
        out.push_str("(no output)\n");
        return;
    }
    for (name, stream) in [("stdout", stdout), ("stderr", stderr)] {
        if !stream.is_empty() {
            out.push_str(&format!("--- {name} ---\n{stream}"));
            if !stream.ends_with('\n') {
                out.push('\n');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_results_stay_and_long_ones_keep_head_and_tail() {
        let exact = "a".repeat(MAX_RESULT_CHARS);
        assert_eq!(truncate_result(&exact), exact);
        assert_eq!(truncate_result(""), "");

        let long = format!(
            "{}{}{}",
            "h".repeat(7_500),
            "m".repeat(5_000),
            "t".repeat(22_500)
        );
        let out = truncate_result(&long);
        assert!(out.starts_with(&"h".repeat(7_500)));
        assert!(out.ends_with(&"t".repeat(22_500)));
        assert!(out.contains("[… 5000 characters left out …]"));
        assert!(!out.contains('m'));

        // Multi-byte characters are counted as characters and never split.
        let wide = "日".repeat(MAX_RESULT_CHARS + 1);
        let out = truncate_result(&wide);
        assert!(out.contains("[… 1 characters left out …]"));
        assert_eq!(out.chars().filter(|&c| c == '日').count(), MAX_RESULT_CHARS);

        // MCP results keep more, in the same proportions.
        let mcp = "x".repeat(MAX_MCP_RESULT_CHARS);
        assert_eq!(truncate_result_to(&mcp, MAX_MCP_RESULT_CHARS), mcp);
        let out = truncate_result_to(&format!("{mcp}yz"), MAX_MCP_RESULT_CHARS);
        assert!(out.starts_with(&"x".repeat(12_500)) && out.ends_with("xyz"));
        assert!(out.contains("[… 2 characters left out …]"));
    }

    #[test]
    fn exec_results_are_formatted() {
        assert_eq!(
            format_exec_result(Some(0), "a\n", ""),
            "exit status: 0\n--- stdout ---\na\n"
        );
        assert_eq!(
            format_exec_result(Some(2), "out", "err"),
            "exit status: 2\n--- stdout ---\nout\n--- stderr ---\nerr\n"
        );
        assert_eq!(
            format_exec_result(Some(0), "", ""),
            "exit status: 0\n(no output)\n"
        );
        // The frontend's status chip matches this first line; keep the wording.
        assert!(format_exec_result(Some(127), "", "not found").starts_with("exit status: 127\n"));
        assert!(
            format_exec_result(None, "", "").starts_with("exit status: none (killed by a signal")
        );
    }

    /// AI-12, AI-18: a stopped command's result says whether it ended, and what it printed.
    #[test]
    fn stopped_command_results_say_what_became_of_the_command() {
        use CommandStop::{Timeout, User};
        use StoppedCommand::{Ended, MayRun, NotStarted};
        assert_eq!(
            stopped_command_result(User, Ended, "partial\n", ""),
            "The user stopped the command while it ran. Hatoba ended it and the processes it \
             started, so none of them is running on the host now, apart from any it started as \
             a daemon or a service, such as a container.\n--- stdout ---\npartial\n"
        );
        assert_eq!(
            stopped_command_result(User, MayRun, "", "err"),
            "The user stopped the command while it ran. Hatoba told it to end but could not \
             confirm that it did, so it may still be running on the host.\n--- stderr ---\nerr\n"
        );
        assert_eq!(
            stopped_command_result(Timeout(30), Ended, "", ""),
            "The command did not finish within 30 seconds, so Hatoba ended it and the processes \
             it started; none of them is running on the host now, apart from any it started as a \
             daemon or a service, such as a container.\n(no output)\n"
        );
        assert_eq!(
            stopped_command_result(Timeout(120), MayRun, "a", "b"),
            "The command did not finish within 120 seconds. Hatoba told it to end but could not \
             confirm that it did, so it may still be running on the host.\n--- stdout ---\na\n\
             --- stderr ---\nb\n"
        );
        // Nothing ran, so there is no output to show.
        assert_eq!(
            stopped_command_result(User, NotStarted, "", ""),
            "The user stopped the call before the command reached the host, so it did not run."
        );
        assert_eq!(
            stopped_command_result(Timeout(5), NotStarted, "", ""),
            "The command did not reach the host within 5 seconds, so Hatoba gave up on it; it \
             did not run."
        );
        // No first line the frontend would read as an exit status.
        for state in [Ended, MayRun] {
            let text = stopped_command_result(User, state, "exit status: 0\n", "");
            assert!(!text.starts_with("exit status"), "{text}");
        }
    }
}
