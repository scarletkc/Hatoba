//! What folder and `.zip` import share: path rules, collecting the files a source lists, and
//! turning them into a [`SkillImport`] while reading as little as possible.
//!
//! A source first lists its files with a [`Collector`] (checking every path), then calls
//! [`assemble`] with the files relative to the skill's folder and a reader. `assemble` reads
//! `SKILL.md` first, stops there when the file count or total size is over the limits, and reads
//! at most [`MAX_FILE_BYTES`] + 1 bytes of any other file, however large it claims to be.

use std::collections::HashSet;

use serde_json::{Map, Value};

use super::frontmatter::parse_skill_md;
use super::{
    MAX_DESCRIPTION_CHARS, MAX_FILE_BYTES, MAX_FILES, MAX_TOTAL_BYTES, SKILL_MD, SkillError,
    SkillFileData, SkillImport, SkillIssue, SkillPackage, SkipReason, SkippedFile, valid_name,
};

/// Components in a path, the file name included.
pub(super) const MAX_PATH_DEPTH: usize = 10;
/// Bytes in a normalized path.
const MAX_PATH_BYTES: usize = 256;
/// Characters of an offending path kept in an [`SkillIssue::UnsafePath`].
const SHOWN_PATH_CHARS: usize = 120;

/// A file a source lists: its normalized path, its declared size and how to read it.
pub(super) struct Entry<H> {
    pub path: String,
    pub declared: u64,
    pub handle: H,
}

/// What a reader returned for one file.
pub(super) enum Read {
    /// Up to the requested number of bytes.
    Bytes(Vec<u8>),
    /// The file cannot be read as stored (a `.zip` entry with an unsupported compression method
    /// or encryption). Skipped like a file that is not text; for `SKILL.md` it is an error.
    Unreadable,
}

/// Directory names that never hold skill files: macOS archive junk and version control data.
pub(super) fn ignored_dir(name: &str) -> bool {
    name == "__MACOSX" || name == ".git"
}

fn ignored(path: &str) -> bool {
    let mut parts = path.split('/');
    let last = parts.next_back().unwrap_or_default();
    last == ".DS_Store" || last == "Thumbs.db" || parts.any(ignored_dir)
}

/// The path with forward slashes, `.` and empty parts dropped, or `None` when it is not a safe
/// relative path: it has a `..` part, starts at the root, a drive (`C:`) or a share (`\\host`),
/// holds a control character, or is deeper or longer than the limits.
pub(super) fn normalize_path(raw: &str) -> Option<String> {
    if raw.chars().any(char::is_control) {
        return None;
    }
    let unified = raw.replace('\\', "/");
    if unified.starts_with('/') {
        return None;
    }
    let mut parts: Vec<&str> = Vec::new();
    for part in unified.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            part => {
                let bytes = part.as_bytes();
                if parts.is_empty()
                    && bytes.len() >= 2
                    && bytes[0].is_ascii_alphabetic()
                    && bytes[1] == b':'
                {
                    return None;
                }
                parts.push(part);
            }
        }
    }
    if parts.is_empty() || parts.len() > MAX_PATH_DEPTH {
        return None;
    }
    let path = parts.join("/");
    (path.len() <= MAX_PATH_BYTES).then_some(path)
}

/// A path for an issue message: cut, with control characters replaced.
pub(super) fn shown_path(raw: &str) -> String {
    raw.chars()
        .take(SHOWN_PATH_CHARS)
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

/// Collects the files of a source. Directories and symbolic links are never added; files in
/// ignored places are dropped; unsafe or duplicate paths become issues.
pub(super) struct Collector<H> {
    entries: Vec<Entry<H>>,
    issues: Vec<SkillIssue>,
    seen: HashSet<String>,
}

impl<H> Collector<H> {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            issues: Vec::new(),
            seen: HashSet::new(),
        }
    }

    pub fn add_file(&mut self, raw_path: &str, declared: u64, handle: H) {
        let Some(path) = normalize_path(raw_path) else {
            self.add_unsafe(raw_path);
            return;
        };
        if ignored(&path) {
            return;
        }
        if !self.seen.insert(path.clone()) {
            self.add_unsafe(&path);
            return;
        }
        self.entries.push(Entry {
            path,
            declared,
            handle,
        });
    }

    pub fn add_unsafe(&mut self, raw_path: &str) {
        self.issues
            .push(SkillIssue::UnsafePath(shown_path(raw_path)));
    }

    /// Files collected so far.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn finish(self) -> (Vec<Entry<H>>, Vec<SkillIssue>) {
        (self.entries, self.issues)
    }
}

/// The text of a file, or `None` when it is not text: invalid UTF-8 or a NUL byte. With
/// `truncated`, the bytes are only the start of a longer file, so a character cut at the end
/// does not count against it.
fn as_text(bytes: &[u8], truncated: bool) -> Option<&str> {
    if bytes.contains(&0) {
        return None;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => Some(text),
        Err(err) if truncated && err.error_len().is_none() => {
            std::str::from_utf8(&bytes[..err.valid_up_to()]).ok()
        }
        Err(_) => None,
    }
}

/// The issue for a description that is empty or too long, judged without surrounding whitespace.
pub(super) fn trimmed_description_issue(description: &str) -> Option<SkillIssue> {
    let description = description.trim();
    if description.is_empty() {
        return Some(SkillIssue::MissingDescription);
    }
    let chars = description.chars().count();
    (chars > MAX_DESCRIPTION_CHARS).then_some(SkillIssue::DescriptionTooLong(chars))
}

