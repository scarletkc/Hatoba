//! Skills in the Agent Skills format (spec §13.8): `SKILL.md` parsing and rendering, folder and
//! `.zip` import, `.zip` export, the rules both imports and edits are checked against, and the
//! built-in `hatoba` skill (AI-34).
//!
//! A skill is a folder with a `SKILL.md` (YAML frontmatter with `name` and `description`, then a
//! Markdown body) and optional UTF-8 text files such as `references/*.md`. Hatoba never runs a
//! file from a skill; nothing here executes, extracts to disk or follows a link.
//!
//! Imports come from untrusted folders and archives, so they are bounded: at most
//! [`MAX_FILES`] files and [`MAX_TOTAL_BYTES`] in total (declared and actually read), at most
//! [`MAX_FILE_BYTES`] read per file however large it claims to be, a bounded folder walk, and a
//! bounded archive. Paths with `..`, absolute and drive paths are reported as
//! [`SkillIssue::UnsafePath`], and symbolic links are never followed. An import returns every
//! problem it found as a [`SkillImport`] instead of stopping at the first; only a source that
//! cannot be read at all is a [`SkillError`].
//!
//! Skill text reaches the model as instructions, so nothing here logs a path or a file's
//! content (SEC-04).

mod archive;
mod builtin;
mod folder;
mod frontmatter;
mod import;

#[cfg(test)]
mod tests;

use serde_json::{Map, Value};

use self::import::{normalize_path, trimmed_description_issue};

pub use self::archive::{read_zip, write_zip};
pub use self::builtin::{BUILTIN_NAME, VERSION_PLACEHOLDER, builtin_description, builtin_skill};
pub use self::folder::read_folder;
pub use self::frontmatter::{parse_skill_md, render_skill_md};

/// The size limit of every file, `SKILL.md` included (AI-27). For `SKILL.md` the limit applies
/// to the file as imported, and to the body once it is a [`SkillPackage`].
pub const MAX_FILE_BYTES: usize = 32 * 1024;
/// The longest description, in characters (AI-27).
pub const MAX_DESCRIPTION_CHARS: usize = 1024;
/// Files in one skill, `SKILL.md` included.
pub const MAX_FILES: usize = 200;
/// Bytes in one skill, all files together (the zip bomb guard).
pub const MAX_TOTAL_BYTES: usize = 5 * 1024 * 1024;

/// The name of the instructions file.
const SKILL_MD: &str = "SKILL.md";

/// Whether `name` is a valid skill name: 1 to 64 lowercase letters, digits and hyphens (AI-27).
#[must_use]
pub fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// One text file of a skill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillFileData {
    /// Relative to the skill's folder, with forward slashes, e.g. `references/nginx.md`. Never
    /// `SKILL.md`, whose body is [`SkillPackage::body`].
    pub path: String,
    /// UTF-8 text, at most [`MAX_FILE_BYTES`] bytes, without a leading byte order mark.
    pub content: String,
}

/// A skill as imported, edited, stored and exported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillPackage {
    /// 1 to 64 lowercase letters, digits and hyphens.
    pub name: String,
    /// At most [`MAX_DESCRIPTION_CHARS`] characters, without surrounding whitespace.
    pub description: String,
    /// Every other frontmatter field as JSON (YAML converted), kept for export. It never
    /// changes behavior: `allowed-tools` does not touch approvals.
    pub frontmatter: Map<String, Value>,
    /// `SKILL.md` without its frontmatter, exactly as written (line endings included).
    pub body: String,
    /// The other files, sorted by path when imported.
    pub files: Vec<SkillFileData>,
}

/// Why a file was left out of an import.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// The file is not UTF-8 text (invalid UTF-8, a NUL byte), or it cannot be decompressed
    /// (an unsupported method or encryption in a `.zip`).
    NotText,
}

/// A file an import skipped; the import is still valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkippedFile {
    /// Relative to the skill's folder, with forward slashes.
    pub path: String,
    /// Why it was skipped.
    pub reason: SkipReason,
}

