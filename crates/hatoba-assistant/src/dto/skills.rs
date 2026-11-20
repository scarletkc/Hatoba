use std::fmt;

use serde::{Deserialize, Serialize};
use specta::Type;

// ───────────────────────── AI assistant: skills (§13.8) ─────────────────────────
//
// Skill text reaches the model as instructions; it is never logged (SEC-04), so the types that
// carry it print only sizes.

#[derive(Clone, Serialize, Deserialize, Type, PartialEq, Eq)]
pub struct SkillFileView {
    /// Relative path with forward slashes, such as `references/nginx.md`. Never `SKILL.md`.
    pub path: String,
    pub content: String,
}

impl fmt::Debug for SkillFileView {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SkillFileView")
            .field("path", &self.path)
            .field("content", &format_args!("<{} bytes>", self.content.len()))
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct SkillView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    /// Paths of the files besides `SKILL.md`, sorted.
    pub files: Vec<String>,
    pub updated_at: i64,
}

/// A memory of the AI assistant (AI-53): a global one, with neither `host_id` nor `local`, or a
/// host memory of a saved host or of a computer's local terminal.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiMemoryView {
    pub id: String,
    pub name: String,
    pub content: String,
    pub host_id: Option<String>,
    /// The computer's name, while `host_id` is `None`.
    pub local: Option<String>,
    pub updated_at: i64,
}

/// A memory to save from Settings → AI (AI-53), in the scope `host_id` and `local` name, as in
/// [`AiMemoryView`].
#[derive(Debug, Clone, Deserialize, Type)]
pub struct AiMemoryInput {
    /// `None` creates a memory.
    pub id: Option<String>,
    pub name: String,
    pub content: String,
    pub host_id: Option<String>,
    pub local: Option<String>,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct SkillDetail {
    pub skill: SkillView,
    /// `SKILL.md` without its frontmatter.
    pub body: String,
    pub files: Vec<SkillFileView>,
    /// Other frontmatter fields, kept for export. They change nothing (AI-27).
    pub frontmatter_keys: Vec<String>,
}

/// The built-in `hatoba` skill (AI-34), read-only.
#[derive(Debug, Clone, Serialize, Type)]
pub struct BuiltinSkillView {
    pub name: String,
    pub description: String,
    /// `Settings.ai.builtin_skill_enabled`.
    pub enabled: bool,
    /// `SKILL.md` without its frontmatter, with the app's version filled in.
    pub body: String,
    /// The files besides `SKILL.md`, sorted by path.
    pub files: Vec<SkillFileView>,
}

#[derive(Clone, Deserialize, Type)]
pub struct SkillInput {
    /// `None` creates a skill.
    pub id: Option<String>,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub body: String,
    /// Every file besides `SKILL.md`; a saved file left out is deleted.
    pub files: Vec<SkillFileView>,
}

impl fmt::Debug for SkillInput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SkillInput")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("enabled", &self.enabled)
            .field("body", &format_args!("<{} bytes>", self.body.len()))
            .field("files", &self.files)
            .finish()
    }
}

/// Why a skill cannot be imported or saved (AI-27).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SkillIssue {
    MissingSkillMd,
    InvalidFrontmatter {
        detail: String,
    },
    InvalidName {
        name: String,
    },
    MissingDescription,
    DescriptionTooLong {
        chars: u64,
    },
    FileTooLarge {
        path: String,
        size: u64,
    },
    UnsafePath {
        path: String,
    },
    TooManyFiles {
        count: u64,
    },
    /// More than 5 MB in total.
    TooLarge {
        bytes: u64,
    },
}

/// Why a file dropped on the AI panel is not attached (AI-35): the panel's own reasons.
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DroppedFileRefusal {
    /// An image, which attachments do not take yet.
    Image,
    /// Over 256 KB.
    TooLarge,
    /// NUL bytes or invalid UTF-8.
    Binary,
    /// Not a file, gone, or not readable.
    Unreadable,
}

/// A text file dropped on the AI panel (AI-35), named by its base name only, never its path (a
/// path can name the local user).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DroppedFile {
    Ok {
        name: String,
        text: String,
    },
    Refused {
        name: String,
        reason: DroppedFileRefusal,
    },
}

/// One entry of a directory on the tab's host, for the panel's `@` menu (AI-51).
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiHostEntry {
    pub name: String,
    /// A directory, or a symbolic link to one.
    pub dir: bool,
}

/// Why a path on the tab's host cannot be listed or attached (AI-51).
#[derive(Debug, Clone, Copy, Serialize, Type, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiHostRefusal {
    /// The tab is not connected.
    Disconnected,
    NotFound,
    /// Read: a directory. List: not a directory.
    WrongKind,
    /// A device, FIFO, socket or a symbolic link to nothing.
    NotRegular,
    PermissionDenied,
    /// Over 256 KB.
    TooLarge,
    /// NUL bytes or invalid UTF-8.
    Binary,
    /// The command failed or did not finish within 30 seconds.
    Failed,
}

/// A directory on the tab's host (AI-51): its entries, or why it cannot be listed.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AiHostListing {
    Ok {
        /// The path as the host resolved it: absolute, with `~/` and a relative path taken from
        /// the home directory.
        path: String,
        /// Sorted by name.
        entries: Vec<AiHostEntry>,
        /// Entries were left out: the listing passed its size limit.
        cut: bool,
    },
    Refused {
        reason: AiHostRefusal,
    },
}

/// A text file on the tab's host for an `@` mention (AI-51), or why it cannot be attached.
#[derive(Clone, Serialize, Type, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum AiHostFile {
    Ok {
        /// The path as the host resolved it.
        path: String,
        text: String,
    },
    Refused {
        reason: AiHostRefusal,
    },
}

impl fmt::Debug for AiHostFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ok { text, .. } => f
                .debug_struct("Ok")
                .field("text", &format_args!("<{} bytes>", text.len()))
                .finish_non_exhaustive(),
            Self::Refused { reason } => f.debug_struct("Refused").field("reason", reason).finish(),
        }
    }
}

/// An enabled skill the panel's `/` menu offers as a command (AI-50), the built-in one included.
#[derive(Debug, Clone, Serialize, Type, PartialEq, Eq)]
pub struct AiSkillCommand {
    pub name: String,
    pub description: String,
}

/// What an import would save, shown before saving (AI-27). Importable when `issues` is empty.
#[derive(Debug, Clone, Serialize, Type)]
pub struct SkillImportPreview {
    /// `None` when `SKILL.md` is missing or unreadable.
    pub name: Option<String>,
    pub description: Option<String>,
    pub body: Option<String>,
    pub files: Vec<SkillFileView>,
    pub frontmatter_keys: Vec<String>,
    /// Files that are not UTF-8 text, which the import skips.
    pub skipped: Vec<String>,
    pub issues: Vec<SkillIssue>,
    /// A saved skill with the same name, which the user may replace (or rename the new one).
    pub existing_id: Option<String>,
    /// The name is the built-in skill's (AI-34): the skill can be imported only under another.
    pub reserved_name: bool,
}
