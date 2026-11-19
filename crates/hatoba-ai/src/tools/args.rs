//! Argument structs for built-in tools (spec §13.4).

use std::time::Duration;

use serde::Deserialize;

/// `run_command` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct RunCommandArgs {
    /// The command line.
    pub command: String,
    /// The directory the command starts in: absolute, or from the home directory.
    #[serde(default)]
    pub workdir: Option<String>,
    /// Seconds before the command is stopped.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

impl RunCommandArgs {
    /// The directory the command starts in; `None` for the home directory, also when the model
    /// gave an empty one.
    #[must_use]
    pub fn workdir(&self) -> Option<&str> {
        self.workdir.as_deref().filter(|d| !d.is_empty())
    }

    /// The timeout: 120 s by default, clamped to 1..=600 s (AI-12).
    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_seconds.unwrap_or(120).clamp(1, 600))
    }
}

/// `read_file` arguments (AI-38).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ReadFileArgs {
    /// The file's path on the host.
    pub path: String,
    /// The 1-based line to start from; 1 by default.
    #[serde(default)]
    pub offset: Option<u64>,
    /// The most lines to return; as many as fit by default.
    #[serde(default)]
    pub limit: Option<u64>,
}

/// `edit_file` arguments (AI-39).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct EditFileArgs {
    /// The file's path on the host.
    pub path: String,
    /// The exact text to replace.
    pub old_string: String,
    /// The text to put in its place.
    pub new_string: String,
    /// Replace every occurrence; `false` by default.
    #[serde(default)]
    pub replace_all: Option<bool>,
}

/// `write_file` arguments (AI-40).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct WriteFileArgs {
    /// The file's path on the host.
    pub path: String,
    /// The whole content of the file.
    pub content: String,
}

/// `grep` arguments (AI-48).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct GrepArgs {
    /// The regular expression, as the model wrote it.
    pub pattern: String,
    /// The file or directory to search; the home directory by default.
    #[serde(default)]
    pub path: Option<String>,
    /// A glob the files searched must match.
    #[serde(default)]
    pub glob: Option<String>,
    /// What the result lists; [`GrepOutput::FilesWithMatches`] by default.
    #[serde(default)]
    pub output_mode: Option<GrepOutput>,
    /// Match regardless of letter case; `false` by default. Claude Code's `-i` too.
    #[serde(default, alias = "-i")]
    pub ignore_case: Option<bool>,
    /// Lines before and after each matching line in content mode; 0 by default. Claude Code's
    /// `-C` too.
    #[serde(default, alias = "-C")]
    pub context: Option<u64>,
    /// Lines before each matching line, in place of `context`'s. Claude Code's `-B` too.
    #[serde(default, alias = "-B")]
    pub before_context: Option<u64>,
    /// Lines after each matching line, in place of `context`'s. Claude Code's `-A` too.
    #[serde(default, alias = "-A")]
    pub after_context: Option<u64>,
    /// A match may span lines, and `.` matches a line break; `false` by default.
    #[serde(default)]
    pub multiline: Option<bool>,
    /// Content mode numbers its lines; `true` by default. Claude Code's `-n`, which the schema
    /// leaves out.
    #[serde(default, alias = "-n")]
    pub line_numbers: Option<bool>,
    /// The most entries to return, 0 for no limit; 250 by default. Claude Code's `head_limit`
    /// too.
    #[serde(default, alias = "head_limit")]
    pub limit: Option<u64>,
    /// Entries to skip; 0 by default.
    #[serde(default)]
    pub offset: Option<u64>,
}

/// What a `grep` result lists (AI-48).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrepOutput {
    /// The paths of the files with a matching line.
    #[default]
    FilesWithMatches,
    /// The number of matching lines in each file.
    Count,
    /// The matching lines, with context lines around them.
    Content,
}

/// `glob` arguments (AI-49).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct GlobArgs {
    /// The glob.
    pub pattern: String,
    /// The directory to search; the home directory by default.
    #[serde(default)]
    pub path: Option<String>,
}