/// Why an import is rejected, or an edited skill is invalid (AI-27).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillIssue {
    /// No `SKILL.md` at the root or inside the single top-level folder.
    MissingSkillMd,
    /// The frontmatter is absent, not closed, not a YAML mapping, or `SKILL.md` is not text. The
    /// message names the problem and, for YAML errors, the line.
    InvalidFrontmatter(String),
    /// The `name` is missing (an empty string), not text, or not 1 to 64 lowercase letters,
    /// digits and hyphens.
    InvalidName(String),
    /// The `description` is missing or empty.
    MissingDescription,
    /// The description has more than [`MAX_DESCRIPTION_CHARS`] characters (the number found).
    DescriptionTooLong(usize),
    /// A text file is larger than [`MAX_FILE_BYTES`]. `size` is the declared size or the bytes
    /// actually read, whichever is larger. A file that is not text is skipped at any size.
    FileTooLarge {
        /// Relative to the skill's folder.
        path: String,
        /// Size in bytes.
        size: u64,
    },
    /// A path that cannot be a skill file path: `..`, absolute, a drive, control characters,
    /// nested or long beyond the limits, a duplicate of another file, or `SKILL.md` as a file
    /// of its own. Carries the path as found.
    UnsafePath(String),
    /// More than [`MAX_FILES`] files (the number found; a folder walk stops shortly after the
    /// limit, so it can be lower than the real number).
    TooManyFiles(usize),
    /// The files together are larger than [`MAX_TOTAL_BYTES`] (the size found: declared sizes
    /// when the source states them, else bytes read).
    TooLarge(u64),
}

/// What reading a folder or a `.zip` produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkillImport {
    /// The skill, or `None` when `SKILL.md` is unusable (missing, too large, not text, or
    /// invalid frontmatter). Otherwise present even when `issues` is not empty, with whatever
    /// could be read, so the caller can show it; a name or description that failed its check is
    /// kept as found. With `TooManyFiles` or `TooLarge`, only `SKILL.md` was read.
    pub package: Option<SkillPackage>,
    /// Files left out because they are not text, sorted by path.
    pub skipped: Vec<SkippedFile>,
    /// Everything that makes the import invalid. It is importable iff this is empty.
    pub issues: Vec<SkillIssue>,
}

impl SkillImport {
    /// Whether the import has no issue, so [`package`](Self::package) can be saved.
    #[must_use]
    pub fn is_importable(&self) -> bool {
        self.issues.is_empty() && self.package.is_some()
    }
}

/// A skill source or target that could not be read or written at all. Content problems are
/// [`SkillIssue`]s instead.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SkillError {
    /// The folder or file could not be read or written, or is not what it should be.
    #[error("I/O error: {0}")]
    Io(String),
    /// The `.zip` is malformed, unreadable, too large, or the skill cannot be exported.
    #[error("zip error: {0}")]
    Zip(String),
}

/// Checks a skill against the import rules, for skills created or edited in the app: the name,
/// the description, the body and every file's size, the file paths (normalized, unique, none
/// named `SKILL.md`), the number of files and the total size. Empty when the skill is valid.
#[must_use]
pub fn validate(pkg: &SkillPackage) -> Vec<SkillIssue> {
    let mut issues = Vec::new();
    if !valid_name(&pkg.name) {
        issues.push(SkillIssue::InvalidName(pkg.name.clone()));
    }
    issues.extend(trimmed_description_issue(&pkg.description));
    if pkg.body.len() > MAX_FILE_BYTES {
        issues.push(SkillIssue::FileTooLarge {
            path: SKILL_MD.to_owned(),
            size: pkg.body.len() as u64,
        });
    }
    let mut seen = std::collections::HashSet::new();
    for file in &pkg.files {
        let usable = file.path != SKILL_MD
            && normalize_path(&file.path).as_deref() == Some(file.path.as_str())
            && seen.insert(file.path.as_str());
        if !usable {
            issues.push(SkillIssue::UnsafePath(file.path.clone()));
        }
        if file.content.len() > MAX_FILE_BYTES {
            issues.push(SkillIssue::FileTooLarge {
                path: file.path.clone(),
                size: file.content.len() as u64,
            });
        }
    }
    if pkg.files.len() + 1 > MAX_FILES {
        issues.push(SkillIssue::TooManyFiles(pkg.files.len() + 1));
    }
    let total = pkg.body.len() as u64
        + pkg
            .files
            .iter()
            .map(|f| f.content.len() as u64)
            .sum::<u64>();
    if total > MAX_TOTAL_BYTES as u64 {
        issues.push(SkillIssue::TooLarge(total));
    }
    issues
}