/// Takes `name` and `description` out of the frontmatter fields, with their issues. A name that
/// is missing is an empty one; a description is trimmed.
fn take_name_and_description(fields: &mut Map<String, Value>) -> (String, String, Vec<SkillIssue>) {
    let mut issues = Vec::new();
    let name = match fields.remove("name") {
        Some(Value::String(name)) => name,
        None | Some(Value::Null) => String::new(),
        Some(other) => {
            issues.push(SkillIssue::InvalidName(format!("{other} is not text")));
            String::new()
        }
    };
    if issues.is_empty() && !valid_name(&name) {
        issues.push(SkillIssue::InvalidName(name.clone()));
    }
    let description = match fields.remove("description") {
        Some(Value::String(description)) => {
            let description = description.trim().to_owned();
            issues.extend(trimmed_description_issue(&description));
            description
        }
        None | Some(Value::Null) => {
            issues.push(SkillIssue::MissingDescription);
            String::new()
        }
        Some(_) => {
            issues.push(SkillIssue::InvalidFrontmatter(
                "`description` must be text".to_owned(),
            ));
            String::new()
        }
    };
    (name, description, issues)
}

fn without_import(issues: Vec<SkillIssue>) -> SkillImport {
    SkillImport {
        package: None,
        skipped: Vec::new(),
        issues,
    }
}

/// Builds the import from the files of a skill, with paths relative to its folder.
///
/// `issues` are those found while listing. `stop_at_skill_md` leaves every file but `SKILL.md`
/// unread (the source already knows the skill is over a limit). `read(handle, limit)` returns
/// at most `limit` bytes of a file.
pub(super) fn assemble<H>(
    mut entries: Vec<Entry<H>>,
    mut issues: Vec<SkillIssue>,
    mut stop_at_skill_md: bool,
    mut read: impl FnMut(&H, u64) -> Result<Read, SkillError>,
) -> Result<SkillImport, SkillError> {
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let Some(position) = entries.iter().position(|e| e.path == SKILL_MD) else {
        issues.push(SkillIssue::MissingSkillMd);
        return Ok(without_import(issues));
    };
    let skill_md = entries.remove(position);

    if entries.len() + 1 > MAX_FILES {
        issues.push(SkillIssue::TooManyFiles(entries.len() + 1));
        stop_at_skill_md = true;
    }
    let declared_total = entries
        .iter()
        .fold(skill_md.declared, |sum, e| sum.saturating_add(e.declared));
    if declared_total > MAX_TOTAL_BYTES as u64 {
        issues.push(SkillIssue::TooLarge(declared_total));
        stop_at_skill_md = true;
    }

    // One byte more than the limit tells a file at the limit from one over it.
    let limit = MAX_FILE_BYTES as u64 + 1;
    let bytes = match read(&skill_md.handle, limit)? {
        Read::Bytes(bytes) => bytes,
        Read::Unreadable => {
            return Err(SkillError::Zip(
                "SKILL.md uses an unsupported compression method or is encrypted".to_owned(),
            ));
        }
    };
    if bytes.len() > MAX_FILE_BYTES {
        issues.push(SkillIssue::FileTooLarge {
            path: SKILL_MD.to_owned(),
            size: skill_md.declared.max(bytes.len() as u64),
        });
        return Ok(without_import(issues));
    }
    let Some(text) = as_text(&bytes, false) else {
        issues.push(SkillIssue::InvalidFrontmatter(
            "SKILL.md is not UTF-8 text".to_owned(),
        ));
        return Ok(without_import(issues));
    };
    let (mut fields, body) = match parse_skill_md(text) {
        Ok(parsed) => parsed,
        Err(issue) => {
            issues.push(issue);
            return Ok(without_import(issues));
        }
    };
    let (name, description, skill_md_issues) = take_name_and_description(&mut fields);
    issues.extend(skill_md_issues);

    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let mut read_total = bytes.len() as u64;
    if !stop_at_skill_md {
        for entry in &entries {
            let bytes = match read(&entry.handle, limit)? {
                Read::Bytes(bytes) => bytes,
                Read::Unreadable => {
                    skipped.push(not_text(&entry.path));
                    continue;
                }
            };
            read_total = read_total.saturating_add(bytes.len() as u64);
            let too_large = bytes.len() > MAX_FILE_BYTES;
            match as_text(&bytes, too_large) {
                None => skipped.push(not_text(&entry.path)),
                Some(_) if too_large => issues.push(SkillIssue::FileTooLarge {
                    path: entry.path.clone(),
                    size: entry.declared.max(bytes.len() as u64),
                }),
                Some(text) => files.push(SkillFileData {
                    path: entry.path.clone(),
                    content: text.strip_prefix('\u{feff}').unwrap_or(text).to_owned(),
                }),
            }
            if read_total > MAX_TOTAL_BYTES as u64 {
                issues.push(SkillIssue::TooLarge(read_total));
                break;
            }
        }
    }

    Ok(SkillImport {
        package: Some(SkillPackage {
            name,
            description,
            frontmatter: fields,
            body,
            files,
        }),
        skipped,
        issues,
    })
}

fn not_text(path: &str) -> SkippedFile {
    SkippedFile {
        path: path.to_owned(),
        reason: SkipReason::NotText,
    }
}