/// `web_search` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct WebSearchArgs {
    /// The query.
    pub query: String,
}

/// `fetch_url` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct FetchUrlArgs {
    /// The URL.
    pub url: String,
    /// Character offset, 0 by default.
    #[serde(default)]
    pub offset: Option<u64>,
    /// The whole HTML page instead of its main content, false by default.
    #[serde(default)]
    pub full_page: Option<bool>,
}

/// What a `memory` call does (AI-53).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAction {
    /// Creates or replaces the memory of that name.
    Save,
    /// Removes the memory of that name.
    Delete,
}

/// Which memories a `memory` call changes (AI-53).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// Those every conversation shows.
    Global,
    /// Those of the saved host or the computer's local terminal the conversation is on.
    Host,
}

/// `memory` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct MemoryArgs {
    /// Save or delete.
    pub action: MemoryAction,
    /// Global, or the conversation's host.
    pub scope: MemoryScope,
    /// The memory's name.
    pub name: String,
    /// The memory's text, for a save.
    #[serde(default)]
    pub content: Option<String>,
}

/// `read_skill` arguments.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct ReadSkillArgs {
    /// The skill's name.
    pub name: String,
    /// A file inside the skill; `None` reads `SKILL.md`.
    #[serde(default)]
    pub path: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_arguments_read_both_actions_and_scopes() {
        let save: MemoryArgs =
            serde_json::from_str(r#"{"action":"save","scope":"global","name":"n","content":"c"}"#)
                .unwrap();
        assert_eq!(save.action, MemoryAction::Save);
        assert_eq!(save.scope, MemoryScope::Global);
        assert_eq!(save.content.as_deref(), Some("c"));
        let delete: MemoryArgs =
            serde_json::from_str(r#"{"action":"delete","scope":"host","name":"n"}"#).unwrap();
        assert_eq!(delete.action, MemoryAction::Delete);
        assert_eq!(delete.scope, MemoryScope::Host);
        assert_eq!(delete.content, None);
        for wrong in [
            r#"{"action":"forget","scope":"global","name":"n"}"#,
            r#"{"action":"delete","name":"n"}"#,
            r#"{"action":"delete","scope":"device","name":"n"}"#,
        ] {
            assert!(
                serde_json::from_str::<MemoryArgs>(wrong).is_err(),
                "{wrong}"
            );
        }
    }

    #[test]
    fn run_command_timeout_defaults_and_clamps() {
        let args: RunCommandArgs = serde_json::from_str(r#"{"command":"ls"}"#).unwrap();
        assert_eq!(args.timeout(), Duration::from_secs(120));
        assert_eq!(args.workdir(), None);
        for (given, expected) in [(0, 1), (1, 1), (45, 45), (600, 600), (9999, 600)] {
            let args = RunCommandArgs {
                command: "x".into(),
                workdir: None,
                timeout_seconds: Some(given),
            };
            assert_eq!(args.timeout(), Duration::from_secs(expected));
        }
        let args: RunCommandArgs =
            serde_json::from_str(r#"{"command":"ls","workdir":"/srv/app"}"#).unwrap();
        assert_eq!(args.workdir(), Some("/srv/app"));
        // An empty directory is the home directory.
        let args: RunCommandArgs =
            serde_json::from_str(r#"{"command":"ls","workdir":""}"#).unwrap();
        assert_eq!(args.workdir(), None);
        let fetch: FetchUrlArgs = serde_json::from_str(r#"{"url":"https://x"}"#).unwrap();
        assert_eq!(fetch.offset, None);
        let skill: ReadSkillArgs =
            serde_json::from_str(r#"{"name":"a","path":"references/b.md"}"#).unwrap();
        assert_eq!(skill.path.as_deref(), Some("references/b.md"));
        let search: WebSearchArgs = serde_json::from_str(r#"{"query":"q"}"#).unwrap();
        assert_eq!(search.query, "q");
    }
}
